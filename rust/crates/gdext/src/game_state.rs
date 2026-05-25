use common::{
    ClientMessage, PlayerInfo, ScoreboardState, ServerMessage, BALL_MOVE_Z_MAX, BALL_MOVE_Z_MIN,
    BALL_MOVE_Z_STEP, BALL_ROT_Y_MAX_DEG, BALL_ROT_Y_STEP_DEG,
    CONTROLLER_HOLD_REPEAT_DELAY_SECS, CONTROLLER_HOLD_REPEAT_INTERVAL_SECS,
};
use getset::Getters;
use godot::{
    classes::web_socket_peer::State as WebSocketState,
    classes::{Button, LineEdit, Node, Node3D, PackedScene, WebSocketPeer},
    global::Error,
    prelude::*,
};
use rand::Rng;
use std::{collections::VecDeque, str::FromStr};

use crate::ball::Ball;
use crate::game_manager::GameManager;
use crate::ui_manager::UiManager;
use crate::{controller, rendering, storage};

const SOCKET_CONNECT_TIMEOUT_SECS: f64 = 5.0;
const THROW_SETTLE_SECS: f64 = 3.0;
const LEAVE_HOLD_SECS: f64 = 5.0;

#[derive(Clone, Copy, Default)]
pub enum Screen {
    #[default]
    MainMenu,
    Host,
    Info,
    Game,
    Controller,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SessionRole {
    Host,
    Player,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum BallSetupAction {
    MoveLeft,
    MoveRight,
    RotateLeft,
    RotateRight,
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
    pending_messages: Vec<String>,
    pending_connect_secs: f64,
    session_role: Option<SessionRole>,
    player_id: String,
    lobby_code: String,
    players: Vec<PlayerInfo>,
    current_player_id: String,
    scoreboard: ScoreboardState,
    controller_holding: bool,
    controller_force: f32,
    controller_direction: Vector2,
    controller_baseline_accel: Vector3,
    controller_motion_history: VecDeque<Vector3>,
    calibration_active: bool,
    calibration_samples: Vec<f32>,
    calibration_offset_x: f32,
    host_ball_was_launched: bool,
    host_throw_start_fallen: i32,
    host_throw_waiting_report: bool,
    host_throw_settle_secs: f64,
    pending_ball_respawn: bool,
    ball_start_initial_transform: Option<Transform3D>,
    ball_setup_action: Option<BallSetupAction>,
    ball_setup_hold_secs: f64,
    ball_setup_next_repeat_secs: f64,
    zoomed_in: bool,
    leave_holding: bool,
    leave_hold_secs: f64,

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
            pending_messages: Vec::new(),
            pending_connect_secs: 0.0,
            session_role: None,
            player_id: String::new(),
            lobby_code: String::new(),
            players: Vec::new(),
            current_player_id: String::new(),
            scoreboard: ScoreboardState::default(),
            controller_holding: false,
            controller_force: 0.0,
            controller_direction: Vector2::new(0.0, 1.0),
            controller_baseline_accel: Vector3::ZERO,
            controller_motion_history: VecDeque::with_capacity(controller::MOTION_HISTORY_LIMIT),
            calibration_active: false,
            calibration_samples: Vec::new(),
            calibration_offset_x: 0.0,
            host_ball_was_launched: false,
            host_throw_start_fallen: 0,
            host_throw_waiting_report: false,
            host_throw_settle_secs: 0.0,
            pending_ball_respawn: false,
            ball_start_initial_transform: None,
            ball_setup_action: None,
            ball_setup_hold_secs: 0.0,
            ball_setup_next_repeat_secs: 0.0,
            zoomed_in: false,
            leave_holding: false,
            leave_hold_secs: 0.0,
        }
    }

