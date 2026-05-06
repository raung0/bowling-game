use common::{ClientMessage, PlayerInfo, PlayerScoreView, ScoreboardState, ServerMessage};
use getset::Getters;
use godot::{
    classes::web_socket_peer::State as WebSocketState,
    classes::{
        Button, HBoxContainer, Label, LineEdit, Node, ProgressBar, VBoxContainer, WebSocketPeer,
    },
    global::{Error, HorizontalAlignment},
    prelude::*,
};
use rand::Rng;
use std::{collections::VecDeque, str::FromStr};

use crate::ball::Ball;
use crate::game_manager::GameManager;
use crate::ui_manager::UiManager;

const STORAGE_ROLE: &str = "session_role";
const STORAGE_TOKEN: &str = "session_token";
const STORAGE_CODE: &str = "lobby_code";
const STORAGE_USERNAME: &str = "username";
const STORAGE_CALIBRATION_X: &str = "calibration_x";
const SOCKET_CONNECT_TIMEOUT_SECS: f64 = 5.0;
const MOTION_HISTORY_LIMIT: usize = 12;
const MOTION_BASELINE_SAMPLES: usize = 4;
const MOTION_PEAK_WINDOW_SAMPLES: usize = 3;
const BOWLING_SWING_MIN_FORCE: f32 = 1.6;
const BOWLING_SWING_MAX_ANGLE_DEG: f32 = 45.0;
const BOWLING_SIDEWAYS_DEADZONE: f32 = 0.08;
const CALIBRATION_THROW_COUNT: usize = 3;
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
            controller_motion_history: VecDeque::with_capacity(MOTION_HISTORY_LIMIT),
            calibration_active: false,
            calibration_samples: Vec::new(),
            calibration_offset_x: 0.0,
            host_ball_was_launched: false,
            host_throw_start_fallen: 0,
            host_throw_waiting_report: false,
            host_throw_settle_secs: 0.0,
            leave_holding: false,
            leave_hold_secs: 0.0,
        }
    }

    fn ready(&mut self) {
        self.base_mut().set_process(true);

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
        let mut back_button = self.base().get_node_as::<Button>("UiManager/CenterContainer/VBoxContainer/DesktopHost/PanelContainer/MarginContainer/VBoxContainer/Actions/Back");
        let mut start_button = self.base().get_node_as::<Button>("UiManager/CenterContainer/VBoxContainer/DesktopHost/PanelContainer/MarginContainer/VBoxContainer/Actions/Start");
        let mut host_stop_button = self.base().get_node_as::<Button>("UiManager/CenterContainer/VBoxContainer/DesktopHost/PanelContainer/MarginContainer/VBoxContainer/Actions/Stop");
        let mut host_kick_button = self.base().get_node_as::<Button>("UiManager/CenterContainer/VBoxContainer/DesktopHost/PanelContainer/MarginContainer/VBoxContainer/KickRow/KickButton");
        let mut hold_button = self
            .base()
            .get_node_as::<Button>("UiManager/Controller/MarginContainer/VBoxContainer/HoldButton");
        let mut controller_back_button = self
            .base()
            .get_node_as::<Button>("UiManager/Controller/MarginContainer/VBoxContainer/BackToMenu");
        let mut game_kick_button = self.base().get_node_as::<Button>(
            "UiManager/GameHud/MarginContainer/VBoxContainer/HostKickRow/KickButton",
        );
        let mut game_stop_button = self.base().get_node_as::<Button>(
            "UiManager/GameHud/MarginContainer/VBoxContainer/StopGameButton",
        );

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
        controller_back_button
            .connect("button_down", &self.base().callable("on_leave_button_down"));
        controller_back_button.connect("button_up", &self.base().callable("on_leave_button_up"));
        game_kick_button.connect("pressed", &self.base().callable("on_kick_pressed"));
        game_stop_button.connect("pressed", &self.base().callable("on_stop_game_pressed"));

        self.ensure_username();
        self.load_calibration();
        self.sync_username_input();
        self.try_auto_reconnect();
    }

    fn process(&mut self, delta: f64) {
        self.accel = self.browser_accel();
        self.is_mobile = self.is_mobile_web();

        self.poll_socket();
        self.handle_socket_connect_timeout(delta);
        self.update_controller_strength();
        self.update_leave_hold(delta);
        self.maybe_finish_host_throw(delta);

        let mut ui_manager = self.base_mut().get_node_as::<UiManager>("UiManager");
        ui_manager
            .bind_mut()
            .set_screen(self.screen, self.is_mobile);

        self.render_mobile_accel();
        self.render_calibration_status();
        self.render_controller_ui();
        self.render_game_ui();
        self.render_info_text();
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

    fn try_ball(&self) -> Option<Gd<Ball>> {
        self.base()
            .get_node_or_null("GameManager/BowlingBall")
            .and_then(|node| node.try_cast::<Ball>().ok())
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
        if let Some(mut ball) = self.try_ball() {
            ball.bind_mut().reset_ball();
        }
        self.host_ball_was_launched = false;
        self.host_throw_start_fallen = 0;
        self.host_throw_waiting_report = false;
        self.host_throw_settle_secs = 0.0;
    }

    fn render_mobile_accel(&mut self) {
        let mut accel_label = self
            .base_mut()
            .get_node_as::<Label>("UiManager/Mobile/VBoxContainer/Accel");
        accel_label.set_text(&format!(
            "Accel: x={:.2} y={:.2} z={:.2}",
            self.accel.x, self.accel.y, self.accel.z
        ));
    }

    fn render_calibration_status(&mut self) {
        let mut calibration_label = self
            .base_mut()
            .get_node_as::<Label>("UiManager/Mobile/VBoxContainer/CalibrationStatus");
        let text = if self.calibration_active {
            format!(
                "Calibration in progress: {}/{}",
                self.calibration_samples.len(),
                CALIBRATION_THROW_COUNT
            )
        } else {
            format!("Calibration offset: {:+.3}", self.calibration_offset_x)
        };
        calibration_label.set_text(&GString::from(text.as_str()));
    }

    fn render_controller_ui(&mut self) {
        let status = if self.calibration_active {
            format!(
                "Calibration throw {}/{}: hold, throw straight, release",
                self.calibration_samples.len() + 1,
                CALIBRATION_THROW_COUNT
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
        };

        let mut status_label = self
            .base_mut()
            .get_node_as::<Label>("UiManager/Controller/MarginContainer/VBoxContainer/Status");
        status_label.set_text(&GString::from(status.as_str()));

        let mut strength_label = self.base_mut().get_node_as::<Label>(
            "UiManager/Controller/MarginContainer/VBoxContainer/StrengthLabel",
        );
        strength_label.set_text(&format!(
            "Force: {:.0}%  Dir: ({:.2}, {:.2})",
            self.controller_force * 100.0,
            self.controller_direction.x,
            self.controller_direction.y,
        ));

        let mut strength_bar = self.base_mut().get_node_as::<ProgressBar>(
            "UiManager/Controller/MarginContainer/VBoxContainer/StrengthBar",
        );
        strength_bar.set_value((self.controller_force * 100.0) as f64);

        let mut hold_button = self
            .base_mut()
            .get_node_as::<Button>("UiManager/Controller/MarginContainer/VBoxContainer/HoldButton");
        hold_button.set_disabled(!(self.calibration_active || self.is_local_player_turn()));
    }

    fn render_game_ui(&mut self) {
        let current_player = if self.current_player_id.is_empty() {
            "Current turn: waiting for player".to_string()
        } else {
            format!("Current turn: {}", self.current_player_name())
        };

        let mut turn_label = self
            .base_mut()
            .get_node_as::<Label>("UiManager/GameHud/MarginContainer/VBoxContainer/TurnLabel");
        turn_label.set_text(&GString::from(current_player.as_str()));

        let lobby_line = if self.lobby_code.is_empty() {
            "Lobby: -".to_string()
        } else {
            format!("Lobby: {}", self.lobby_code)
        };
        let mut lobby_label = self
            .base_mut()
            .get_node_as::<Label>("UiManager/GameHud/MarginContainer/VBoxContainer/LobbyLabel");
        lobby_label.set_text(&GString::from(lobby_line.as_str()));

        let mut host_kick_row = self.base_mut().get_node_as::<HBoxContainer>(
            "UiManager/GameHud/MarginContainer/VBoxContainer/HostKickRow",
        );
        host_kick_row.set_visible(self.is_host());

        let mut stop_game_button = self.base_mut().get_node_as::<Button>(
            "UiManager/GameHud/MarginContainer/VBoxContainer/StopGameButton",
        );
        stop_game_button.set_visible(self.is_host());

        self.render_scoreboard();
    }

    fn render_scoreboard(&mut self) {
        let players = self.scoreboard.players.clone();
        let current = players.first().cloned();
        let next_players = players.iter().skip(1).take(3).cloned().collect::<Vec<_>>();
        let remaining = players.iter().skip(4).cloned().collect::<Vec<_>>();

        let mut current_root = self.base_mut().get_node_as::<VBoxContainer>(
            "UiManager/GameHud/ScoreboardPanel/MarginContainer/VBoxContainer/CurrentTable",
        );
        Self::clear_container(&mut current_root);
        if let Some(current) = current.as_ref() {
            self.add_scorecard(&mut current_root, current, true);
        }

        let mut next_root = self.base_mut().get_node_as::<VBoxContainer>(
            "UiManager/GameHud/ScoreboardPanel/MarginContainer/VBoxContainer/NextTables",
        );
        Self::clear_container(&mut next_root);
        for player in &next_players {
            self.add_scorecard(&mut next_root, player, false);
        }

        let mut remaining_header = self.base_mut().get_node_as::<Label>(
            "UiManager/GameHud/ScoreboardPanel/MarginContainer/VBoxContainer/RemainingHeader",
        );
        remaining_header.set_visible(!remaining.is_empty());

        let mut remaining_root = self.base_mut().get_node_as::<VBoxContainer>(
            "UiManager/GameHud/ScoreboardPanel/MarginContainer/VBoxContainer/RemainingPlayers",
        );
        Self::clear_container(&mut remaining_root);
        for player in &remaining {
            let mut row = HBoxContainer::new_alloc();

            let mut name = Label::new_alloc();
            name.set_text(&GString::from(player.username.as_str()));
            row.add_child(&name);

            let mut status = Label::new_alloc();
            status.set_text(&GString::from(player.status_label.as_str()));
            row.add_child(&status);

            let mut total = Label::new_alloc();
            total.set_text(&GString::from(format!("{}", player.total_score).as_str()));
            row.add_child(&total);

            remaining_root.add_child(&row);
        }
    }

    fn add_scorecard(
        &self,
        root: &mut Gd<VBoxContainer>,
        player: &PlayerScoreView,
        featured: bool,
    ) {
        let mut wrapper = VBoxContainer::new_alloc();
        wrapper.add_theme_constant_override("separation", if featured { 8 } else { 6 });

        let mut header = HBoxContainer::new_alloc();
        let mut name = Label::new_alloc();
        name.set_text(&GString::from(player.username.as_str()));
        name.add_theme_font_size_override("font_size", if featured { 28 } else { 22 });
        header.add_child(&name);

        let mut status = Label::new_alloc();
        status.set_text(&GString::from(player.status_label.as_str()));
        status.add_theme_font_size_override("font_size", if featured { 20 } else { 16 });
        header.add_child(&status);

        let mut total = Label::new_alloc();
        total.set_text(&GString::from(format!("{}", player.total_score).as_str()));
        total.add_theme_font_size_override("font_size", if featured { 24 } else { 18 });
        header.add_child(&total);
        wrapper.add_child(&header);

        let mut frames = HBoxContainer::new_alloc();
        frames.add_theme_constant_override("separation", if featured { 6 } else { 4 });
        for (idx, frame) in player.frames.iter().enumerate() {
            let mut frame_box = VBoxContainer::new_alloc();
            frame_box
                .set_custom_minimum_size(Vector2::new(if featured { 48.0 } else { 38.0 }, 0.0));
            frame_box.add_theme_constant_override("separation", 2);

            let mut frame_label = Label::new_alloc();
            frame_label.set_text(&GString::from(format!("{}", idx + 1).as_str()));
            frame_label.set_horizontal_alignment(HorizontalAlignment::CENTER);
            frame_label.add_theme_font_size_override("font_size", if featured { 14 } else { 12 });
            frame_box.add_child(&frame_label);

            let mut rolls_label = Label::new_alloc();
            let rolls_text = if frame.rolls.is_empty() {
                String::from(" ")
            } else {
                frame.rolls.join(" ")
            };
            rolls_label.set_text(&GString::from(rolls_text.as_str()));
            rolls_label.set_horizontal_alignment(HorizontalAlignment::CENTER);
            rolls_label.add_theme_font_size_override("font_size", if featured { 16 } else { 13 });
            frame_box.add_child(&rolls_label);

            let mut score_label = Label::new_alloc();
            let score_text = frame
                .cumulative_score
                .map(|score| score.to_string())
                .unwrap_or_default();
            score_label.set_text(&GString::from(score_text.as_str()));
            score_label.set_horizontal_alignment(HorizontalAlignment::CENTER);
            score_label.add_theme_font_size_override("font_size", if featured { 16 } else { 13 });
            frame_box.add_child(&score_label);

            frames.add_child(&frame_box);
        }
        wrapper.add_child(&frames);
        root.add_child(&wrapper);
    }

    fn clear_container<T>(container: &mut Gd<T>)
    where
        T: Inherits<Node>,
    {
        let mut node = container.clone().upcast::<Node>();
        let children = node.get_children();
        for mut child in children.iter_shared() {
            node.remove_child(&child);
            child.queue_free();
        }
    }

    fn update_controller_strength(&mut self) {
        if !self.controller_holding {
            return;
        }

        if self.controller_motion_history.len() == MOTION_HISTORY_LIMIT {
            self.controller_motion_history.pop_front();
        }
        self.controller_motion_history.push_back(self.accel);
        let (force, direction) = self.estimate_throw_from_history();
        self.controller_force = force;
        self.controller_direction = direction;
    }

    fn estimate_throw_from_history(&self) -> (f32, Vector2) {
        if self.controller_motion_history.is_empty() {
            return (0.0, Vector2::new(0.0, 1.0));
        }

        let baseline_sample_count = self
            .controller_motion_history
            .len()
            .clamp(1, MOTION_BASELINE_SAMPLES);
        let baseline_divisor = baseline_sample_count as f32 + 1.0;
        let baseline = self
            .controller_motion_history
            .iter()
            .take(baseline_sample_count)
            .copied()
            .fold(self.controller_baseline_accel, |acc, sample| acc + sample)
            / baseline_divisor;

        let mut relative_samples = Vec::with_capacity(self.controller_motion_history.len());
        let mut last = None;
        for sample in &self.controller_motion_history {
            let smoothed = if let Some(prev) = last {
                (*sample + prev) * 0.5
            } else {
                *sample
            };
            last = Some(*sample);
            relative_samples.push(smoothed - baseline);
        }

        let mut best_forward = 0.0_f32;
        let mut best_index = 0usize;
        for (idx, relative) in relative_samples.iter().enumerate() {
            let forward = (-relative.y).max(0.0);
            if forward > best_forward {
                best_forward = forward;
                best_index = idx;
            }
        }

        if best_forward <= 0.0 {
            return (0.0, Vector2::new(0.0, 1.0));
        }

        let window_radius = MOTION_PEAK_WINDOW_SAMPLES / 2;
        let window_start = best_index.saturating_sub(window_radius);
        let window_end = (best_index + window_radius + 1).min(relative_samples.len());
        let mut sideways_total = 0.0_f32;
        let mut sideways_count = 0usize;
        for relative in &relative_samples[window_start..window_end] {
            sideways_total += relative.z;
            sideways_count += 1;
        }
        let mut best_sideways = if sideways_count > 0 {
            sideways_total / sideways_count as f32
        } else {
            0.0
        };
        if best_sideways.abs() < BOWLING_SIDEWAYS_DEADZONE {
            best_sideways = 0.0;
        }

        let max_sideways = best_forward * BOWLING_SWING_MAX_ANGLE_DEG.to_radians().tan();
        let clamped_sideways = best_sideways.clamp(-max_sideways, max_sideways);
        let direction = Vector2::new(clamped_sideways, best_forward).normalized();
        let force = ((best_forward - BOWLING_SWING_MIN_FORCE) / 8.0).clamp(0.0, 1.0);

        (force, direction)
    }

    fn maybe_finish_host_throw(&mut self, delta: f64) {
        if !self.is_host() {
            return;
        }

        let Some(ball) = self.try_ball() else { return };
        let launched = ball.bind().is_launched();

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

            if let Some(mut ball) = self.try_ball() {
                ball.bind_mut().reset_ball();
            }

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
                self.update_lobby_ui(&code, &players);
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
                self.update_lobby_ui(&code, &players);
                self.info_text = format!("Joined lobby {code}. Waiting for host to start...");
                self.screen = Screen::Info;
            }
            ServerMessage::ReconnectOkHost { code, players } => {
                self.session_role = Some(SessionRole::Host);
                self.lobby_code = code.clone();
                self.update_lobby_ui(&code, &players);
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
                self.update_lobby_ui(&code, &players);
                self.screen = self.gameplay_screen();
            }
            ServerMessage::LobbyUpdated { code, players } => {
                self.update_lobby_ui(&code, &players);
            }
            ServerMessage::GameStarted {
                code,
                current_player_id,
            } => {
                self.lobby_code = code;
                self.current_player_id = current_player_id;
                self.screen = self.gameplay_screen();
            }
            ServerMessage::TurnChanged { current_player_id } => {
                self.current_player_id = current_player_id;
                self.controller_holding = false;
                self.controller_force = 0.0;
                self.controller_direction = Vector2::new(0.0, 1.0);
                self.controller_baseline_accel = Vector3::ZERO;
                self.controller_motion_history.clear();
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
                if self.is_host()
                    && let Some(mut ball) = self.try_ball()
                {
                    ball.bind_mut()
                        .launch_throw(force, direction_x, direction_z);
                    self.host_ball_was_launched = true;
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

    fn update_lobby_ui(&mut self, code: &str, players: &[PlayerInfo]) {
        self.lobby_code = code.to_string();
        self.players = players.to_vec();
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

    fn sync_username_input(&mut self) {
        let username = self.get_storage(STORAGE_USERNAME);
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
        let raw = self.get_storage(STORAGE_CALIBRATION_X);
        self.calibration_offset_x = raw.parse::<f32>().unwrap_or(0.0);
    }

    fn save_calibration(&self) {
        self.set_storage(
            STORAGE_CALIBRATION_X,
            &format!("{}", self.calibration_offset_x),
        );
    }

    fn try_auto_reconnect(&mut self) {
        let role = self.get_storage(STORAGE_ROLE);
        let token = self.get_storage(STORAGE_TOKEN);
        let code = self.get_storage(STORAGE_CODE);
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
        if !(self.calibration_active || self.is_local_player_turn()) {
            return;
        }

        self.controller_holding = true;
        self.controller_force = 0.0;
        self.controller_direction = Vector2::new(0.0, 1.0);
        //self.controller_baseline_accel = self.accel;
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
            if self.calibration_samples.len() >= CALIBRATION_THROW_COUNT {
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
            let corrected_x = soft_deadzone(corrected_x, BOWLING_SIDEWAYS_DEADZONE);

            let corrected = Vector2::new(corrected_x, direction.y).normalized();

            self.send_message(ClientMessage::ThrowEvent {
                force,
                direction_x: corrected.x,
                direction_z: corrected.y,
            });
        }
    }

    #[func]
    fn on_calibrate_pressed(&mut self) {
        if !self.current_player_id.is_empty() {
            return;
        }

        self.calibration_active = true;
        self.calibration_samples.clear();
        self.controller_holding = false;
        self.controller_force = 0.0;
        self.controller_direction = Vector2::new(0.0, 1.0);
        //self.controller_baseline_accel = Vector3::ZERO;
        self.controller_motion_history.clear();
        self.screen = Screen::Controller;
    }
}

fn soft_deadzone(value: f32, deadzone: f32) -> f32 {
    let abs = value.abs();

    if abs <= deadzone {
        return 0.0;
    }

    value.signum() * ((abs - deadzone) / (1.0 - deadzone)).clamp(0.0, 1.0)
}
