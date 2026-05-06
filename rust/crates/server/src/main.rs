use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};

use axum::{
    Router,
    body::Body,
    extract::{
        State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::{StatusCode, Uri, header},
    response::{IntoResponse, Response},
    routing::get,
};
use common::{ClientMessage, PlayerInfo, ServerMessage};
use futures_util::{SinkExt, StreamExt};
use rand::Rng;
use rust_embed::RustEmbed;
use tokio::{
    sync::{Mutex, mpsc},
    time::sleep,
};
use tracing::{info, warn};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
use uuid::Uuid;

const RECONNECT_TIMEOUT: Duration = Duration::from_secs(5 * 60);
const CLEANUP_TICK: Duration = Duration::from_secs(15);

#[derive(RustEmbed)]
#[folder = "public/"]
struct Assets;

type SharedState = Arc<Mutex<AppState>>;

#[derive(Default)]
struct AppState {
    lobbies: HashMap<String, Lobby>,
    player_sessions: HashMap<String, PlayerSessionRef>,
    host_sessions: HashMap<String, String>,
}

struct PlayerSessionRef {
    code: String,
    player_id: String,
}

struct Lobby {
    host_session: String,
    host_tx: Option<mpsc::UnboundedSender<Message>>,
    host_disconnected_at: Option<Instant>,
    players: HashMap<String, Player>,
    player_order: Vec<String>,
    current_turn: Option<usize>,
    game_in_progress: bool,
    ball_in_play: bool,
}

struct Player {
    username: String,
    session: String,
    tx: Option<mpsc::UnboundedSender<Message>>,
    disconnected_at: Option<Instant>,
}

#[derive(Clone)]
enum ConnectionRole {
    Host { code: String, session: String },
    Player { code: String, session: String },
}

#[tokio::main]
async fn main() {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                if cfg!(debug_assertions) {
                    "server=debug,tower_http=debug,axum=debug".into()
                } else {
                    "server=info,tower_http=info,axum=info".into()
                }
            }),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    info!("starting server on http://0.0.0.0:3000");

    let app_state: SharedState = Arc::new(Mutex::new(AppState::default()));
    tokio::spawn(cleanup_task(app_state.clone()));

    let app = Router::new()
        .route("/ws", get(ws_handler))
        .fallback(static_handler)
        .with_state(app_state);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
    axum::serve(listener, app).await.unwrap();
}

async fn cleanup_task(state: SharedState) {
    loop {
        sleep(CLEANUP_TICK).await;
        cleanup_timeouts(&state).await;
    }
}

async fn cleanup_timeouts(state: &SharedState) {
    let now = Instant::now();

    let (close_lobbies, disconnect_players) = {
        let s = state.lock().await;
        let mut close_lobbies = Vec::new();
        let mut disconnect_players = Vec::new();

        for (code, lobby) in &s.lobbies {
            if let Some(at) = lobby.host_disconnected_at
                && now.duration_since(at) > RECONNECT_TIMEOUT
            {
                close_lobbies.push(code.clone());
                continue;
            }

            for (player_id, player) in &lobby.players {
                if let Some(at) = player.disconnected_at
                    && now.duration_since(at) > RECONNECT_TIMEOUT
                {
                    disconnect_players.push((
                        code.clone(),
                        player_id.clone(),
                        player.session.clone(),
                    ));
                }
            }
        }

        (close_lobbies, disconnect_players)
    };

    for code in close_lobbies {
        close_lobby(state, &code, "lobby closed: host timeout").await;
    }

    for (code, player_id, session) in disconnect_players {
        remove_player_session(state, &code, &player_id, &session).await;
    }
}

async fn static_handler(uri: Uri) -> impl IntoResponse {
    let path = uri.path().trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };

    match Assets::get(path) {
        Some(content) => {
            let mime = mime_guess::from_path(path).first_or_octet_stream();
            Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, mime.as_ref())
                .body(Body::from(content.data))
                .unwrap()
        }
        None => match Assets::get("index.html") {
            Some(content) => Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, "text/html")
                .body(Body::from(content.data))
                .unwrap(),
            None => Response::builder()
                .status(StatusCode::NOT_FOUND)
                .body(Body::from("404"))
                .unwrap(),
        },
    }
}

async fn ws_handler(ws: WebSocketUpgrade, State(state): State<SharedState>) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, state))
}

