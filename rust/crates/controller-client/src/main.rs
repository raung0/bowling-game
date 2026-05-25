use std::{collections::VecDeque, str::FromStr, sync::mpsc, thread};

use common::{ClientMessage, PlayerInfo, ScoreboardState, ServerMessage};
use eframe::egui::{self, Color32};
use futures_util::{SinkExt, StreamExt};
use tokio::sync::mpsc as tokio_mpsc;
use tokio_tungstenite::{connect_async, tungstenite::Message};

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions::default();
    eframe::run_native(
        "Bowling Controller Client",
        options,
        Box::new(|_cc| Ok(Box::new(ControllerApp::new()))),
    )
}

struct ControllerApp {
    command_tx: tokio_mpsc::UnboundedSender<NetCommand>,
    event_rx: mpsc::Receiver<UiEvent>,
    server_url: String,
    lobby_code: String,
    username: String,
    force: f32,
    direction_x: f32,
    direction_z: f32,
    connected: bool,
    joined: bool,
    status: String,
    lobby_players: Vec<PlayerInfo>,
    scoreboard: Option<ScoreboardState>,
    log: VecDeque<String>,
}

impl ControllerApp {
    fn new() -> Self {
        let (command_tx, command_rx) = tokio_mpsc::unbounded_channel();
        let (event_tx, event_rx) = mpsc::channel();

        thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .expect("failed to build tokio runtime");
            runtime.block_on(network_loop(command_rx, event_tx));
        });

        Self {
            command_tx,
            event_rx,
            server_url: "ws://127.0.0.1:3000/ws".to_string(),
            lobby_code: String::new(),
            username: String::new(),
            force: 0.5,
            direction_x: 0.0,
            direction_z: 1.0,
            connected: false,
            joined: false,
            status: "idle".to_string(),
            lobby_players: Vec::new(),
            scoreboard: None,
            log: VecDeque::with_capacity(64),
        }
    }

    fn push_log(&mut self, line: impl Into<String>) {
        if self.log.len() >= 64 {
            self.log.pop_front();
        }
        self.log.push_back(line.into());
    }

    fn send_command(&self, cmd: NetCommand) {
        let _ = self.command_tx.send(cmd);
    }

    fn connect(&mut self) {
        self.status = format!("connecting to {}", self.server_url);
        self.push_log(self.status.clone());
        self.send_command(NetCommand::Connect {
            url: self.server_url.clone(),
        });
    }

    fn disconnect(&mut self) {
        self.send_command(NetCommand::Disconnect);
    }

    fn join_lobby(&mut self) {
        if self.lobby_code.trim().is_empty() || self.username.trim().is_empty() {
            self.status = "lobby code and username are required".to_string();
            return;
        }

        self.send_command(NetCommand::Send(ClientMessage::JoinLobby {
            code: self.lobby_code.trim().to_string(),
            username: self.username.trim().to_string(),
        }));
    }

    fn leave_lobby(&mut self) {
        self.send_command(NetCommand::Send(ClientMessage::Leave));
        self.joined = false;
        self.lobby_players.clear();
        self.scoreboard = None;
    }

    fn send_throw(&mut self) {
        self.send_command(NetCommand::Send(ClientMessage::ThrowEvent {
            force: self.force,
            direction_x: self.direction_x,
            direction_z: self.direction_z,
        }));
        self.push_log(format!(
            "sent throw: force={:.2}, x={:.2}, z={:.2}",
            self.force, self.direction_x, self.direction_z
        ));
    }

    fn handle_event(&mut self, event: UiEvent) {
        match event {
            UiEvent::Status(text) => {
                self.status = text.clone();
                self.push_log(text);
            }
            UiEvent::Connected(url) => {
                self.connected = true;
                self.status = format!("connected to {url}");
                self.push_log(self.status.clone());
            }
            UiEvent::Disconnected(reason) => {
                self.connected = false;
                self.joined = false;
                self.lobby_players.clear();
                self.scoreboard = None;
                self.status = format!("disconnected: {reason}");
                self.push_log(self.status.clone());
            }
            UiEvent::ServerMessage(message) => {
                self.apply_server_message(message);
            }
            UiEvent::Error(message) => {
                self.status = format!("error: {message}");
                self.push_log(self.status.clone());
            }
        }
    }

    fn apply_server_message(&mut self, message: ServerMessage) {
        match message {
            ServerMessage::LobbyCreated { code, .. } => {
                self.lobby_code = code.clone();
                self.joined = false;
                self.status = format!("host created lobby {code}");
                self.push_log(self.status.clone());
            }
            ServerMessage::LobbyJoined { code, players, .. } => {
                self.lobby_code = code.clone();
                self.joined = true;
                self.lobby_players = players;
                self.status = format!("joined lobby {code}");
                self.push_log(self.status.clone());
            }
            ServerMessage::LobbyUpdated { code, players } => {
                self.lobby_code = code;
                self.lobby_players = players;
            }
            ServerMessage::ReconnectOkPlayer { code, players, .. } => {
                self.lobby_code = code;
                self.joined = true;
                self.lobby_players = players;
                self.status = "reconnected as player".to_string();
                self.push_log(self.status.clone());
            }
            ServerMessage::ReconnectOkHost { .. } => {
                self.status = "host reconnect not used by controller client".to_string();
                self.push_log(self.status.clone());
            }
            ServerMessage::GameStarted {
                code,
                current_player_id,
            } => {
                self.status = format!("game started in {code}, turn: {current_player_id}");
                self.push_log(self.status.clone());
            }
            ServerMessage::TurnChanged { current_player_id } => {
                self.status = format!("turn changed: {current_player_id}");
                self.push_log(self.status.clone());
            }
            ServerMessage::ScoreboardUpdated { scoreboard } => {
                self.scoreboard = Some(scoreboard);
            }
            ServerMessage::GameStopped { code } => {
                self.status = format!("game stopped in {code}");
                self.push_log(self.status.clone());
            }
            ServerMessage::ThrowEvent {
                player_id,
                force,
                direction_x,
                direction_z,
            } => {
                self.push_log(format!(
                    "throw relayed for {player_id}: force={force:.2}, x={direction_x:.2}, z={direction_z:.2}"
                ));
            }
            ServerMessage::Info { message } => {
                self.status = message.clone();
                self.push_log(message);
            }
            ServerMessage::Error { message } => {
                self.status = format!("server error: {message}");
                self.push_log(self.status.clone());
            }
        }
    }
}