    fn ready(&mut self) {
        self.base_mut().set_process(true);
        self.capture_ball_start_initial_transform();
        self.set_zoomed_in(false);
        self.pending_ball_respawn = true;

        let mut join_button = self
            .base()
            .get_node_as::<Button>("UiManager/Mobile/VBoxContainer/Join");
        let mut calibrate_button = self
            .base()
            .get_node_as::<Button>("UiManager/Mobile/VBoxContainer/Calibrate");
        let mut create_button = self.base().get_node_as::<Button>(
            "UiManager/CenterContainer/VBoxContainer/Desktop/VBoxContainer/Create",
        );
        let mut spectate_button = self.base().get_node_as::<Button>(
            "UiManager/CenterContainer/VBoxContainer/Desktop/VBoxContainer/Spectate",
        );
        let mut back_button = self.base().get_node_as::<Button>(
            "UiManager/CenterContainer/VBoxContainer/DesktopHost/PanelContainer/MarginContainer/VBoxContainer/Actions/Back",
        );
        let mut start_button = self.base().get_node_as::<Button>(
            "UiManager/CenterContainer/VBoxContainer/DesktopHost/PanelContainer/MarginContainer/VBoxContainer/Actions/Start",
        );
        let mut host_stop_button = self.base().get_node_as::<Button>(
            "UiManager/CenterContainer/VBoxContainer/DesktopHost/PanelContainer/MarginContainer/VBoxContainer/Actions/Stop",
        );
        let mut host_kick_button = self.base().get_node_as::<Button>(
            "UiManager/CenterContainer/VBoxContainer/DesktopHost/PanelContainer/MarginContainer/VBoxContainer/KickRow/KickButton",
        );
        let mut hold_button = self
            .base()
            .get_node_as::<Button>("UiManager/Controller/MarginContainer/VBoxContainer/HoldButton");
        let mut move_left_button = self
            .base()
            .get_node_as::<Button>("UiManager/Controller/MarginContainer/VBoxContainer/Move/Left");
        let mut move_right_button = self
            .base()
            .get_node_as::<Button>("UiManager/Controller/MarginContainer/VBoxContainer/Move/Right");
        let mut rotate_left_button = self
            .base()
            .get_node_as::<Button>("UiManager/Controller/MarginContainer/VBoxContainer/Rotate/Left");
        let mut rotate_right_button = self
            .base()
            .get_node_as::<Button>("UiManager/Controller/MarginContainer/VBoxContainer/Rotate/Right");
        let mut controller_back_button = self
            .base()
            .get_node_as::<Button>("UiManager/Controller/MarginContainer/VBoxContainer/BackToMenu");
        let mut game_kick_button = self.base().get_node_as::<Button>(
            "UiManager/GameHud/MarginContainer/VBoxContainer/HostKickRow/KickButton",
        );
        let mut game_stop_button = self.base().get_node_as::<Button>(
            "UiManager/GameHud/MarginContainer/VBoxContainer/StopGameButton",
        );
        let mut zoom_button = self
            .base()
            .get_node_as::<Button>("UiManager/Controller/MarginContainer/VBoxContainer/Zoom");

        join_button.connect("pressed", &self.base().callable("on_join_pressed"));
        calibrate_button.connect("pressed", &self.base().callable("on_calibrate_pressed"));
        create_button.connect("pressed", &self.base().callable("on_create_pressed"));
        spectate_button.connect("pressed", &self.base().callable("on_spectate_pressed"));
        back_button.connect("button_down", &self.base().callable("on_leave_button_down"));
        back_button.connect("button_up", &self.base().callable("on_leave_button_up"));
        start_button.connect("pressed", &self.base().callable("on_start_pressed"));
        host_stop_button.connect("pressed", &self.base().callable("on_stop_game_pressed"));
        host_kick_button.connect("pressed", &self.base().callable("on_kick_pressed"));
        hold_button.connect("button_down", &self.base().callable("on_hold_button_down"));
        hold_button.connect("button_up", &self.base().callable("on_hold_button_up"));
        move_left_button.connect("button_down", &self.base().callable("on_move_left_button_down"));
        move_left_button.connect("button_up", &self.base().callable("on_move_button_up"));
        move_right_button.connect("button_down", &self.base().callable("on_move_right_button_down"));
        move_right_button.connect("button_up", &self.base().callable("on_move_button_up"));
        rotate_left_button.connect("button_down", &self.base().callable("on_rotate_left_button_down"));
        rotate_left_button.connect("button_up", &self.base().callable("on_rotate_button_up"));
        rotate_right_button.connect("button_down", &self.base().callable("on_rotate_right_button_down"));
        rotate_right_button.connect("button_up", &self.base().callable("on_rotate_button_up"));
        controller_back_button
            .connect("button_down", &self.base().callable("on_leave_button_down"));
        controller_back_button.connect("button_up", &self.base().callable("on_leave_button_up"));
        game_kick_button.connect("pressed", &self.base().callable("on_kick_pressed"));
        game_stop_button.connect("pressed", &self.base().callable("on_stop_game_pressed"));
        zoom_button.connect("pressed", &self.base().callable("on_zoom_pressed"));

        self.ensure_username();
        self.load_calibration();
        self.sync_username_input();
        self.try_auto_reconnect();
    }

    fn process(&mut self, delta: f64) {
        self.accel = self.browser_accel();
        self.is_mobile = self.is_mobile_web();

        self.process_pending_ball_respawn();
        self.poll_socket();
        self.handle_socket_connect_timeout(delta);
        self.update_controller_strength();
        self.update_ball_setup_hold(delta);
        self.update_leave_hold(delta);
        self.maybe_finish_host_throw(delta);

        let is_mobile = self.is_mobile;
        {
            let mut ui_manager = self.base_mut().get_node_as::<UiManager>("UiManager");
            ui_manager.bind_mut().set_screen(self.screen, is_mobile);
        }

        let accel = self.accel;
        let calibration_active = self.calibration_active;
        let calibration_samples_len = self.calibration_samples.len();
        let calibration_offset_x = self.calibration_offset_x;

        let controller_status = self.controller_status_text();
        let controller_force = self.controller_force;
        let controller_direction = self.controller_direction;
        let controller_enabled = self.calibration_active || self.is_local_player_turn();

        let current_player_label = if self.current_player_id.is_empty() {
            "Current turn: waiting for player".to_string()
        } else {
            format!("Current turn: {}", self.current_player_name())
        };
        let lobby_code = self.lobby_code.clone();
        let is_host = self.is_host();
        let scoreboard = self.scoreboard.clone();
        let info_text = self.info_text.clone();

        rendering::render_mobile_accel(self, accel);
        rendering::render_calibration_status(
            self,
            calibration_active,
            calibration_samples_len,
            calibration_offset_x,
            controller::CALIBRATION_THROW_COUNT,
        );
        rendering::render_controller_ui(
            self,
            &controller_status,
            controller_force,
            controller_direction,
            controller_enabled,
            self.zoomed_in,
        );
        rendering::render_game_ui(self, &current_player_label, &lobby_code, is_host);
        rendering::render_scoreboard(self, &scoreboard);
        rendering::render_info_text(self, &info_text);
    }
}