async fn handle_socket(socket: WebSocket, state: SharedState) {
    let (mut ws_sender, mut ws_receiver) = socket.split();
    let (tx, mut rx) = mpsc::unbounded_channel::<Message>();
    tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            if ws_sender.send(msg).await.is_err() {
                break;
            }
        }
    });

    let mut role: Option<ConnectionRole> = None;

    while let Some(Ok(msg)) = ws_receiver.next().await {
        let Message::Text(text) = msg else { continue };
        let req = match serde_json::from_str::<ClientMessage>(&text) {
            Ok(v) => v,
            Err(err) => {
                let _ = send_to_tx(
                    &tx,
                    &ServerMessage::Error {
                        message: format!("invalid request: {err}"),
                    },
                );
                continue;
            }
        };

        match req {
            ClientMessage::CreateLobby => {
                let (code, host_session, players) = {
                    let mut s = state.lock().await;
                    let code = new_lobby_code(&s);
                    let host_session = Uuid::new_v4().to_string();

                    let lobby = Lobby {
                        host_session: host_session.clone(),
                        host_tx: Some(tx.clone()),
                        host_disconnected_at: None,
                        players: HashMap::new(),
                        player_order: Vec::new(),
                        current_turn: None,
                        game_in_progress: false,
                        ball_in_play: false,
                    };

                    s.host_sessions.insert(host_session.clone(), code.clone());
                    s.lobbies.insert(code.clone(), lobby);
                    (code, host_session, Vec::new())
                };

                role = Some(ConnectionRole::Host {
                    code: code.clone(),
                    session: host_session.clone(),
                });
                let _ = send_to_tx(
                    &tx,
                    &ServerMessage::LobbyCreated {
                        code,
                        host_session,
                        players,
                    },
                );
            }
            ClientMessage::JoinLobby { code, username } => {
                let maybe = {
                    let mut s = state.lock().await;
                    if let Some(lobby) = s.lobbies.get_mut(&code) {
                        let player_id = Uuid::new_v4().to_string();
                        let player_session = Uuid::new_v4().to_string();

                        lobby.players.insert(
                            player_id.clone(),
                            Player {
                                username,
                                session: player_session.clone(),
                                tx: Some(tx.clone()),
                                disconnected_at: None,
                            },
                        );
                        lobby.player_order.push(player_id.clone());

                        let players = lobby_players(lobby);
                        let _ = lobby;
                        s.player_sessions.insert(
                            player_session.clone(),
                            PlayerSessionRef {
                                code: code.clone(),
                                player_id: player_id.clone(),
                            },
                        );
                        Some((player_id, player_session, players))
                    } else {
                        None
                    }
                };

                if let Some((player_id, player_session, players)) = maybe {
                    role = Some(ConnectionRole::Player {
                        code: code.clone(),
                        session: player_session.clone(),
                    });
                    let _ = send_to_tx(
                        &tx,
                        &ServerMessage::LobbyJoined {
                            code: code.clone(),
                            player_id,
                            player_session,
                            players: players.clone(),
                        },
                    );
                    broadcast_lobby(
                        &state,
                        &code,
                        &ServerMessage::LobbyUpdated {
                            code: code.clone(),
                            players,
                        },
                    )
                    .await;
                } else {
                    let _ = send_to_tx(
                        &tx,
                        &ServerMessage::Error {
                            message: "lobby not found".into(),
                        },
                    );
                }
            }
            ClientMessage::ReconnectHost { code, host_session } => {
                if reconnect_host(&state, &tx, &code, &host_session).await {
                    role = Some(ConnectionRole::Host {
                        code,
                        session: host_session,
                    });
                } else {
                    let _ = send_to_tx(
                        &tx,
                        &ServerMessage::Error {
                            message: "host reconnect failed".into(),
                        },
                    );
                }
            }
            ClientMessage::ReconnectPlayer {
                code,
                player_session,
            } => {
                if reconnect_player(&state, &tx, &code, &player_session).await {
                    role = Some(ConnectionRole::Player {
                        code,
                        session: player_session,
                    });
                } else {
                    let _ = send_to_tx(
                        &tx,
                        &ServerMessage::Error {
                            message: "player reconnect failed".into(),
                        },
                    );
                }
            }
            ClientMessage::Leave => {
                if let Some(current) = role.clone() {
                    leave_connection(&state, &current).await;
                    role = None;
                }
            }
            ClientMessage::StartGame => {
                if let Some(ConnectionRole::Host { code, .. }) = role.clone() {
                    if let Some(current_player_id) = start_game(&state, &code).await {
                        broadcast_lobby(
                            &state,
                            &code,
                            &ServerMessage::GameStarted {
                                code: code.clone(),
                                current_player_id,
                            },
                        )
                        .await;
                    } else {
                        let _ = send_to_tx(
                            &tx,
                            &ServerMessage::Error {
                                message: "need at least one connected player to start".into(),
                            },
                        );
                    }
                } else {
                    let _ = send_to_tx(
                        &tx,
                        &ServerMessage::Error {
                            message: "only host can start game".into(),
                        },
                    );
                }
            }
            ClientMessage::ThrowEvent { strength } => {
                if let Some(ConnectionRole::Player { code, session }) = role.clone() {
                    match relay_throw_event(&state, &code, &session, strength).await {
                        Ok(player_id) => {
                            broadcast_lobby(
                                &state,
                                &code,
                                &ServerMessage::ThrowEvent {
                                    player_id,
                                    strength,
                                },
                            )
                            .await;
                        }
                        Err(message) => {
                            let _ = send_to_tx(&tx, &ServerMessage::Error { message });
                        }
                    }
                } else {
                    let _ = send_to_tx(
                        &tx,
                        &ServerMessage::Error {
                            message: "only players can throw".into(),
                        },
                    );
                }
            }
            ClientMessage::AdvanceTurn => {
                if let Some(ConnectionRole::Host { code, .. }) = role.clone() {
                    if let Some(current_player_id) = advance_turn(&state, &code).await {
                        broadcast_lobby(
                            &state,
                            &code,
                            &ServerMessage::TurnChanged { current_player_id },
                        )
                        .await;
                    } else {
                        let _ = send_to_tx(
                            &tx,
                            &ServerMessage::Error {
                                message: "no connected players available for next turn".into(),
                            },
                        );
                    }
                } else {
                    let _ = send_to_tx(
                        &tx,
                        &ServerMessage::Error {
                            message: "only host can advance turn".into(),
                        },
                    );
                }
            }
        }
    }

    if let Some(current) = role {
        disconnect_connection(&state, &current).await;
    }
}

