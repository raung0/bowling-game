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
use common::{
    ClientMessage, FrameScoreView, PlayerInfo, PlayerScoreView, ScoreboardState, ServerMessage,
};
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
    bowling: BowlingState,
}

struct Player {
    username: String,
    session: String,
    tx: Option<mpsc::UnboundedSender<Message>>,
    disconnected_at: Option<Instant>,
}

#[derive(Default)]
struct BowlingState {
    players: HashMap<String, PlayerBowlingState>,
    pins_remaining: u8,
    current_roll: u8,
    game_over: bool,
}

#[derive(Default, Clone)]
struct PlayerBowlingState {
    frames: Vec<FrameRecord>,
}

#[derive(Default, Clone)]
struct FrameRecord {
    rolls: Vec<u8>,
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
                        bowling: BowlingState::default(),
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
                    if let Some((current_player_id, scoreboard)) = start_game(&state, &code).await {
                        broadcast_lobby(
                            &state,
                            &code,
                            &ServerMessage::GameStarted {
                                code: code.clone(),
                                current_player_id,
                            },
                        )
                        .await;
                        broadcast_lobby(
                            &state,
                            &code,
                            &ServerMessage::ScoreboardUpdated { scoreboard },
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
            ClientMessage::ThrowEvent {
                force,
                direction_x,
                direction_z,
            } => {
                if let Some(ConnectionRole::Player { code, session }) = role.clone() {
                    match relay_throw_event(
                        &state,
                        &code,
                        &session,
                        force,
                        direction_x,
                        direction_z,
                    )
                    .await
                    {
                        Ok(player_id) => {
                            broadcast_lobby(
                                &state,
                                &code,
                                &ServerMessage::ThrowEvent {
                                    player_id,
                                    force,
                                    direction_x,
                                    direction_z,
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
            ClientMessage::ReportThrowResult {
                knocked_pins,
                standing_pins,
            } => {
                if let Some(ConnectionRole::Host { code, .. }) = role.clone() {
                    match apply_throw_result(&state, &code, knocked_pins, standing_pins).await {
                        Ok(scoreboard) => {
                            broadcast_lobby(
                                &state,
                                &code,
                                &ServerMessage::ScoreboardUpdated { scoreboard },
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
                            message: "only host can report throw results".into(),
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
    let (players, scoreboard) = {
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
        let scoreboard = if lobby.game_in_progress {
            Some(build_scoreboard(lobby))
        } else {
            None
        };
        (lobby_players(lobby), scoreboard)
    };

    let _ = send_to_tx(
        tx,
        &ServerMessage::ReconnectOkHost {
            code: code.to_string(),
            players: players.clone(),
        },
    );
    if let Some(scoreboard) = scoreboard {
        let _ = send_to_tx(tx, &ServerMessage::ScoreboardUpdated { scoreboard });
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
    let (player_id, players, scoreboard) = {
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
        let scoreboard = if lobby.game_in_progress {
            Some(build_scoreboard(lobby))
        } else {
            None
        };
        (player_id, lobby_players(lobby), scoreboard)
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
    if let Some(scoreboard) = scoreboard {
        let _ = send_to_tx(tx, &ServerMessage::ScoreboardUpdated { scoreboard });
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

async fn start_game(state: &SharedState, code: &str) -> Option<(String, ScoreboardState)> {
    let mut s = state.lock().await;
    let lobby = s.lobbies.get_mut(code)?;
    let turn_index = next_connected_turn_index(lobby, None)?;
    lobby.game_in_progress = true;
    lobby.ball_in_play = false;
    lobby.current_turn = Some(turn_index);
    lobby.bowling = BowlingState::default();
    lobby.bowling.pins_remaining = 10;
    lobby.bowling.current_roll = 1;
    for player_id in &lobby.player_order {
        lobby
            .bowling
            .players
            .insert(player_id.clone(), PlayerBowlingState::default());
    }
    let current_player_id = lobby.player_order.get(turn_index)?.clone();
    let scoreboard = build_scoreboard(lobby);
    Some((current_player_id, scoreboard))
}

async fn relay_throw_event(
    state: &SharedState,
    code: &str,
    session: &str,
    force: f32,
    direction_x: f32,
    direction_z: f32,
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
    if !(0.0..=1.0).contains(&force) {
        return Err("throw force must be between 0 and 1".into());
    }
    if !direction_x.is_finite() || !direction_z.is_finite() {
        return Err("throw direction must be finite".into());
    }
    let direction_len = (direction_x * direction_x + direction_z * direction_z).sqrt();
    if !(0.5..=1.5).contains(&direction_len) {
        return Err("throw direction must be normalized".into());
    }
    lobby.ball_in_play = true;
    Ok(player_id)
}

async fn apply_throw_result(
    state: &SharedState,
    code: &str,
    knocked_pins: u8,
    standing_pins: u8,
) -> Result<ScoreboardState, String> {
    let mut s = state.lock().await;
    let lobby = s
        .lobbies
        .get_mut(code)
        .ok_or_else(|| "lobby not found".to_string())?;
    if !lobby.game_in_progress {
        return Err("game has not started".into());
    }
    if !lobby.ball_in_play {
        return Err("no throw in progress".into());
    }
    if lobby.bowling.game_over {
        return Err("game is already over".into());
    }

    let current_turn = lobby
        .current_turn
        .ok_or_else(|| "no active player turn".to_string())?;
    let current_player_id = lobby
        .player_order
        .get(current_turn)
        .cloned()
        .ok_or_else(|| "active player not found".to_string())?;

    if knocked_pins > lobby.bowling.pins_remaining {
        return Err("knocked pins exceed remaining pins".into());
    }
    if standing_pins + knocked_pins != lobby.bowling.pins_remaining {
        return Err("standing pins do not match throw result".into());
    }

    let player_state = lobby
        .bowling
        .players
        .entry(current_player_id)
        .or_insert_with(PlayerBowlingState::default);
    let frame_index = active_frame_index(player_state);
    if frame_index >= 10 {
        return Err("player has already finished the game".into());
    }
    if player_state.frames.len() == frame_index {
        player_state.frames.push(FrameRecord::default());
    }
    let frame = &mut player_state.frames[frame_index];
    frame.rolls.push(knocked_pins);

    lobby.ball_in_play = false;
    if let Some((next_roll, next_pins_remaining)) = continuing_turn_state(frame_index, &frame.rolls)
    {
        lobby.bowling.current_roll = next_roll;
        lobby.bowling.pins_remaining = next_pins_remaining;
    } else if let Some(next_turn) = next_connected_turn_index(lobby, lobby.current_turn) {
        lobby.current_turn = Some(next_turn);
        lobby.bowling.current_roll = 1;
        lobby.bowling.pins_remaining = 10;
    } else {
        lobby.current_turn = None;
        lobby.bowling.current_roll = 0;
        lobby.bowling.pins_remaining = 0;
        lobby.bowling.game_over = all_players_finished(lobby);
    }

    if all_players_finished(lobby) {
        lobby.current_turn = None;
        lobby.bowling.current_roll = 0;
        lobby.bowling.pins_remaining = 0;
        lobby.bowling.game_over = true;
    }

    Ok(build_scoreboard(lobby))
}

async fn remove_player_session(state: &SharedState, code: &str, player_id: &str, session: &str) {
    let (player_tx, players_after) = {
        let mut s = state.lock().await;
        let (player_tx, players_after) = {
            let Some(lobby) = s.lobbies.get_mut(code) else {
                return;
            };
            let removed_index = lobby.player_order.iter().position(|id| id == player_id);
            let player = lobby.players.remove(player_id);
            lobby.player_order.retain(|id| id != player_id);
            lobby.bowling.players.remove(player_id);
            if let (Some(current_turn), Some(removed_index)) = (lobby.current_turn, removed_index) {
                lobby.current_turn = if lobby.player_order.is_empty() {
                    None
                } else if removed_index < current_turn {
                    Some(current_turn - 1)
                } else if current_turn >= lobby.player_order.len() {
                    Some(0)
                } else {
                    Some(current_turn)
                };
            }
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

fn build_scoreboard(lobby: &Lobby) -> ScoreboardState {
    let current_player_id = lobby
        .current_turn
        .and_then(|idx| lobby.player_order.get(idx))
        .cloned()
        .unwrap_or_default();
    let rotation_start = lobby.current_turn.unwrap_or(0);
    let rotated_ids = rotate_player_order(&lobby.player_order, rotation_start);
    let players = rotated_ids
        .into_iter()
        .filter_map(|player_id| build_player_score_view(lobby, &player_id, &current_player_id))
        .collect();

    let current_frame = if current_player_id.is_empty() {
        0
    } else {
        lobby
            .bowling
            .players
            .get(&current_player_id)
            .map(player_frame_number)
            .unwrap_or(1)
    };

    ScoreboardState {
        current_player_id,
        current_frame,
        current_roll: lobby.bowling.current_roll,
        pins_remaining: lobby.bowling.pins_remaining,
        players,
        game_over: lobby.bowling.game_over,
    }
}

fn build_player_score_view(
    lobby: &Lobby,
    player_id: &str,
    current_player_id: &str,
) -> Option<PlayerScoreView> {
    let player = lobby.players.get(player_id)?;
    let player_state = lobby
        .bowling
        .players
        .get(player_id)
        .cloned()
        .unwrap_or_default();
    let frames = build_frame_views(&player_state);
    let total_score = frames
        .iter()
        .filter_map(|frame| frame.cumulative_score)
        .next_back()
        .unwrap_or(0);
    let finished = player_finished(&player_state);
    let status_label = if player_id == current_player_id {
        if lobby.bowling.game_over {
            "Done".to_string()
        } else {
            format!("Frame {}", player_frame_number(&player_state))
        }
    } else if finished {
        "Done".to_string()
    } else {
        format!("Frame {}", player_frame_number(&player_state))
    };

    Some(PlayerScoreView {
        player_id: player_id.to_string(),
        username: player.username.clone(),
        total_score,
        frames,
        status_label,
        finished,
    })
}

fn build_frame_views(player_state: &PlayerBowlingState) -> Vec<FrameScoreView> {
    let mut views = Vec::with_capacity(10);
    let mut running_total = 0u16;
    for frame_index in 0..10 {
        let frame = player_state.frames.get(frame_index);
        let rolls = frame.map_or_else(Vec::new, |frame| frame_marks(frame_index, &frame.rolls));
        let frame_score = frame.and_then(|_| score_frame(&player_state.frames, frame_index));
        let cumulative_score = frame_score.map(|score| {
            running_total += score;
            running_total
        });
        views.push(FrameScoreView {
            rolls,
            cumulative_score,
        });
    }
    views
}

fn frame_marks(frame_index: usize, rolls: &[u8]) -> Vec<String> {
    if rolls.is_empty() {
        return Vec::new();
    }
    if frame_index < 9 {
        if rolls[0] == 10 {
            return vec!["X".to_string()];
        }
        let mut marks = vec![roll_mark(rolls[0])];
        if let Some(&second) = rolls.get(1) {
            if rolls[0] + second == 10 {
                marks.push("/".to_string());
            } else {
                marks.push(roll_mark(second));
            }
        }
        return marks;
    }

    let mut marks = Vec::new();
    if let Some(&first) = rolls.first() {
        marks.push(if first == 10 {
            "X".to_string()
        } else {
            roll_mark(first)
        });
    }
    if let Some(&second) = rolls.get(1) {
        let first = rolls[0];
        let second_mark = if first == 10 {
            if second == 10 {
                "X".to_string()
            } else {
                roll_mark(second)
            }
        } else if first + second == 10 {
            "/".to_string()
        } else {
            roll_mark(second)
        };
        marks.push(second_mark);
    }
    if let Some(&third) = rolls.get(2) {
        let first = rolls[0];
        let second = rolls[1];
        let third_mark = if first == 10 {
            if second == 10 {
                if third == 10 {
                    "X".to_string()
                } else {
                    roll_mark(third)
                }
            } else if second + third == 10 {
                "/".to_string()
            } else {
                roll_mark(third)
            }
        } else if first + second == 10 {
            if third == 10 {
                "X".to_string()
            } else {
                roll_mark(third)
            }
        } else {
            roll_mark(third)
        };
        marks.push(third_mark);
    }
    marks
}

fn roll_mark(pins: u8) -> String {
    if pins == 0 {
        "-".to_string()
    } else {
        pins.to_string()
    }
}

fn score_frame(frames: &[FrameRecord], frame_index: usize) -> Option<u16> {
    let frame = frames.get(frame_index)?;
    if frame_index == 9 {
        if frame_complete(frame_index, frame) {
            return Some(frame.rolls.iter().map(|&roll| roll as u16).sum());
        }
        return None;
    }

    let first = *frame.rolls.first()?;
    if first == 10 {
        let bonuses = subsequent_rolls(frames, frame_index + 1);
        if bonuses.len() >= 2 {
            return Some(10 + bonuses[0] as u16 + bonuses[1] as u16);
        }
        return None;
    }

    let second = *frame.rolls.get(1)?;
    if first + second == 10 {
        let bonus = subsequent_rolls(frames, frame_index + 1).first().copied()?;
        return Some(10 + bonus as u16);
    }

    Some(first as u16 + second as u16)
}

fn subsequent_rolls(frames: &[FrameRecord], start_index: usize) -> Vec<u8> {
    let mut rolls = Vec::new();
    for frame in frames.iter().skip(start_index) {
        rolls.extend(frame.rolls.iter().copied());
    }
    rolls
}

fn player_frame_number(player_state: &PlayerBowlingState) -> u8 {
    let active_frame = active_frame_index(player_state).min(9);
    (active_frame + 1) as u8
}

fn active_frame_index(player_state: &PlayerBowlingState) -> usize {
    if let Some((idx, _)) = player_state
        .frames
        .iter()
        .enumerate()
        .find(|(idx, frame)| !frame_complete(*idx, frame))
    {
        idx
    } else {
        player_state.frames.len()
    }
}

fn player_finished(player_state: &PlayerBowlingState) -> bool {
    player_state.frames.len() >= 10
        && player_state
            .frames
            .get(9)
            .is_some_and(|frame| frame_complete(9, frame))
}

fn frame_complete(frame_index: usize, frame: &FrameRecord) -> bool {
    if frame_index < 9 {
        frame.rolls.first() == Some(&10) || frame.rolls.len() >= 2
    } else {
        match frame.rolls.as_slice() {
            [] | [_] => false,
            [first, second] => *first + *second < 10 && *first != 10,
            _ => true,
        }
    }
}

fn continuing_turn_state(frame_index: usize, rolls: &[u8]) -> Option<(u8, u8)> {
    if frame_index < 9 {
        if rolls.len() == 1 && rolls[0] < 10 {
            return Some((2, 10 - rolls[0]));
        }
        return None;
    }

    match rolls {
        [first] => {
            if *first == 10 {
                Some((2, 10))
            } else {
                Some((2, 10 - *first))
            }
        }
        [first, second] => {
            if *first == 10 {
                if *second == 10 {
                    Some((3, 10))
                } else {
                    Some((3, 10 - *second))
                }
            } else if *first + *second == 10 {
                Some((3, 10))
            } else {
                None
            }
        }
        _ => None,
    }
}

fn all_players_finished(lobby: &Lobby) -> bool {
    lobby.player_order.iter().all(|player_id| {
        lobby
            .bowling
            .players
            .get(player_id)
            .is_some_and(player_finished)
    })
}

fn rotate_player_order(order: &[String], start: usize) -> Vec<String> {
    if order.is_empty() {
        return Vec::new();
    }
    let mut rotated = Vec::with_capacity(order.len());
    for offset in 0..order.len() {
        rotated.push(order[(start + offset) % order.len()].clone());
    }
    rotated
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
        let finished = lobby
            .bowling
            .players
            .get(player_id)
            .is_some_and(player_finished);
        if player.tx.is_some() && !finished {
            return Some(idx);
        }
    }

    None
}

async fn advance_turn_after_player_change(state: &SharedState, code: &str) {
    let scoreboard = {
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
            lobby.bowling.current_roll = 0;
            lobby.bowling.pins_remaining = 0;
            return;
        };
        lobby.current_turn = Some(next_turn);
        Some(build_scoreboard(lobby))
    };

    if let Some(scoreboard) = scoreboard {
        broadcast_lobby(
            state,
            code,
            &ServerMessage::ScoreboardUpdated { scoreboard },
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