impl GameState {
    fn is_host(&self) -> bool {
        self.session_role == Some(SessionRole::Host)
    }

    fn gameplay_screen(&self) -> Screen {
        if self.is_mobile && self.session_role == Some(SessionRole::Player) {
            Screen::Controller
        } else {
            Screen::Game
        }
    }

    fn is_local_player_turn(&self) -> bool {
        !self.player_id.is_empty() && self.player_id == self.current_player_id
    }

    fn current_player_name(&self) -> String {
        self.players
            .iter()
            .find(|player| player.player_id == self.current_player_id)
            .map(|player| player.username.clone())
            .unwrap_or_else(|| "Waiting for player".to_string())
    }

    fn controller_status_text(&self) -> String {
        if self.calibration_active {
            format!(
                "Calibration throw {}/{}: hold, throw straight, release",
                self.calibration_samples.len() + 1,
                controller::CALIBRATION_THROW_COUNT
            )
        } else if self.current_player_id.is_empty() {
            "Waiting for host...".to_string()
        } else if self.is_local_player_turn() {
            if self.controller_holding {
                "Throw now, then let go to release".to_string()
            } else {
                "Your turn - hold to throw".to_string()
            }
        } else {
            format!("Waiting for {}", self.current_player_name())
        }
    }

    fn try_ball(&self) -> Option<Gd<Ball>> {
        self.base()
            .get_node_or_null("GameManager/BowlingBall")
            .and_then(|node| node.try_cast::<Ball>().ok())
    }

    fn try_ball_start_point(&self) -> Option<Gd<Node3D>> {
        self.base()
            .get_node_or_null("GameManager/BallStartPoint")
            .and_then(|node| node.try_cast::<Node3D>().ok())
    }

    fn capture_ball_start_initial_transform(&mut self) {
        if self.ball_start_initial_transform.is_some() {
            return;
        }

        let Some(start) = self.try_ball_start_point() else {
            return;
        };

        self.ball_start_initial_transform = Some(start.get_global_transform());
    }

    fn restore_ball_start_point_transform(&self) {
        let Some(initial_transform) = self.ball_start_initial_transform else {
            return;
        };

        let Some(mut start) = self.try_ball_start_point() else {
            return;
        };

        start.set_global_transform(initial_transform);
    }

    fn ball_setup_adjustment(action: BallSetupAction) -> (f32, f32) {
        match action {
            BallSetupAction::MoveLeft => (-BALL_MOVE_Z_STEP, 0.0),
            BallSetupAction::MoveRight => (BALL_MOVE_Z_STEP, 0.0),
            BallSetupAction::RotateLeft => (0.0, BALL_ROT_Y_STEP_DEG),
            BallSetupAction::RotateRight => (0.0, -BALL_ROT_Y_STEP_DEG),
        }
    }

    fn apply_ball_setup_adjustment(&mut self, move_z_delta: f32, rotate_y_delta_deg: f32) {
        if self.host_ball_was_launched || self.host_throw_waiting_report {
            return;
        }

        let Some(mut start) = self.try_ball_start_point() else {
            return;
        };

        if move_z_delta != 0.0 {
            let mut position = start.get_global_position();
            position.z = (position.z + move_z_delta).clamp(BALL_MOVE_Z_MIN, BALL_MOVE_Z_MAX);
            start.set_global_position(position);
        }

        if rotate_y_delta_deg != 0.0 {
            let mut rotation = start.get_global_rotation();
            let max_y = BALL_ROT_Y_MAX_DEG.to_radians();
            rotation.y = (rotation.y + rotate_y_delta_deg.to_radians()).clamp(-max_y, max_y);
            start.set_global_rotation(rotation);
        }
    }

    fn push_ball_setup_log(&self, player_id: &str, move_z_delta: f32, rotate_y_delta_deg: f32) {
        godot_print!(
            "ball setup adjust from {}: move_z_delta={:.3}, rotate_y_delta_deg={:.3}",
            player_id,
            move_z_delta,
            rotate_y_delta_deg
        );
    }

