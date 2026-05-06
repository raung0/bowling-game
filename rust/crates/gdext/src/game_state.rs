use common::{ClientMessage, PlayerInfo, ServerMessage};
use getset::Getters;
use godot::{
    classes::{Button, Label, LineEdit, Node, VBoxContainer, WebSocketPeer},
    prelude::*,
};
use rand::Rng;

use crate::ui_manager::UiManager;

const STORAGE_ROLE: &str = "session_role";
const STORAGE_TOKEN: &str = "session_token";
const STORAGE_CODE: &str = "lobby_code";
const STORAGE_USERNAME: &str = "username";

#[derive(Clone, Copy)]
pub enum Screen {
    MainMenu,
    Host,
    Info,
    Game,
}

impl Default for Screen {
    fn default() -> Self {
        Screen::MainMenu
    }
}

#[derive(Clone)]
enum SessionRole {
    Host,
    Player,
}

#[derive(GodotClass, Getters)]
#[class(base=Node)]
pub struct GameState {
    #[getset(get = "pub")]
    is_mobile: bool,
    #[getset(get = "pub")]
    accel: Vector3,
    #[getset(get = "pub", set = "pub")]
    screen: Screen,
    info_text: String,
    ws: Option<Gd<WebSocketPeer>>,

    base: Base<Node>,
}

#[godot_api]
impl INode for GameState {
    fn init(base: Base<Node>) -> Self {
        Self {
            base,
            is_mobile: false,
            accel: Vector3::ZERO,
            screen: Screen::default(),
            info_text: String::new(),
            ws: None,
        }
    }

    fn ready(&mut self) {
        self.base_mut().set_process(true);

        let mut join_button = self
            .base()
            .get_node_as::<Button>("UiManager/Mobile/VBoxContainer/Join");
        let mut create_button = self.base().get_node_as::<Button>(
            "UiManager/CenterContainer/VBoxContainer/Desktop/VBoxContainer/Create",
        );
        let mut spectate_button = self.base().get_node_as::<Button>(
            "UiManager/CenterContainer/VBoxContainer/Desktop/VBoxContainer/Spectate",
        );
        let mut back_button = self.base().get_node_as::<Button>("UiManager/CenterContainer/VBoxContainer/DesktopHost/PanelContainer/MarginContainer/VBoxContainer/Actions/Back");
        let mut start_button = self.base().get_node_as::<Button>("UiManager/CenterContainer/VBoxContainer/DesktopHost/PanelContainer/MarginContainer/VBoxContainer/Actions/Start");

        join_button.connect("pressed", &self.base().callable("on_join_pressed"));
        create_button.connect("pressed", &self.base().callable("on_create_pressed"));
        spectate_button.connect("pressed", &self.base().callable("on_spectate_pressed"));
        back_button.connect("pressed", &self.base().callable("on_back_pressed"));
        start_button.connect("pressed", &self.base().callable("on_start_pressed"));

        self.ensure_username();
        self.try_auto_reconnect();
    }

    fn process(&mut self, _delta: f64) {
        self.accel = self.browser_accel();
        self.is_mobile = self.is_mobile_web();

        let mut accel_label = self
            .base_mut()
            .get_node_as::<Label>("UiManager/Mobile/VBoxContainer/Accel");
        accel_label.set_text(&format!(
            "Accel: x={:.2} y={:.2} z={:.2}",
            self.accel.x, self.accel.y, self.accel.z
        ));

        let mut ui_manager = self.base_mut().get_node_as::<UiManager>("UiManager");
        ui_manager
            .bind_mut()
            .set_screen(self.screen, self.is_mobile);

        self.poll_socket();
        self.render_info_text();
    }
}