async fn reconnect_host(
    state: &SharedState,
    tx: &mpsc::UnboundedSender<Message>,
    code: &str,
    session: &str,
) -> bool {
    let (players, current_player_id) = {
        let mut s = state.lock().await;
        let Some(mapped_code) = s.host_sessions.get(session) else {
            return false;
        };
        if mapped_code != code {
            return false;
        }
        let Some(lobby) = s.lobbies.get_mut(code) else {
            return false;
        };
        if lobby.host_session != session {
            return false;
        }

        lobby.host_tx = Some(tx.clone());
        lobby.host_disconnected_at = None;
        let current_player_id = lobby
            .current_turn
            .and_then(|idx| lobby.player_order.get(idx))
            .cloned();
        (lobby_players(lobby), current_player_id)
    };

    let _ = send_to_tx(
        tx,
        &ServerMessage::ReconnectOkHost {
            code: code.to_string(),
            players: players.clone(),
        },
    );
    if let Some(current_player_id) = current_player_id {
        let _ = send_to_tx(tx, &ServerMessage::TurnChanged { current_player_id });
    }
    broadcast_lobby(
        state,
        code,
        &ServerMessage::LobbyUpdated {
            code: code.to_string(),
            players,
        },
    )
    .await;
    true
}

async fn reconnect_player(
    state: &SharedState,
    tx: &mpsc::UnboundedSender<Message>,
    code: &str,
    session: &str,
) -> bool {
    let (player_id, players, current_player_id) = {
        let mut s = state.lock().await;
        let Some(sref) = s.player_sessions.get(session) else {
            return false;
        };
        if sref.code != code {
            return false;
        }
        let player_id = sref.player_id.clone();
        let Some(lobby) = s.lobbies.get_mut(code) else {
            return false;
        };
        let Some(player) = lobby.players.get_mut(&player_id) else {
            return false;
        };

        player.tx = Some(tx.clone());
        player.disconnected_at = None;
        let current_player_id = lobby
            .current_turn
            .and_then(|idx| lobby.player_order.get(idx))
            .cloned();
        (player_id, lobby_players(lobby), current_player_id)
    };

    let _ = send_to_tx(
        tx,
        &ServerMessage::ReconnectOkPlayer {
            code: code.to_string(),
            player_id,
            player_session: session.to_string(),
            players: players.clone(),
        },
    );
    if let Some(current_player_id) = current_player_id {
        let _ = send_to_tx(tx, &ServerMessage::TurnChanged { current_player_id });
    }
    broadcast_lobby(
        state,
        code,
        &ServerMessage::LobbyUpdated {
            code: code.to_string(),
            players,
        },
    )
    .await;
    true
}