    fn set_zoomed_in(&mut self, zoomed_in: bool) {
        if self.zoomed_in == zoomed_in {
            return;
        }

        self.zoomed_in = zoomed_in;
        self.stop_ball_setup_hold();

        if let Some(mut normal_camera) = self
            .base()
            .get_node_or_null("GameManager/PhantomCameraNormal")
        {
            normal_camera.set("priority", &(if zoomed_in { 1 } else { 2 }).to_variant());
        }

        if let Some(mut zoom_camera) = self
            .base()
            .get_node_or_null("GameManager/PhantomCameraZoomedIn")
        {
            zoom_camera.set("priority", &(if zoomed_in { 2 } else { 0 }).to_variant());
        }
    }

    fn start_ball_setup_hold(&mut self, action: BallSetupAction) {
        if !self.is_local_player_turn() {
            return;
        }

        self.ball_setup_action = Some(action);
        self.ball_setup_hold_secs = 0.0;
        self.ball_setup_next_repeat_secs = CONTROLLER_HOLD_REPEAT_DELAY_SECS as f64;

        let (move_z_delta, rotate_y_delta_deg) = Self::ball_setup_adjustment(action);
        self.send_message(ClientMessage::AdjustBallSetup {
            move_z_delta,
            rotate_y_delta_deg,
        });
    }

    fn stop_ball_setup_hold(&mut self) {
        self.ball_setup_action = None;
        self.ball_setup_hold_secs = 0.0;
        self.ball_setup_next_repeat_secs = 0.0;
    }

    fn update_ball_setup_hold(&mut self, delta: f64) {
        let Some(action) = self.ball_setup_action else {
            return;
        };

        self.ball_setup_hold_secs += delta;
        if self.ball_setup_hold_secs < self.ball_setup_next_repeat_secs {
            return;
        }

        let (move_z_delta, rotate_y_delta_deg) = Self::ball_setup_adjustment(action);
        self.send_message(ClientMessage::AdjustBallSetup {
            move_z_delta,
            rotate_y_delta_deg,
        });
        self.ball_setup_next_repeat_secs += CONTROLLER_HOLD_REPEAT_INTERVAL_SECS as f64;
    }

    fn request_ball_respawn(&mut self) {
        if let Some(mut ball) = self.try_ball() {
            ball.queue_free();
        }
        self.pending_ball_respawn = true;
    }

    fn process_pending_ball_respawn(&mut self) {
        if !self.pending_ball_respawn || self.try_ball().is_some() {
            return;
        }

        let Some(mut game_root) = self.base().get_node_or_null("GameManager") else {
            return;
        };

        self.restore_ball_start_point_transform();

        let ball_scene = load::<PackedScene>("res://Assets/bowling_ball.tscn");
        let mut ball = ball_scene.instantiate_as::<Ball>();

        ball.set(
            "track_start",
            &NodePath::from("../BallStartPoint").to_variant(),
        );
        ball.set("track_end", &NodePath::from("../BallEndPoint").to_variant());
        ball.set("name", &StringName::from("BowlingBall").to_variant());

        game_root.add_child(&ball);
        ball.bind_mut().reset_ball();

        if let Some(mut normal_camera) = game_root.get_node_or_null("PhantomCameraNormal") {
            normal_camera.set("follow_target", &ball.to_variant());
            normal_camera.set("look_at_target", &ball.to_variant());
        }

        if let Some(mut zoom_camera) = game_root.get_node_or_null("PhantomCameraZoomedIn") {
            zoom_camera.set("follow_target", &ball.to_variant());
            zoom_camera.set("look_at_target", &ball.to_variant());
        }

        self.pending_ball_respawn = false;
    }

    fn try_game_manager(&self) -> Option<Gd<GameManager>> {
        self.base()
            .get_node_or_null("GameManager")
            .and_then(|node| node.try_cast::<GameManager>().ok())
    }

    fn reset_lane_for_turn(&mut self) {
        if let Some(mut game_manager) = self.try_game_manager() {
            game_manager.bind_mut().spawn_pins();
        }
        self.restore_ball_start_point_transform();
        self.request_ball_respawn();
        self.host_ball_was_launched = false;
        self.host_throw_start_fallen = 0;
        self.host_throw_waiting_report = false;
        self.host_throw_settle_secs = 0.0;
    }

    fn update_controller_strength(&mut self) {
        if !self.controller_holding {
            return;
        }

        if self.controller_motion_history.len() == controller::MOTION_HISTORY_LIMIT {
            self.controller_motion_history.pop_front();
        }
        self.controller_motion_history.push_back(self.accel);
        let (force, direction) = controller::estimate_throw_from_history(
            &self.controller_motion_history,
            self.controller_baseline_accel,
        );
        self.controller_force = force;
        self.controller_direction = direction;
    }