impl eframe::App for ControllerApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        while let Ok(event) = self.event_rx.try_recv() {
            self.handle_event(event);
        }

        egui::TopBottomPanel::top("top").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label("Server");
                ui.text_edit_singleline(&mut self.server_url);
                if ui.button("Connect").clicked() {
                    self.connect();
                }
                if ui.button("Disconnect").clicked() {
                    self.disconnect();
                }
            });
            ui.horizontal(|ui| {
                ui.label(format!("Status: {}", self.status));
                ui.separator();
                ui.label(if self.connected {
                    "connected"
                } else {
                    "offline"
                });
                ui.separator();
                ui.label(if self.joined { "joined" } else { "not joined" });
            });
        });

        egui::SidePanel::left("left")
            .resizable(true)
            .show(ctx, |ui| {
                ui.heading("Lobby");
                ui.label("Lobby code");
                ui.text_edit_singleline(&mut self.lobby_code);
                ui.label("Username");
                ui.text_edit_singleline(&mut self.username);

                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(self.connected, egui::Button::new("Join lobby"))
                        .clicked()
                    {
                        self.join_lobby();
                    }
                    if ui
                        .add_enabled(self.connected && self.joined, egui::Button::new("Leave"))
                        .clicked()
                    {
                        self.leave_lobby();
                    }
                });

                ui.separator();
                ui.heading("Controller");
                ui.add_enabled_ui(self.connected && self.joined, |ui| {
                    ui.add(egui::Slider::new(&mut self.force, 0.0..=1.0).text("Force"));
                    ui.add(
                        egui::Slider::new(&mut self.direction_x, -1.0..=1.0).text("Direction X"),
                    );
                    ui.add(egui::Slider::new(&mut self.direction_z, 0.0..=1.0).text("Direction Z"));

                    if ui.button("Send throw").clicked() {
                        self.send_throw();
                    }
                });

                ui.separator();
                ui.heading("Lobby players");
                for player in &self.lobby_players {
                    let color = if player.connected {
                        Color32::LIGHT_GREEN
                    } else {
                        Color32::LIGHT_RED
                    };
                    ui.colored_label(color, format!("{} ({})", player.username, player.player_id));
                }
            });

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("Scoreboard");
            if let Some(scoreboard) = &self.scoreboard {
                ui.label(format!(
                    "Current player: {} | frame {} | roll {} | pins {}",
                    scoreboard.current_player_id,
                    scoreboard.current_frame,
                    scoreboard.current_roll,
                    scoreboard.pins_remaining,
                ));
                ui.label(format!("Game over: {}", scoreboard.game_over));
                ui.separator();
                for player in &scoreboard.players {
                    ui.group(|ui| {
                        ui.label(format!("{} - {}", player.username, player.total_score));
                        ui.label(&player.status_label);
                    });
                }
            } else {
                ui.label("No scoreboard data yet.");
            }

            ui.separator();
            ui.heading("Log");
            for line in self.log.iter().rev().take(16) {
                ui.label(line);
            }
        });

        ctx.request_repaint();
    }
}