async fn leave_connection(state: &SharedState, role: &ConnectionRole) {
    match role {
        ConnectionRole::Host { code, .. } => {
            close_lobby(state, code, "lobby closed: host left").await;
        }
        ConnectionRole::Player { code, session } => {
            let player_id = {
                let s = state.lock().await;
                s.player_sessions.get(session).map(|r| r.player_id.clone())
            };
            if let Some(player_id) = player_id {
                remove_player_session(state, code, &player_id, session).await;
            }
        }
    }
}

async fn disconnect_connection(state: &SharedState, role: &ConnectionRole) {
    let now = Instant::now();
    match role {
        ConnectionRole::Host { code, session } => {
            let mut s = state.lock().await;
            let Some(mapped_code) = s.host_sessions.get(session) else {
                return;
            };
            if mapped_code != code {
                return;
            }
            if let Some(lobby) = s.lobbies.get_mut(code) {
                lobby.host_tx = None;
                lobby.host_disconnected_at = Some(now);
            }
        }
        ConnectionRole::Player { code, session } => {
            let mut s = state.lock().await;
            let Some(sref) = s.player_sessions.get(session) else {
                return;
            };
            if sref.code != *code {
                return;
            }
            let player_id = sref.player_id.clone();
            if let Some(lobby) = s.lobbies.get_mut(code)
                && let Some(player) = lobby.players.get_mut(&player_id)
            {
                player.tx = None;
                player.disconnected_at = Some(now);
            }
            let players = s.lobbies.get(code).map(lobby_players);
            drop(s);
            if let Some(players) = players {
                broadcast_lobby(
                    state,
                    code,
                    &ServerMessage::LobbyUpdated {
                        code: code.clone(),
                        players,
                    },
                )
                .await;
                advance_turn_after_player_change(state, code).await;
            }
        }
    }
}

async fn close_lobby(state: &SharedState, code: &str, reason: &str) {
    let (host_tx, player_txs, sessions) = {
        let mut s = state.lock().await;
        let Some(lobby) = s.lobbies.remove(code) else {
            return;
        };

        s.host_sessions.remove(&lobby.host_session);

        let mut sessions = Vec::new();
        let mut player_txs = Vec::new();
        for (_, player) in lobby.players {
            sessions.push(player.session.clone());
            if let Some(tx) = player.tx {
                player_txs.push(tx);
            }
        }
        for session in &sessions {
            s.player_sessions.remove(session);
        }
        (lobby.host_tx, player_txs, sessions)
    };

    let _ = sessions;
    let err = ServerMessage::Error {
        message: reason.to_string(),
    };
    if let Some(tx) = host_tx {
        let _ = send_to_tx(&tx, &err);
    }
    for tx in player_txs {
        let _ = send_to_tx(&tx, &err);
    }
}

async fn start_game(state: &SharedState, code: &str) -> Option<String> {
    let mut s = state.lock().await;
    let lobby = s.lobbies.get_mut(code)?;
    let turn_index = next_connected_turn_index(lobby, None)?;
    lobby.game_in_progress = true;
    lobby.ball_in_play = false;
    lobby.current_turn = Some(turn_index);
    lobby.player_order.get(turn_index).cloned()
}

async fn relay_throw_event(
    state: &SharedState,
    code: &str,
    session: &str,
    strength: f32,
) -> Result<String, String> {
    let mut s = state.lock().await;
    let sref = s
        .player_sessions
        .get(session)
        .ok_or_else(|| "player session not found".to_string())?;
    if sref.code != code {
        return Err("player is not in this lobby".into());
    }
    let player_id = sref.player_id.clone();
    let lobby = s
        .lobbies
        .get_mut(code)
        .ok_or_else(|| "lobby not found".to_string())?;
    if !lobby.game_in_progress {
        return Err("game has not started".into());
    }
    if lobby.ball_in_play {
        return Err("throw already in progress".into());
    }
    let Some(current_turn) = lobby.current_turn else {
        return Err("no active player turn".into());
    };
    let Some(active_player_id) = lobby.player_order.get(current_turn) else {
        return Err("active player not found".into());
    };
    if active_player_id != &player_id {
        return Err("not your turn".into());
    }
    if !(0.0..=1.0).contains(&strength) {
        return Err("throw strength must be between 0 and 1".into());
    }
    lobby.ball_in_play = true;
    Ok(player_id)
}