impl GameState {
    #[cfg(target_arch = "wasm32")]
    fn browser_accel(&self) -> Vector3 {
        let Some(mut bridge) = self.base().get_node_or_null("WebBridge") else {
            return Vector3::ZERO;
        };
        bridge
            .call("get_accelerometer", &[])
            .try_to::<Vector3>()
            .unwrap_or(Vector3::ZERO)
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn browser_accel(&self) -> Vector3 {
        Vector3::ZERO
    }

    #[cfg(target_arch = "wasm32")]
    fn is_mobile_web(&self) -> bool {
        let Some(mut bridge) = self.base().get_node_or_null("WebBridge") else {
            return false;
        };
        bridge
            .call("is_mobile", &[])
            .try_to::<bool>()
            .unwrap_or(false)
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn is_mobile_web(&self) -> bool {
        false
    }

    fn ws_url(&self) -> String {
        #[cfg(target_arch = "wasm32")]
        {
            if let Some(mut bridge) = self.base().get_node_or_null("WebBridge") {
                let url = bridge
                    .call("get_ws_url", &[])
                    .try_to::<GString>()
                    .unwrap_or_default()
                    .to_string();
                if !url.is_empty() {
                    return url;
                }
            }
        }
        "ws://127.0.0.1:3000/ws".to_string()
    }

    fn ensure_socket(&mut self) {
        if self.ws.is_some() {
            return;
        }
        let mut ws = WebSocketPeer::new_gd();
        let url = self.ws_url();
        let _ = ws.connect_to_url(&GString::from(url.as_str()));
        self.ws = Some(ws);
    }

    fn send_message(&mut self, msg: ClientMessage) {
        self.ensure_socket();
        let Some(ws) = self.ws.as_mut() else { return };
        if let Ok(encoded) = msg.encode() {
            let _ = ws.send_text(&GString::from(encoded.as_str()));
        }
    }

    fn poll_socket(&mut self) {
        let Some(ws) = self.ws.as_mut() else { return };
        let _ = ws.poll();

        let mut messages = Vec::new();
        while ws.get_available_packet_count() > 0 {
            let packet = ws.get_packet();
            let text = String::from_utf8(packet.to_vec()).unwrap_or_default();
            if !text.is_empty() {
                messages.push(text);
            }
        }

        for text in messages {
            if let Ok(msg) = ServerMessage::from_str(&text) {
                self.apply_server_message(msg);
            }
        }
    }

    fn apply_server_message(&mut self, msg: ServerMessage) {
        match msg {
            ServerMessage::LobbyCreated {
                code,
                host_session,
                players,
            } => {
                self.persist_role_session(SessionRole::Host, &code, &host_session);
                self.update_lobby_ui(&code, &players);
                self.screen = Screen::Host;
            }
            ServerMessage::LobbyJoined {
                code,
                player_session,
                players,
            } => {
                self.persist_role_session(SessionRole::Player, &code, &player_session);
                self.update_lobby_ui(&code, &players);
                self.screen = Screen::Host;
            }
            ServerMessage::ReconnectOkHost { code, players } => {
                self.update_lobby_ui(&code, &players);
                self.screen = Screen::Host;
            }
            ServerMessage::ReconnectOkPlayer {
                code,
                player_session,
                players,
            } => {
                self.persist_role_session(SessionRole::Player, &code, &player_session);
                self.update_lobby_ui(&code, &players);
                self.screen = Screen::Host;
            }
            ServerMessage::LobbyUpdated { code, players } => {
                self.update_lobby_ui(&code, &players);
            }
            ServerMessage::Info { message } => {
                self.info_text = message;
                self.screen = Screen::Info;
            }
            ServerMessage::Error { message } => {
                self.info_text = format!("Error: {message}");
                self.clear_session_keys();
                self.screen = Screen::MainMenu;
            }
            ServerMessage::GameStarted { .. } => {
                self.screen = Screen::Game;
            }
        }
    }

    fn update_lobby_ui(&mut self, code: &str, players: &[PlayerInfo]) {
        let mut room_code = self.base_mut().get_node_as::<Label>("UiManager/CenterContainer/VBoxContainer/DesktopHost/PanelContainer/MarginContainer/VBoxContainer/RoomCodeValue");
        room_code.set_text(&GString::from(code));

        let mut players_box = self.base_mut().get_node_as::<VBoxContainer>("UiManager/CenterContainer/VBoxContainer/DesktopHost/PanelContainer/MarginContainer/VBoxContainer/Players");
        let children = players_box.get_children();
        for mut child in children.iter_shared() {
            players_box.remove_child(&child);
            child.queue_free();
        }
        for player in players {
            let mut label = Label::new_alloc();
            let suffix = if player.connected {
                ""
            } else {
                " (reconnecting...)"
            };
            let line = format!("{}{}", player.username, suffix);
            label.set_text(&GString::from(line.as_str()));
            players_box.add_child(&label);
        }
    }

    fn render_info_text(&mut self) {
        let mut info_label = self.base_mut().get_node_as::<Label>("UiManager/Info/Label");
        if self.info_text.is_empty() {
            info_label.set_text("Loading...");
        } else {
            info_label.set_text(&GString::from(self.info_text.as_str()));
        }
    }

    fn ensure_username(&self) {
        let username = self.get_storage(STORAGE_USERNAME);
        if !username.trim().is_empty() {
            return;
        }

        let mut rng = rand::rng();
        let chars: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
        let rand_part: String = (0..5)
            .map(|_| {
                let idx = rng.random_range(0..chars.len());
                chars[idx] as char
            })
            .collect();
        self.set_storage(STORAGE_USERNAME, &format!("Player_{rand_part}"));
    }

    fn try_auto_reconnect(&mut self) {
        let role = self.get_storage(STORAGE_ROLE);
        let token = self.get_storage(STORAGE_TOKEN);
        let code = self.get_storage(STORAGE_CODE);
        if role.is_empty() || token.is_empty() || code.is_empty() {
            return;
        }

        self.screen = Screen::Info;
        if role == "host" {
            self.info_text = "Reconnecting as host...".to_string();
            self.send_message(ClientMessage::ReconnectHost {
                code,
                host_session: token,
            });
        } else if role == "player" {
            self.info_text = "Reconnecting as player...".to_string();
            self.send_message(ClientMessage::ReconnectPlayer {
                code,
                player_session: token,
            });
        }
    }

    fn persist_role_session(&self, role: SessionRole, code: &str, session: &str) {
        let role_str = match role {
            SessionRole::Host => "host",
            SessionRole::Player => "player",
        };
        self.set_storage(STORAGE_ROLE, role_str);
        self.set_storage(STORAGE_TOKEN, session);
        self.set_storage(STORAGE_CODE, code);
    }

    fn clear_session_keys(&self) {
        self.clear_storage(STORAGE_ROLE);
        self.clear_storage(STORAGE_TOKEN);
        self.clear_storage(STORAGE_CODE);
    }

    fn get_join_username(&mut self) -> String {
        let mut username_input = self
            .base_mut()
            .get_node_as::<LineEdit>("UiManager/Mobile/VBoxContainer/Username");
        let typed = username_input.get_text().to_string().trim().to_string();
        if !typed.is_empty() {
            self.set_storage(STORAGE_USERNAME, &typed);
            return typed;
        }
        let fallback = self.get_storage(STORAGE_USERNAME);
        username_input.set_text(&GString::from(fallback.as_str()));
        fallback
    }

    fn get_storage(&self, key: &str) -> String {
        let Some(mut bridge) = self.base().get_node_or_null("WebBridge") else {
            return String::new();
        };
        bridge
            .call("get_local_value", &[key.to_variant()])
            .try_to::<GString>()
            .unwrap_or_default()
            .to_string()
    }

    fn set_storage(&self, key: &str, value: &str) {
        let Some(mut bridge) = self.base().get_node_or_null("WebBridge") else {
            return;
        };
        bridge.call(
            "set_local_value",
            &[key.to_variant(), GString::from(value).to_variant()],
        );
    }

    fn clear_storage(&self, key: &str) {
        let Some(mut bridge) = self.base().get_node_or_null("WebBridge") else {
            return;
        };
        bridge.call("clear_local_value", &[key.to_variant()]);
    }
}

#[godot_api]
impl GameState {
    #[func]
    fn on_join_pressed(&mut self) {
        let code = self
            .base()
            .get_node_as::<LineEdit>("UiManager/Mobile/VBoxContainer/LobbyCode")
            .get_text()
            .to_string()
            .trim()
            .to_string();
        if code.len() != 5 || !code.chars().all(|c| c.is_ascii_digit()) {
            self.info_text = "Lobby code must be 5 digits".to_string();
            self.screen = Screen::Info;
            return;
        }

        let username = self.get_join_username();
        self.info_text = "Joining lobby...".to_string();
        self.screen = Screen::Info;
        self.send_message(ClientMessage::JoinLobby { code, username });
    }

    #[func]
    fn on_create_pressed(&mut self) {
        self.info_text = "Creating lobby...".to_string();
        self.screen = Screen::Info;
        self.send_message(ClientMessage::CreateLobby);
    }

    #[func]
    fn on_spectate_pressed(&mut self) {
        self.screen = Screen::Game;
    }

    #[func]
    fn on_back_pressed(&mut self) {
        self.send_message(ClientMessage::Leave);
        self.clear_session_keys();
        self.screen = Screen::MainMenu;
    }

    #[func]
    fn on_start_pressed(&mut self) {
        self.send_message(ClientMessage::StartGame);
        self.info_text = "Starting game...".to_string();
        self.screen = Screen::Info;
    }
}