enum NetCommand {
    Connect { url: String },
    Disconnect,
    Send(ClientMessage),
}

enum UiEvent {
    Connected(String),
    Disconnected(String),
    Status(String),
    ServerMessage(ServerMessage),
    Error(String),
}

async fn network_loop(
    mut command_rx: tokio_mpsc::UnboundedReceiver<NetCommand>,
    event_tx: mpsc::Sender<UiEvent>,
) {
    let mut pending_connection: Option<String> = None;

    loop {
        let command = if let Some(url) = pending_connection.take() {
            NetCommand::Connect { url }
        } else {
            match command_rx.recv().await {
                Some(command) => command,
                None => break,
            }
        };

        match command {
            NetCommand::Connect { url } => match connect_async(&url).await {
                Ok((stream, _)) => {
                    let _ = event_tx.send(UiEvent::Connected(url.clone()));
                    if let Some(next_url) =
                        run_connected_loop(stream, &mut command_rx, &event_tx).await
                    {
                        pending_connection = Some(next_url);
                    }
                }
                Err(err) => {
                    let _ = event_tx.send(UiEvent::Error(format!("connect failed: {err}")));
                }
            },
            NetCommand::Disconnect => {
                let _ = event_tx.send(UiEvent::Status("already disconnected".to_string()));
            }
            NetCommand::Send(_) => {
                let _ = event_tx.send(UiEvent::Status("not connected".to_string()));
            }
        }
    }
}

async fn run_connected_loop<S>(
    stream: tokio_tungstenite::WebSocketStream<S>,
    command_rx: &mut tokio_mpsc::UnboundedReceiver<NetCommand>,
    event_tx: &mpsc::Sender<UiEvent>,
) -> Option<String>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let (mut ws_sink, mut ws_stream) = stream.split();

    loop {
        tokio::select! {
            maybe_cmd = command_rx.recv() => {
                let Some(command) = maybe_cmd else {
                    let _ = event_tx.send(UiEvent::Disconnected("command channel closed".to_string()));
                    let _ = ws_sink.close().await;
                    return None;
                };

                match command {
                    NetCommand::Connect { url } => {
                        let _ = event_tx.send(UiEvent::Status(format!("reconnecting to {url}")));
                        let _ = ws_sink.close().await;
                        return Some(url);
                    }
                    NetCommand::Disconnect => {
                        let _ = ws_sink.close().await;
                        let _ = event_tx.send(UiEvent::Disconnected("user disconnected".to_string()));
                        return None;
                    }
                    NetCommand::Send(message) => {
                        if let Err(err) = send_client_message(&mut ws_sink, message).await {
                            let _ = event_tx.send(UiEvent::Disconnected(format!("send failed: {err}")));
                            return None;
                        }
                    }
                }
            }
            maybe_msg = ws_stream.next() => {
                let Some(msg) = maybe_msg else {
                    let _ = event_tx.send(UiEvent::Disconnected("server closed connection".to_string()));
                    return None;
                };

                match msg {
                    Ok(Message::Text(text)) => match ServerMessage::from_str(&text) {
                        Ok(message) => {
                            let _ = event_tx.send(UiEvent::ServerMessage(message));
                        }
                        Err(err) => {
                            let _ = event_tx.send(UiEvent::Error(format!("invalid server message: {err}")));
                        }
                    },
                    Ok(Message::Close(_)) => {
                        let _ = event_tx.send(UiEvent::Disconnected("server closed connection".to_string()));
                        return None;
                    }
                    Ok(Message::Ping(payload)) => {
                        let _ = ws_sink.send(Message::Pong(payload)).await;
                    }
                    Ok(Message::Binary(_)) | Ok(Message::Pong(_)) => {}
                    Err(err) => {
                        let _ = event_tx.send(UiEvent::Disconnected(format!("socket error: {err}")));
                        return None;
                    }
                    _ => {}
                }
            }
        }
    }
}

async fn send_client_message<S>(
    ws_sink: &mut futures_util::stream::SplitSink<tokio_tungstenite::WebSocketStream<S>, Message>,
    message: ClientMessage,
) -> Result<(), String>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let payload = message.encode().map_err(|err| err.to_string())?;
    ws_sink
        .send(Message::Text(payload.into()))
        .await
        .map_err(|err| err.to_string())
}