async fn advance_turn(state: &SharedState, code: &str) -> Option<String> {
    let mut s = state.lock().await;
    let lobby = s.lobbies.get_mut(code)?;
    if !lobby.game_in_progress {
        return None;
    }
    lobby.ball_in_play = false;
    // TODO: replace this temporary round-robin turn system with real bowling frame logic.
    let turn_index = next_connected_turn_index(lobby, lobby.current_turn)?;
    lobby.current_turn = Some(turn_index);
    lobby.player_order.get(turn_index).cloned()
}

async fn remove_player_session(state: &SharedState, code: &str, player_id: &str, session: &str) {
    let (player_tx, players_after) = {
        let mut s = state.lock().await;
        let (player_tx, players_after) = {
            let Some(lobby) = s.lobbies.get_mut(code) else {
                return;
            };
            let player = lobby.players.remove(player_id);
            lobby.player_order.retain(|id| id != player_id);
            let players_after = lobby_players(lobby);
            (player.and_then(|p| p.tx), players_after)
        };
        s.player_sessions.remove(session);
        (player_tx, players_after)
    };

    if let Some(tx) = player_tx {
        let _ = send_to_tx(
            &tx,
            &ServerMessage::Error {
                message: "disconnected: reconnect timeout".into(),
            },
        );
    }

    broadcast_lobby(
        state,
        code,
        &ServerMessage::LobbyUpdated {
            code: code.to_string(),
            players: players_after,
        },
    )
    .await;
    advance_turn_after_player_change(state, code).await;
}

async fn broadcast_lobby(state: &SharedState, code: &str, msg: &ServerMessage) {
    let encoded = match msg.encode() {
        Ok(v) => v,
        Err(err) => {
            warn!("failed to encode message: {err}");
            return;
        }
    };

    let txs = {
        let s = state.lock().await;
        let Some(lobby) = s.lobbies.get(code) else {
            return;
        };
        let mut txs = lobby
            .players
            .values()
            .filter_map(|p| p.tx.clone())
            .collect::<Vec<_>>();
        if let Some(host_tx) = lobby.host_tx.clone() {
            txs.push(host_tx);
        }
        txs
    };

    for tx in txs {
        let _ = tx.send(Message::Text(encoded.clone().into()));
    }
}

fn send_to_tx(tx: &mpsc::UnboundedSender<Message>, msg: &ServerMessage) -> Result<(), ()> {
    let encoded = msg.encode().map_err(|_| ())?;
    tx.send(Message::Text(encoded.into())).map_err(|_| ())
}

fn lobby_players(lobby: &Lobby) -> Vec<PlayerInfo> {
    lobby
        .player_order
        .iter()
        .filter_map(|player_id| {
            lobby.players.get(player_id).map(|p| PlayerInfo {
                player_id: player_id.clone(),
                username: p.username.clone(),
                connected: p.tx.is_some(),
            })
        })
        .collect()
}

fn next_connected_turn_index(lobby: &Lobby, current_turn: Option<usize>) -> Option<usize> {
    if lobby.player_order.is_empty() {
        return None;
    }

    let start = current_turn.map_or(0, |idx| idx.saturating_add(1));
    for offset in 0..lobby.player_order.len() {
        let idx = (start + offset) % lobby.player_order.len();
        let Some(player_id) = lobby.player_order.get(idx) else {
            continue;
        };
        let Some(player) = lobby.players.get(player_id) else {
            continue;
        };
        if player.tx.is_some() {
            return Some(idx);
        }
    }

    None
}

async fn advance_turn_after_player_change(state: &SharedState, code: &str) {
    let current_player_id = {
        let mut s = state.lock().await;
        let Some(lobby) = s.lobbies.get_mut(code) else {
            return;
        };
        if !lobby.game_in_progress || lobby.ball_in_play {
            return;
        }

        let active_connected = lobby
            .current_turn
            .and_then(|idx| lobby.player_order.get(idx))
            .and_then(|player_id| lobby.players.get(player_id))
            .is_some_and(|player| player.tx.is_some());
        if active_connected {
            return;
        }

        let Some(next_turn) = next_connected_turn_index(lobby, lobby.current_turn) else {
            lobby.current_turn = None;
            return;
        };
        lobby.current_turn = Some(next_turn);
        lobby.player_order.get(next_turn).cloned()
    };

    if let Some(current_player_id) = current_player_id {
        broadcast_lobby(
            state,
            code,
            &ServerMessage::TurnChanged { current_player_id },
        )
        .await;
    }
}

fn new_lobby_code(state: &AppState) -> String {
    let mut rng = rand::rng();
    loop {
        let code = format!("{:05}", rng.random_range(0..100000));
        if !state.lobbies.contains_key(&code) {
            return code;
        }
    }
}