    fn maybe_finish_host_throw(&mut self, delta: f64) {
        if !self.is_host() {
            return;
        }

        let launched = self
            .try_ball()
            .map(|ball| ball.bind().is_launched())
            .unwrap_or(false);

        if launched {
            self.host_ball_was_launched = true;
            self.host_throw_waiting_report = false;
            self.host_throw_settle_secs = 0.0;
            return;
        }

        if self.host_ball_was_launched && !self.host_throw_waiting_report {
            self.host_throw_waiting_report = true;
            self.host_throw_settle_secs = 0.0;
            return;
        }

        if self.host_throw_waiting_report {
            self.host_throw_settle_secs += delta;

            if self.host_throw_settle_secs < THROW_SETTLE_SECS {
                return;
            }

            self.host_ball_was_launched = false;
            self.host_throw_waiting_report = false;
            self.host_throw_settle_secs = 0.0;

            let fallen_count = self.current_fallen_count();
            let knocked = fallen_count.clamp(0, self.scoreboard.pins_remaining as i32) as u8;
            let standing = self.scoreboard.pins_remaining - knocked;

            godot_print!(
                "REPORT throw: fallen={}, knocked={}, remaining={}, standing={}",
                fallen_count,
                knocked,
                self.scoreboard.pins_remaining,
                standing
            );

            self.send_message(ClientMessage::ReportThrowResult {
                knocked_pins: knocked,
                standing_pins: standing,
            });

            self.host_throw_start_fallen = fallen_count;
        }
    }

    fn current_fallen_count(&self) -> i32 {
        self.try_game_manager()
            .map(|game_manager| game_manager.bind().fallen_count())
            .unwrap_or(0)
    }

    fn sync_host_lane_to_scoreboard(&mut self, previous: &ScoreboardState) {
        if !self.is_host() {
            return;
        }
        if self.scoreboard.game_over {
            self.reset_lane_for_turn();
            self.host_throw_start_fallen = 0;
            return;
        }

        let same_player = previous.current_player_id == self.scoreboard.current_player_id;
        let should_keep_rack =
            same_player && self.scoreboard.current_roll > 1 && self.scoreboard.pins_remaining < 10;

        if should_keep_rack {
            if let Some(mut game_manager) = self.try_game_manager() {
                game_manager.bind_mut().clear_fallen_pins();
            }

            self.request_ball_respawn();

            self.host_throw_start_fallen = 0;
        } else {
            self.reset_lane_for_turn();
            self.host_throw_start_fallen = 0;
        }
    }

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
        let result = ws.connect_to_url(&GString::from(url.as_str()));
        if result != Error::OK {
            self.info_text = format!("Could not connect to server at {url}");
            self.screen = Screen::Info;
            self.pending_messages.clear();
            return;
        }
        self.ws = Some(ws);
    }

    fn send_message(&mut self, msg: ClientMessage) {
        self.ensure_socket();
        if let Ok(encoded) = msg.encode() {
            self.pending_messages.push(encoded);
            self.flush_pending_messages();
        }
    }

    fn flush_pending_messages(&mut self) {
        let Some(ws) = self.ws.as_mut() else { return };
        if self.pending_messages.is_empty() {
            return;
        }

        if ws.get_ready_state() != WebSocketState::OPEN {
            return;
        }

        let mut sent_count = 0usize;
        for message in &self.pending_messages {
            if ws.send_text(&GString::from(message.as_str())) == Error::OK {
                sent_count += 1;
            } else {
                break;
            }
        }

        if sent_count > 0 {
            self.pending_messages.drain(0..sent_count);
            if self.pending_messages.is_empty() {
                self.pending_connect_secs = 0.0;
            }
        }
    }

    fn handle_socket_connect_timeout(&mut self, delta: f64) {
        if self.pending_messages.is_empty() {
            self.pending_connect_secs = 0.0;
            return;
        }

        let Some(ws) = self.ws.as_ref() else {
            self.pending_connect_secs += delta;
            if self.pending_connect_secs >= SOCKET_CONNECT_TIMEOUT_SECS {
                let url = self.ws_url();
                self.info_text = format!("Could not connect to server at {url}");
                self.screen = Screen::Info;
                self.pending_messages.clear();
                self.pending_connect_secs = 0.0;
            }
            return;
        };

        if ws.get_ready_state() == WebSocketState::OPEN {
            self.pending_connect_secs = 0.0;
            return;
        }

        self.pending_connect_secs += delta;
        if self.pending_connect_secs < SOCKET_CONNECT_TIMEOUT_SECS {
            return;
        }

        let url = self.ws_url();
        let mut message = format!("Could not connect to server at {url}");
        if ws.get_ready_state() == WebSocketState::CLOSED {
            let code = ws.get_close_code();
            let reason = ws.get_close_reason().to_string();
            if code != -1 {
                if reason.is_empty() {
                    message = format!("WebSocket closed (code {code}) while connecting to {url}");
                } else {
                    message = format!(
                        "WebSocket closed (code {code}: {reason}) while connecting to {url}"
                    );
                }
            }
        }

        self.info_text = message;
        self.screen = Screen::Info;
        self.pending_messages.clear();
        self.pending_connect_secs = 0.0;
        self.ws = None;
    }

    fn poll_socket(&mut self) {
        let mut should_reset_socket = false;
        {
            let Some(ws) = self.ws.as_mut() else { return };
            ws.poll();
            if ws.get_ready_state() == WebSocketState::CLOSED {
                should_reset_socket = true;
            }
        }

        if should_reset_socket {
            self.ws = None;
            if !self.pending_messages.is_empty() {
                self.ensure_socket();
            }
        }

        self.flush_pending_messages();

        let mut messages = Vec::new();
        if let Some(ws) = self.ws.as_mut() {
            while ws.get_available_packet_count() > 0 {
                let packet = ws.get_packet();
                let text = String::from_utf8(packet.to_vec()).unwrap_or_default();
                if !text.is_empty() {
                    messages.push(text);
                }
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
                self.session_role = Some(SessionRole::Host);
                self.lobby_code = code.clone();
                self.persist_role_session(SessionRole::Host, &code, &host_session);
                rendering::update_lobby_ui(self, &code, &players);
                self.screen = Screen::Host;
            }
            ServerMessage::LobbyJoined {
                code,
                player_id,
                player_session,
                players,
            } => {
                self.session_role = Some(SessionRole::Player);
                self.player_id = player_id;
                self.lobby_code = code.clone();
                self.persist_role_session(SessionRole::Player, &code, &player_session);
                rendering::update_lobby_ui(self, &code, &players);
                self.info_text = format!("Joined lobby {code}. Waiting for host to start...");
                self.screen = Screen::Info;
            }
            ServerMessage::ReconnectOkHost { code, players } => {
                self.session_role = Some(SessionRole::Host);
                self.lobby_code = code.clone();
                rendering::update_lobby_ui(self, &code, &players);
                self.screen = Screen::Host;
            }
            ServerMessage::ReconnectOkPlayer {
                code,
                player_id,
                player_session,
                players,
            } => {
                self.session_role = Some(SessionRole::Player);
                self.player_id = player_id;
                self.lobby_code = code.clone();
                self.persist_role_session(SessionRole::Player, &code, &player_session);
                rendering::update_lobby_ui(self, &code, &players);
                self.screen = self.gameplay_screen();
            }
            ServerMessage::LobbyUpdated { code, players } => {
                rendering::update_lobby_ui(self, &code, &players);
            }
            ServerMessage::GameStarted {
                code,
                current_player_id,
            } => {
                self.lobby_code = code;
                self.current_player_id = current_player_id;
                self.stop_ball_setup_hold();
                self.set_zoomed_in(false);
                self.restore_ball_start_point_transform();
                self.reset_lane_for_turn();
                self.screen = self.gameplay_screen();
            }
            ServerMessage::TurnChanged { current_player_id } => {
                self.current_player_id = current_player_id;
                self.controller_holding = false;
                self.controller_force = 0.0;
                self.controller_direction = Vector2::new(0.0, 1.0);
                self.controller_baseline_accel = Vector3::ZERO;
                self.controller_motion_history.clear();
                self.stop_ball_setup_hold();
                self.set_zoomed_in(false);
                self.screen = self.gameplay_screen();
            }
            ServerMessage::ThrowEvent {
                player_id: _,
                force,
                direction_x,
                direction_z,
            } => {
                self.controller_holding = false;
                self.controller_force = 0.0;
                self.controller_direction = Vector2::new(0.0, 1.0);
                self.controller_baseline_accel = Vector3::ZERO;
                self.controller_motion_history.clear();
                self.stop_ball_setup_hold();
                if self.is_host()
                    && let Some(mut ball) = self.try_ball()
                {
                    ball.bind_mut().reset_ball();
                    ball.bind_mut()
                        .launch_throw(force, direction_x, direction_z);
                    self.restore_ball_start_point_transform();
                    self.host_ball_was_launched = true;
                }
            }
            ServerMessage::AdjustBallSetup {
                player_id,
                move_z_delta,
                rotate_y_delta_deg,
            } => {
                if self.is_host() {
                    self.apply_ball_setup_adjustment(move_z_delta, rotate_y_delta_deg);
                    self.push_ball_setup_log(&player_id, move_z_delta, rotate_y_delta_deg);
                }
            }
            ServerMessage::ToggleZoom { player_id, zoomed_in } => {
                if self.is_host() {
                    self.set_zoomed_in(zoomed_in);
                    godot_print!("zoom toggle from {}: {}", player_id, zoomed_in);
                }
            }
            ServerMessage::ScoreboardUpdated { scoreboard } => {
                let previous_scoreboard = self.scoreboard.clone();
                self.current_player_id = scoreboard.current_player_id.clone();
                self.scoreboard = scoreboard;
                self.sync_host_lane_to_scoreboard(&previous_scoreboard);
                self.screen = self.gameplay_screen();
            }
            ServerMessage::Info { message } => {
                self.info_text = message;
                self.screen = Screen::Info;
            }
            ServerMessage::GameStopped { code } => {
                self.info_text = format!("Game stopped in lobby {code}");
                self.scoreboard = ScoreboardState::default();
                self.current_player_id.clear();
                self.host_throw_start_fallen = 0;
                self.stop_ball_setup_hold();
                self.set_zoomed_in(false);
                self.screen = if self.is_host() {
                    Screen::Host
                } else {
                    Screen::Info
                };
            }
            ServerMessage::Error { message } => {
                godot_error!("SERVER ERROR: {}", message);
                self.info_text = format!("Error: {message}");

                self.controller_holding = false;
                self.controller_force = 0.0;
                self.controller_direction = Vector2::new(0.0, 1.0);
                self.controller_baseline_accel = Vector3::ZERO;
                self.controller_motion_history.clear();
                self.stop_ball_setup_hold();
                self.set_zoomed_in(false);

                self.host_ball_was_launched = false;
                self.host_throw_waiting_report = false;
                self.host_throw_settle_secs = 0.0;

                if message.contains("kicked")
                    || message.contains("lobby closed")
                    || message.contains("reconnect failed")
                    || message.contains("lobby not found")
                {
                    self.clear_session_keys();
                    self.session_role = None;
                    self.player_id.clear();
                    self.current_player_id.clear();
                    self.scoreboard = ScoreboardState::default();
                    self.screen = Screen::MainMenu;
                } else {
                    self.screen = self.gameplay_screen();
                }
            }
        }
    }

    fn ensure_username(&self) {
        let username = storage::get_value(self, storage::STORAGE_USERNAME);
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
        storage::set_value(
            self,
            storage::STORAGE_USERNAME,
            &format!("Player_{rand_part}"),
        );
    }

    fn sync_username_input(&mut self) {
        let username = storage::get_value(self, storage::STORAGE_USERNAME);
        if username.trim().is_empty() {
            return;
        }
        let mut username_input = self
            .base_mut()
            .get_node_as::<LineEdit>("UiManager/Mobile/VBoxContainer/Username");
        if username_input.get_text().to_string().trim().is_empty() {
            username_input.set_text(&GString::from(username.as_str()));
        }
    }

    fn load_calibration(&mut self) {
        let raw = storage::get_value(self, storage::STORAGE_CALIBRATION_X);
        self.calibration_offset_x = raw.parse::<f32>().unwrap_or(0.0);
    }

    fn save_calibration(&self) {
        storage::set_value(
            self,
            storage::STORAGE_CALIBRATION_X,
            &format!("{}", self.calibration_offset_x),
        );
    }

    fn try_auto_reconnect(&mut self) {
        let role = storage::get_value(self, storage::STORAGE_ROLE);
        let token = storage::get_value(self, storage::STORAGE_TOKEN);
        let code = storage::get_value(self, storage::STORAGE_CODE);
        if role.is_empty() || token.is_empty() || code.is_empty() {
            return;
        }

        self.lobby_code = code.clone();
        self.screen = Screen::Info;
        if role == "host" {
            self.session_role = Some(SessionRole::Host);
            self.info_text = "Reconnecting as host...".to_string();
            self.send_message(ClientMessage::ReconnectHost {
                code,
                host_session: token,
            });
        } else if role == "player" {
            self.session_role = Some(SessionRole::Player);
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
        storage::set_value(self, storage::STORAGE_ROLE, role_str);
        storage::set_value(self, storage::STORAGE_TOKEN, session);
        storage::set_value(self, storage::STORAGE_CODE, code);
    }

    fn clear_session_keys(&self) {
        storage::clear_value(self, storage::STORAGE_ROLE);
        storage::clear_value(self, storage::STORAGE_TOKEN);
        storage::clear_value(self, storage::STORAGE_CODE);
    }

    fn get_join_username(&mut self) -> String {
        let mut username_input = self
            .base_mut()
            .get_node_as::<LineEdit>("UiManager/Mobile/VBoxContainer/Username");
        let typed = username_input.get_text().to_string().trim().to_string();
        if !typed.is_empty() {
            storage::set_value(self, storage::STORAGE_USERNAME, &typed);
            return typed;
        }
        let fallback = storage::get_value(self, storage::STORAGE_USERNAME);
        username_input.set_text(&GString::from(fallback.as_str()));
        fallback
    }

    fn update_leave_hold(&mut self, delta: f64) {
        if !self.leave_holding {
            return;
        }

        self.leave_hold_secs += delta;

        if self.leave_hold_secs >= LEAVE_HOLD_SECS {
            self.leave_holding = false;
            self.leave_hold_secs = 0.0;
            self.leave_game();
        }
    }

    fn leave_game(&mut self) {
        self.send_message(ClientMessage::Leave);
        self.clear_session_keys();
        self.session_role = None;
        self.player_id.clear();
        self.lobby_code.clear();
        self.players.clear();
        self.current_player_id.clear();
        self.scoreboard = ScoreboardState::default();
        self.controller_holding = false;
        self.controller_force = 0.0;
        self.controller_direction = Vector2::new(0.0, 1.0);
        self.controller_baseline_accel = Vector3::ZERO;
        self.controller_motion_history.clear();
        self.calibration_active = false;
        self.calibration_samples.clear();
        self.host_throw_start_fallen = 0;
        self.host_throw_waiting_report = false;
        self.host_throw_settle_secs = 0.0;
        self.sync_username_input();
        self.screen = Screen::MainMenu;
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
    fn on_leave_button_down(&mut self) {
        self.stop_ball_setup_hold();
        self.leave_holding = true;
        self.leave_hold_secs = 0.0;
        self.info_text = "Hold for 5 seconds to leave...".to_string();
    }

    #[func]
    fn on_leave_button_up(&mut self) {
        self.leave_holding = false;
        self.leave_hold_secs = 0.0;
    }

    #[func]
    fn on_start_pressed(&mut self) {
        self.send_message(ClientMessage::StartGame);
        self.info_text = "Starting game...".to_string();
        self.screen = Screen::Info;
    }

    #[func]
    fn on_stop_game_pressed(&mut self) {
        if !self.is_host() {
            return;
        }
        self.stop_ball_setup_hold();
        self.send_message(ClientMessage::StopGame);
        self.info_text = "Stopping game...".to_string();
        self.screen = Screen::Info;
    }

    #[func]
    fn on_kick_pressed(&mut self) {
        if !self.is_host() {
            return;
        }

        let mut host_input = self.base_mut().get_node_as::<LineEdit>(
            "UiManager/CenterContainer/VBoxContainer/DesktopHost/PanelContainer/MarginContainer/VBoxContainer/KickRow/KickPlayerRef",
        );
        let mut game_input = self.base_mut().get_node_as::<LineEdit>(
            "UiManager/GameHud/MarginContainer/VBoxContainer/HostKickRow/KickPlayerRef",
        );

        let mut player_ref = game_input.get_text().to_string().trim().to_string();
        if player_ref.is_empty() {
            player_ref = host_input.get_text().to_string().trim().to_string();
        }
        if player_ref.is_empty() {
            self.info_text = "Enter a player name or id to kick".to_string();
            self.screen = Screen::Info;
            return;
        }

        self.send_message(ClientMessage::KickPlayer {
            player_ref: player_ref.clone(),
        });
        host_input.clear();
        game_input.clear();
    }

    #[func]
    fn on_hold_button_down(&mut self) {
        if self.zoomed_in {
            return;
        }
        self.stop_ball_setup_hold();
        if !(self.calibration_active || self.is_local_player_turn()) {
            return;
        }

        self.controller_holding = true;
        self.controller_force = 0.0;
        self.controller_direction = Vector2::new(0.0, 1.0);
        self.controller_motion_history.clear();
        self.controller_motion_history.push_back(self.accel);
    }

    #[func]
    fn on_hold_button_up(&mut self) {
        if !self.controller_holding {
            return;
        }

        self.controller_holding = false;
        let force = self.controller_force.clamp(0.0, 1.0);
        let direction = self.controller_direction;
        self.controller_force = 0.0;
        self.controller_direction = Vector2::new(0.0, 1.0);
        self.controller_baseline_accel = Vector3::ZERO;
        self.controller_motion_history.clear();
        if self.calibration_active {
            self.calibration_samples.push(direction.x);
            if self.calibration_samples.len() >= controller::CALIBRATION_THROW_COUNT {
                let sum: f32 = self.calibration_samples.iter().copied().sum();
                self.calibration_offset_x = sum / self.calibration_samples.len() as f32;
                self.save_calibration();
                self.calibration_active = false;
                self.calibration_samples.clear();
                self.screen = Screen::MainMenu;
            }
            return;
        }
        if self.is_local_player_turn() {
            let corrected_x = direction.x - self.calibration_offset_x;
            let corrected_x =
                controller::soft_deadzone(corrected_x, controller::BOWLING_SIDEWAYS_DEADZONE);

            let corrected = Vector2::new(corrected_x, direction.y).normalized();

            self.send_message(ClientMessage::ThrowEvent {
                force,
                direction_x: corrected.x,
                direction_z: corrected.y,
            });
        }

        self.stop_ball_setup_hold();
    }

    #[func]
    fn on_zoom_pressed(&mut self) {
        if !(self.calibration_active || self.is_local_player_turn()) {
            return;
        }

        self.set_zoomed_in(!self.zoomed_in);
        self.send_message(ClientMessage::ToggleZoom {
            zoomed_in: self.zoomed_in,
        });
    }

    #[func]
    fn on_move_left_button_down(&mut self) {
        self.start_ball_setup_hold(BallSetupAction::MoveLeft);
    }

    #[func]
    fn on_move_right_button_down(&mut self) {
        self.start_ball_setup_hold(BallSetupAction::MoveRight);
    }

    #[func]
    fn on_move_button_up(&mut self) {
        self.stop_ball_setup_hold();
    }

    #[func]
    fn on_rotate_left_button_down(&mut self) {
        self.start_ball_setup_hold(BallSetupAction::RotateLeft);
    }

    #[func]
    fn on_rotate_right_button_down(&mut self) {
        self.start_ball_setup_hold(BallSetupAction::RotateRight);
    }

    #[func]
    fn on_rotate_button_up(&mut self) {
        self.stop_ball_setup_hold();
    }

    #[func]
    fn on_calibrate_pressed(&mut self) {
        if !self.current_player_id.is_empty() {
            return;
        }

        self.stop_ball_setup_hold();
        self.calibration_active = true;
        self.calibration_samples.clear();
        self.controller_holding = false;
        self.controller_force = 0.0;
        self.controller_direction = Vector2::new(0.0, 1.0);
        self.controller_motion_history.clear();
        self.screen = Screen::Controller;
    }
}
