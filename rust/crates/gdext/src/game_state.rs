use common::{
    BALL_MOVE_Z_MAX, BALL_MOVE_Z_MIN, BALL_MOVE_Z_STEP, BALL_ROT_Y_MAX_DEG, BALL_ROT_Y_STEP_DEG,
    CONTROLLER_HOLD_REPEAT_DELAY_SECS, CONTROLLER_HOLD_REPEAT_INTERVAL_SECS, ClientMessage,
    PlayerInfo, ScoreboardState, ServerMessage,
};
use getset::Getters;
use godot::{
    classes::web_socket_peer::State as WebSocketState,
    classes::{
        AudioStream, AudioStreamPlayer, Button, HSlider, LineEdit, MeshInstance3D, Node, Node3D,
        PackedScene, RigidBody3D, WebSocketPeer,
    },
    global::Error,
    prelude::*,
};
use rand::{seq::SliceRandom, Rng};
use std::{collections::VecDeque, str::FromStr};

use crate::ball::Ball;
use crate::game_manager::GameManager;
use crate::ui_manager::UiManager;
use crate::{controller, rendering, storage};

const SOCKET_CONNECT_TIMEOUT_SECS: f64 = 5.0;
const THROW_SETTLE_SECS: f64 = 3.0;
const LEAVE_HOLD_SECS: f64 = 5.0;
const DIRECTION_INDICATOR_FADE_SECS: f64 = 0.25;
const DEFAULT_MASTER_VOLUME: f32 = 1.0;
const DEFAULT_MUSIC_VOLUME: f32 = 0.5;
const DEFAULT_SFX_VOLUME: f32 = 0.8;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
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

#[derive(Clone)]
struct ReplayFrame {
    ball: Transform3D,
    pins: Vec<Transform3D>,
}

#[derive(Clone)]
struct ReplayCameraInfo {
    node: Gd<Node3D>,
    name: String,
    half_speed: bool,
}

#[derive(Clone, PartialEq)]
enum GamePhase {
    TakingShot {
        launched: bool,
        waiting_report: bool,
        settle_secs: f64,
        start_fallen: i32,
        recording_started: bool,
    },
    Shot,
    AdvertiseResult {
        announcement: String,
    },
    PlayReplay {
        announcement: String,
        camera_index: usize,
        frame_index: usize,
        frame_t: f32,
    },
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
    pending_ball_respawn: bool,
    ball_start_initial_transform: Option<Transform3D>,
    ball_setup_action: Option<BallSetupAction>,
    ball_setup_hold_secs: f64,
    ball_setup_next_repeat_secs: f64,
    zoomed_in: bool,
    replay_samples: Vec<ReplayFrame>,
    replay_saved_zoomed_in: bool,
    replay_pending_scoreboard_previous: Option<ScoreboardState>,
    replay_recording_missing_marker_warned: bool,
    replay_cameras: Vec<ReplayCameraInfo>,
    replay_ball_ghost: Option<Gd<Node3D>>,
    replay_pin_ghosts: Vec<Gd<Node3D>>,
    direction_indicator: Option<Gd<Node3D>>,
    direction_indicator_fade_progress: f64,
    zoomed_in_camera_aim_offset: Option<Transform3D>,
    camera_target_fix_yz: Option<Vector2>,
    music_playlist: Vec<Gd<AudioStream>>,
    music_order: Vec<usize>,
    music_order_index: usize,
    music_track_was_playing: bool,
    replay_audio_waiting_for_finish: bool,
    master_volume: f32,
    music_volume: f32,
    sfx_volume: f32,
    game_phase: GamePhase,
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
            pending_ball_respawn: false,
            ball_start_initial_transform: None,
            ball_setup_action: None,
            ball_setup_hold_secs: 0.0,
            ball_setup_next_repeat_secs: 0.0,
            zoomed_in: false,
            replay_samples: Vec::new(),
            replay_saved_zoomed_in: false,
            replay_pending_scoreboard_previous: None,
            replay_recording_missing_marker_warned: false,
            replay_cameras: Vec::new(),
            replay_ball_ghost: None,
            replay_pin_ghosts: Vec::new(),
            direction_indicator: None,
            direction_indicator_fade_progress: 0.0,
            zoomed_in_camera_aim_offset: None,
            camera_target_fix_yz: None,
            music_playlist: Vec::new(),
            music_order: Vec::new(),
            music_order_index: 0,
            music_track_was_playing: false,
            replay_audio_waiting_for_finish: false,
            master_volume: DEFAULT_MASTER_VOLUME,
            music_volume: DEFAULT_MUSIC_VOLUME,
            sfx_volume: DEFAULT_SFX_VOLUME,
            game_phase: GamePhase::TakingShot {
                launched: false,
                waiting_report: false,
                settle_secs: 0.0,
                start_fallen: 0,
                recording_started: false,
            },
            leave_holding: false,
            leave_hold_secs: 0.0,
        }
    }

    fn ready(&mut self) {
        self.base_mut().set_process(true);
        self.base_mut().set_physics_process(true);
        self.capture_ball_start_initial_transform();
        if let Some(game_manager) = self.try_game_manager_root() {
            self.disable_tweening_on_phantom_cameras(game_manager.upcast::<Node>());
        }
        self.prepare_replay_cameras();
        self.set_zoomed_in(false);
        self.pending_ball_respawn = true;
        self.capture_zoomed_in_camera_aim_offset();

        if self.try_replay_audio_player().is_none() {
            godot_warn!("ReplayAudioPlayer missing under GameManager; replay audio disabled");
        }

        let mut join_button = self
            .base()
            .get_node_as::<Button>("UiManager/Mobile/VBoxContainer/Join");
        let mut calibrate_button = self
            .base()
            .get_node_as::<Button>("UiManager/Mobile/VBoxContainer/Calibrate");
        let mut create_button = self.base().get_node_as::<Button>(
            "UiManager/CenterContainer/VBoxContainer/Desktop/PanelContainer/MarginContainer/VBoxContainer/Create",
        );
        let mut spectate_button = self.base().get_node_as::<Button>(
            "UiManager/CenterContainer/VBoxContainer/Desktop/PanelContainer/MarginContainer/VBoxContainer/Spectate",
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
        let mut rotate_left_button = self.base().get_node_as::<Button>(
            "UiManager/Controller/MarginContainer/VBoxContainer/Rotate/Left",
        );
        let mut rotate_right_button = self.base().get_node_as::<Button>(
            "UiManager/Controller/MarginContainer/VBoxContainer/Rotate/Right",
        );
        let mut controller_back_button = self
            .base()
            .get_node_as::<Button>("UiManager/Controller/MarginContainer/VBoxContainer/BackToMenu");
        let mut game_kick_button = self.base().get_node_as::<Button>(
            "UiManager/GameHud/MarginContainer/VBoxContainer/HostKickRow/KickButton",
        );
        let mut game_stop_button = self.base().get_node_as::<Button>(
            "UiManager/GameHud/MarginContainer/VBoxContainer/StopGameButton",
        );
        let mut game_leave_button = self.base().get_node_as::<Button>(
            "UiManager/GameHud/MarginContainer/VBoxContainer/LeaveGameButton",
        );
        let mut info_back_button = self
            .base()
            .get_node_as::<Button>("UiManager/Info/VBoxContainer/Back");
        let mut zoom_button = self
            .base()
            .get_node_as::<Button>("UiManager/Controller/MarginContainer/VBoxContainer/Zoom");
        let mut settings_button = self
            .base()
            .get_node_as::<Button>("UiManager/CenterContainer/VBoxContainer/Desktop/PanelContainer/MarginContainer/VBoxContainer/Settings");
        let mut settings_close_button = self
            .base()
            .get_node_as::<Button>("UiManager/SettingsPanel/MarginContainer/VBoxContainer/Close");
        let mut master_slider = self
            .base()
            .get_node_as::<HSlider>("UiManager/SettingsPanel/MarginContainer/VBoxContainer/Master");
        let mut music_slider = self
            .base()
            .get_node_as::<HSlider>("UiManager/SettingsPanel/MarginContainer/VBoxContainer/Music");
        let mut sfx_slider = self
            .base()
            .get_node_as::<HSlider>("UiManager/SettingsPanel/MarginContainer/VBoxContainer/Sfx");

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
        move_left_button.connect(
            "button_down",
            &self.base().callable("on_move_left_button_down"),
        );
        move_left_button.connect("button_up", &self.base().callable("on_move_button_up"));
        move_right_button.connect(
            "button_down",
            &self.base().callable("on_move_right_button_down"),
        );
        move_right_button.connect("button_up", &self.base().callable("on_move_button_up"));
        rotate_left_button.connect(
            "button_down",
            &self.base().callable("on_rotate_left_button_down"),
        );
        rotate_left_button.connect("button_up", &self.base().callable("on_rotate_button_up"));
        rotate_right_button.connect(
            "button_down",
            &self.base().callable("on_rotate_right_button_down"),
        );
        rotate_right_button.connect("button_up", &self.base().callable("on_rotate_button_up"));
        controller_back_button
            .connect("button_down", &self.base().callable("on_leave_button_down"));
        controller_back_button.connect("button_up", &self.base().callable("on_leave_button_up"));
        game_kick_button.connect("pressed", &self.base().callable("on_kick_pressed"));
        game_stop_button.connect("pressed", &self.base().callable("on_stop_game_pressed"));
        game_leave_button.connect("pressed", &self.base().callable("on_leave_game_pressed"));
        info_back_button.connect("pressed", &self.base().callable("on_info_back_pressed"));
        zoom_button.connect("pressed", &self.base().callable("on_zoom_pressed"));
        settings_button.connect("pressed", &self.base().callable("on_settings_pressed"));
        settings_close_button.connect("pressed", &self.base().callable("on_settings_close_pressed"));
        master_slider.connect("value_changed", &self.base().callable("on_master_volume_changed"));
        music_slider.connect("value_changed", &self.base().callable("on_music_volume_changed"));
        sfx_slider.connect("value_changed", &self.base().callable("on_sfx_volume_changed"));

        self.ensure_username();
        self.load_calibration();
        self.load_audio_settings();
        master_slider.set_value(self.master_volume as f64);
        music_slider.set_value(self.music_volume as f64);
        sfx_slider.set_value(self.sfx_volume as f64);
        self.apply_audio_settings();
        self.prepare_music_playlist();
        self.start_music_if_needed();
        self.sync_username_input();
        self.try_auto_reconnect();
    }

    fn process(&mut self, delta: f64) {
        self.accel = self.browser_accel();
        self.is_mobile = self.is_mobile_web();
        self.apply_audio_settings();
        self.poll_replay_audio_finished();
        self.poll_music_finished();
        self.start_music_if_needed();

        self.process_pending_ball_respawn();
        self.sync_zoomed_in_camera_to_aim();
        self.sync_camera_target_to_ball_with_clamp();
        self.update_direction_indicator(delta);
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

        if self.screen != Screen::MainMenu || is_mobile {
            self.set_settings_panel_visible(false);
        }

        let accel = self.accel;
        let calibration_active = self.calibration_active;
        let calibration_samples_len = self.calibration_samples.len();
        let calibration_offset_x = self.calibration_offset_x;

        let controller_status = self.controller_status_text();
        let controller_force = self.controller_force;
        let controller_direction = self.controller_direction;
        let controller_enabled = self.can_accept_controller_gameplay_input();

        let current_player_label = if self.current_player_id.is_empty() {
            "Current turn: waiting for player".to_string()
        } else {
            format!("Current turn: {}", self.current_player_name())
        };
        let lobby_code = self.lobby_code.clone();
        let is_host = self.is_host();
        let show_spectate_leave = self.is_spectator();
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
        rendering::render_game_ui(
            self,
            &current_player_label,
            &lobby_code,
            is_host,
            show_spectate_leave,
        );
        rendering::render_scoreboard(self, &scoreboard);
        rendering::render_info_text(self, &info_text);
    }

    fn physics_process(&mut self, delta: f64) {
        self.update_replay_recording();
        self.update_replay_playback(delta);
    }
}

impl GameState {
    fn is_host(&self) -> bool {
        self.session_role == Some(SessionRole::Host)
    }

    fn is_spectator(&self) -> bool {
        self.session_role.is_none()
    }

    fn is_replay_active(&self) -> bool {
        matches!(
            self.game_phase,
            GamePhase::AdvertiseResult { .. } | GamePhase::PlayReplay { .. }
        )
    }

    fn can_accept_controller_gameplay_input(&self) -> bool {
        if self.calibration_active {
            return true;
        }

        self.is_local_player_turn()
            && matches!(
                self.game_phase,
                GamePhase::TakingShot {
                    launched: false,
                    waiting_report: false,
                    ..
                }
            )
    }

    fn clear_controller_input_state(&mut self) {
        self.controller_holding = false;
        self.controller_force = 0.0;
        self.controller_direction = Vector2::new(0.0, 1.0);
        self.controller_baseline_accel = Vector3::ZERO;
        self.controller_motion_history.clear();
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
        self.scoreboard
            .players
            .iter()
            .find(|player| player.player_id == self.current_player_id)
            .map(|player| player.username.clone())
            .or_else(|| {
                if self.current_player_id.is_empty() {
                    None
                } else {
                    Some(self.current_player_id.clone())
                }
            })
            .unwrap_or_else(|| "Waiting for player".to_string())
    }

    fn try_music_player(&self) -> Option<Gd<AudioStreamPlayer>> {
        self.base()
            .get_node_or_null("MusicPlayer")
            .and_then(|node| node.try_cast::<AudioStreamPlayer>().ok())
    }

    fn linear_to_db(level: f32) -> f32 {
        if level <= 0.0001 {
            -80.0
        } else {
            20.0 * level.log10()
        }
    }

    fn set_settings_panel_visible(&mut self, visible: bool) {
        if let Some(mut center) = self.base().get_node_or_null("UiManager/CenterContainer") {
            center.set("visible", &(!visible).to_variant());
        }

        let Some(mut panel) = self.base().get_node_or_null("UiManager/SettingsPanel") else {
            return;
        };
        panel.set("visible", &visible.to_variant());
    }

    fn load_audio_settings(&mut self) {
        self.master_volume = storage::get_value(self, storage::STORAGE_AUDIO_MASTER)
            .parse::<f32>()
            .ok()
            .map(|v| v.clamp(0.0, 1.0))
            .unwrap_or(DEFAULT_MASTER_VOLUME);
        self.music_volume = storage::get_value(self, storage::STORAGE_AUDIO_MUSIC)
            .parse::<f32>()
            .ok()
            .map(|v| v.clamp(0.0, 1.0))
            .unwrap_or(DEFAULT_MUSIC_VOLUME);
        self.sfx_volume = storage::get_value(self, storage::STORAGE_AUDIO_SFX)
            .parse::<f32>()
            .ok()
            .map(|v| v.clamp(0.0, 1.0))
            .unwrap_or(DEFAULT_SFX_VOLUME);
    }

    fn save_audio_settings(&self) {
        storage::set_value(
            self,
            storage::STORAGE_AUDIO_MASTER,
            &format!("{}", self.master_volume),
        );
        storage::set_value(
            self,
            storage::STORAGE_AUDIO_MUSIC,
            &format!("{}", self.music_volume),
        );
        storage::set_value(self, storage::STORAGE_AUDIO_SFX, &format!("{}", self.sfx_volume));
    }

    fn apply_audio_settings(&mut self) {
        let master = if self.is_mobile { 0.0 } else { self.master_volume };

        if let Some(mut music_player) = self.try_music_player() {
            let music_level = (master * self.music_volume).clamp(0.0, 1.0);
            music_player.set("volume_db", &Self::linear_to_db(music_level).to_variant());
            if self.is_mobile {
                music_player.call("stop", &[]);
            }
        }

        if let Some(mut replay_audio) = self.try_replay_audio_player() {
            let sfx_level = (master * self.sfx_volume).clamp(0.0, 1.0);
            replay_audio.set("volume_db", &Self::linear_to_db(sfx_level).to_variant());
        }
    }

    fn prepare_music_playlist(&mut self) {
        self.music_playlist = vec![
            load::<AudioStream>("res://Music/0.mp3"),
            load::<AudioStream>("res://Music/1.mp3"),
        ];
        self.music_order = (0..self.music_playlist.len()).collect();
        self.music_order.shuffle(&mut rand::rng());
        self.music_order_index = 0;
    }

    fn play_current_music_track(&mut self) {
        if self.music_playlist.is_empty() || self.music_order.is_empty() || self.is_mobile {
            return;
        }

        let Some(mut music_player) = self.try_music_player() else {
            return;
        };

        if self.music_order_index >= self.music_order.len() {
            self.music_order.shuffle(&mut rand::rng());
            self.music_order_index = 0;
        }

        let track_index = self.music_order[self.music_order_index];
        let track = self.music_playlist[track_index].clone();
        music_player.set_stream(&track);
        music_player.call("play", &[]);
    }

    fn start_music_if_needed(&mut self) {
        if self.is_mobile {
            if let Some(mut music_player) = self.try_music_player() {
                music_player.call("stop", &[]);
            }
            self.music_track_was_playing = false;
            return;
        }

        let Some(music_player) = self.try_music_player() else {
            return;
        };

        if self.music_playlist.is_empty() || self.music_order.is_empty() {
            return;
        }

        if !music_player.is_playing() {
            self.play_current_music_track();
        }

        self.music_track_was_playing = true;
    }

    fn poll_music_finished(&mut self) {
        if self.is_mobile {
            self.music_track_was_playing = false;
            return;
        }

        let Some(music_player) = self.try_music_player() else {
            return;
        };

        if music_player.is_playing() {
            self.music_track_was_playing = true;
            return;
        }

        if self.music_track_was_playing {
            self.music_track_was_playing = false;
            self.handle_music_finished();
        }
    }

    fn poll_replay_audio_finished(&mut self) {
        if !self.replay_audio_waiting_for_finish {
            return;
        }

        let Some(replay_audio) = self.try_replay_audio_player() else {
            self.replay_audio_waiting_for_finish = false;
            self.start_replay_playback();
            return;
        };

        if replay_audio.is_playing() {
            return;
        }

        self.replay_audio_waiting_for_finish = false;
        self.start_replay_playback();
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

    fn try_camera_target(&self) -> Option<Gd<Node3D>> {
        self.base()
            .get_node_or_null("GameManager/CameraTarget")
            .and_then(|node| node.try_cast::<Node3D>().ok())
    }

    fn try_ball_start_point(&self) -> Option<Gd<Node3D>> {
        self.base()
            .get_node_or_null("GameManager/BallStartPoint")
            .and_then(|node| node.try_cast::<Node3D>().ok())
    }

    fn try_ball_end_point(&self) -> Option<Gd<Node3D>> {
        self.base()
            .get_node_or_null("GameManager/BallEndPoint")
            .and_then(|node| node.try_cast::<Node3D>().ok())
    }

    fn sync_camera_target_to_ball_with_clamp(&mut self) {
        let Some(ball) = self.try_ball() else {
            self.camera_target_fix_yz = None;
            return;
        };
        let stop_marker = self.try_camera_track_stop();
        let fix_start = self.try_camera_fix_start();
        let Some(marker) = stop_marker.clone().or_else(|| self.try_replay_start_marker()) else {
            self.camera_target_fix_yz = None;
            return;
        };
        let Some(mut camera_target) = self.try_camera_target() else {
            self.camera_target_fix_yz = None;
            return;
        };

        let ball_transform = ball.get_global_transform();
        let marker_x = marker.get_global_position().x;

        let mut target_transform = ball_transform;
        target_transform.origin.x = ball_transform.origin.x.min(marker_x);
        if let Some(fix_start) = fix_start {
            let fix_start_position = fix_start.get_global_position();
            if ball_transform.origin.x >= fix_start_position.x {
                if self.camera_target_fix_yz.is_none() {
                    let current_target_pos = camera_target.get_global_position();
                    self.camera_target_fix_yz = Some(Vector2::new(
                        current_target_pos.y,
                        current_target_pos.z,
                    ));
                }

                if let Some(fixed_yz) = self.camera_target_fix_yz {
                    target_transform.origin.y = fixed_yz.x;
                    target_transform.origin.z = fixed_yz.y;
                }
            } else {
                self.camera_target_fix_yz = None;
            }
        } else {
            self.camera_target_fix_yz = None;
        }
        camera_target.set_global_transform(target_transform);
    }

    fn try_zoomed_in_camera(&self) -> Option<Gd<Node3D>> {
        self.base()
            .get_node_or_null("GameManager/PhantomCameraZoomedIn")
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
        if !matches!(
            self.game_phase,
            GamePhase::TakingShot {
                launched: false,
                waiting_report: false,
                ..
            }
        ) {
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

    fn capture_zoomed_in_camera_aim_offset(&mut self) {
        let Some(start) = self.try_ball_start_point() else {
            return;
        };

        let Some(camera) = self.try_zoomed_in_camera() else {
            return;
        };

        let start_transform = start.get_global_transform();
        let camera_transform = camera.get_global_transform();
        self.zoomed_in_camera_aim_offset = Some(start_transform.affine_inverse() * camera_transform);
    }

    fn sync_zoomed_in_camera_to_aim(&mut self) {
        let taking_aim = matches!(
            self.game_phase,
            GamePhase::TakingShot {
                launched: false,
                waiting_report: false,
                ..
            }
        );

        if !taking_aim {
            return;
        }

        let Some(start) = self.try_ball_start_point() else {
            return;
        };

        let Some(mut camera) = self.try_zoomed_in_camera() else {
            return;
        };

        if self.zoomed_in_camera_aim_offset.is_none() {
            self.capture_zoomed_in_camera_aim_offset();
        }

        let Some(offset) = self.zoomed_in_camera_aim_offset else {
            return;
        };

        let start_transform = start.get_global_transform();
        camera.set_global_transform(start_transform * offset);
    }

    fn ensure_direction_indicator(&mut self) -> Option<Gd<Node3D>> {
        if let Some(indicator) = &self.direction_indicator {
            return Some(indicator.clone());
        }

        let mut game_manager = self.try_game_manager_root()?;
        let indicator_scene = load::<PackedScene>("res://Assets/DirectionIndicator.tscn");
        let mut indicator = indicator_scene.instantiate_as::<Node3D>();
        indicator.set_name("DirectionIndicator");
        game_manager.add_child(&indicator);
        self.direction_indicator = Some(indicator.clone());
        Some(indicator)
    }

    fn clear_direction_indicator(&mut self) {
        if let Some(mut indicator) = self.direction_indicator.take() {
            indicator.queue_free();
        }
    }

    fn update_direction_indicator(&mut self, delta: f64) {
        let taking_aim = matches!(
            self.game_phase,
            GamePhase::TakingShot {
                launched: false,
                waiting_report: false,
                ..
            }
        );

        if !taking_aim {
            self.direction_indicator_fade_progress = 0.0;
            self.clear_direction_indicator();
            return;
        }

        let Some(start) = self.try_ball_start_point() else {
            self.clear_direction_indicator();
            return;
        };

        let Some(end) = self.try_ball_end_point() else {
            self.clear_direction_indicator();
            return;
        };

        let Some(mut indicator) = self.ensure_direction_indicator() else {
            return;
        };

        let start_transform = start.get_global_transform();
        let mut start_point = start_transform.origin;
        start_point.y = 0.05;

        let forward = start_transform.basis * Vector3::RIGHT;
        let target_x = end.get_global_position().x;
        let travel = if forward.x.abs() > 0.0001 {
            ((target_x - start_point.x) / forward.x).max(0.0)
        } else {
            0.0
        } * 0.8;

        start_point = start_point + forward;

        let mut finish_point = start_point + (forward * travel);
        finish_point.y = 0.05;

        indicator.call(
            "set_points",
            &[start_point.to_variant(), finish_point.to_variant()],
        );

        if self.controller_holding {
            self.direction_indicator_fade_progress = (self.direction_indicator_fade_progress
                + (delta / DIRECTION_INDICATOR_FADE_SECS))
                .clamp(0.0, 1.0);
        } else {
            self.direction_indicator_fade_progress = 0.0;
        }

        indicator.call(
            "set_fade",
            &[self.direction_indicator_fade_progress.to_variant()],
        );
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
        self.set_live_camera_priorities(zoomed_in);
    }

    fn try_replay_cameras_root(&self) -> Option<Gd<Node3D>> {
        self.base()
            .get_node_or_null("GameManager/ReplayCameras")
            .and_then(|node| node.try_cast::<Node3D>().ok())
    }

    fn collect_replay_cameras(&self) -> Vec<ReplayCameraInfo> {
        let Some(root) = self.try_replay_cameras_root() else {
            return Vec::new();
        };

        let mut cameras = Vec::new();
        for child in root.get_children().iter_shared() {
            if child.has_method("set_priority") && child.has_method("set_tween_duration") {
                if let Ok(camera) = child.try_cast::<Node3D>() {
                    let name = camera.get_name().to_string();
                    cameras.push(ReplayCameraInfo {
                        half_speed: name.to_ascii_lowercase().contains("slow"),
                        name,
                        node: camera,
                    });
                }
            } else {
                godot_warn!("Ignoring non-PhantomCamera child under ReplayCameras");
            }
        }

        cameras
    }

    fn prepare_replay_cameras(&mut self) {
        self.replay_cameras = self.collect_replay_cameras();

        for camera in &mut self.replay_cameras {
            camera.node.set("tween_duration", &0.0.to_variant());
        }

        if self.replay_cameras.is_empty() {
            godot_warn!("ReplayCameras has no camera children; replay will be skipped");
        }
    }

    fn collect_live_cameras(&self) -> Vec<ReplayCameraInfo> {
        let Some(game_manager) = self.try_game_manager_root() else {
            return Vec::new();
        };

        let mut cameras = Vec::new();
        for child in game_manager.get_children().iter_shared() {
            if child.get_name().to_string() == "ReplayCameras" {
                continue;
            }

            if child.has_method("set_priority") && child.has_method("set_tween_duration") {
                if let Ok(camera) = child.try_cast::<Node3D>() {
                    let name = camera.get_name().to_string();
                    cameras.push(ReplayCameraInfo {
                        half_speed: name.to_ascii_lowercase().contains("slow"),
                        name,
                        node: camera,
                    });
                }
            }
        }

        cameras
    }

    fn set_live_camera_priorities(&mut self, zoomed_in: bool) {
        let cameras = self.collect_live_cameras();

        let mut active_rank = 0;
        for camera in cameras {
            let is_zoom = camera.name.to_ascii_lowercase().contains("zoom");
            let is_active_group = if zoomed_in { is_zoom } else { !is_zoom };
            let priority = if is_active_group {
                let value = if active_rank == 0 { 2 } else { 1 };
                active_rank += 1;
                value
            } else {
                0
            };

            let mut node = camera.node;
            node.set("priority", &priority.to_variant());
        }
    }

    fn retarget_live_cameras_to_ball(&self, ball: &Gd<Ball>) {
        let ball = ball.to_variant();
        for camera in self.collect_live_cameras() {
            if camera.name == "PhantomCameraNormal" {
                continue;
            }
            let mut node = camera.node;
            node.set("follow_target", &ball);
            node.set("look_at_target", &ball);
        }
    }

    fn disable_tweening_on_phantom_cameras(&self, mut node: Gd<Node>) {
        if node.has_method("set_tween_duration") {
            node.call("set_tween_duration", &[0.0.to_variant()]);
        }

        for child in node.get_children().iter_shared() {
            self.disable_tweening_on_phantom_cameras(child);
        }
    }

    fn try_game_manager_root(&self) -> Option<Gd<Node3D>> {
        self.base()
            .get_node_or_null("GameManager")
            .and_then(|node| node.try_cast::<Node3D>().ok())
    }

    fn try_replay_node(&self, path: &str) -> Option<Gd<Node>> {
        self.try_game_manager_root()
            .and_then(|root| root.get_node_or_null(path))
    }

    fn try_replay_start_marker(&self) -> Option<Gd<Node3D>> {
        self.try_replay_node("ReplayStartMarker")
            .and_then(|node| node.try_cast::<Node3D>().ok())
    }

    fn try_camera_track_stop(&self) -> Option<Gd<Node3D>> {
        self.base()
            .get_node_or_null("GameManager/CameraTrackStop")
            .and_then(|node| node.try_cast::<Node3D>().ok())
    }

    fn try_camera_fix_start(&self) -> Option<Gd<Node3D>> {
        self.base()
            .get_node_or_null("GameManager/CameraFixStart")
            .and_then(|node| node.try_cast::<Node3D>().ok())
    }

    fn try_replay_audio_player(&self) -> Option<Gd<AudioStreamPlayer>> {
        self.try_replay_node("ReplayAudioPlayer")
            .and_then(|node| node.try_cast::<AudioStreamPlayer>().ok())
    }

    fn replay_audio_path(announcement: &str) -> &'static str {
        match announcement {
            "strike" => "res://Assets/NiceStrike.wav",
            "spare" => "res://Assets/NiceSpare.wav",
            "gutter ball" => "res://Assets/GutterBall.wav",
            _ => "res://Assets/NiceShot.wav",
        }
    }

    fn try_replay_ghost_root(&self) -> Option<Gd<Node3D>> {
        self.try_replay_node("ReplayGhosts")
            .and_then(|node| node.try_cast::<Node3D>().ok())
    }

    fn clear_replay_ghosts(&mut self) {
        if let Some(root) = self.try_replay_ghost_root() {
            let children = root.get_children();
            for mut child in children.iter_shared() {
                child.queue_free();
            }
        }

        self.replay_ball_ghost = None;
        self.replay_pin_ghosts.clear();
    }

    fn reset_replay_flow(&mut self) {
        self.game_phase = GamePhase::TakingShot {
            launched: false,
            waiting_report: false,
            settle_secs: 0.0,
            start_fallen: 0,
            recording_started: false,
        };
        self.replay_samples.clear();
        self.replay_pending_scoreboard_previous = None;
        self.replay_recording_missing_marker_warned = false;
        self.replay_cameras.clear();
        self.clear_replay_ghosts();
        self.hide_live_replay_subjects(true);
        self.replay_audio_waiting_for_finish = false;
        if let Some(mut audio) = self.try_replay_audio_player() {
            audio.call("stop", &[]);
        }
    }

    fn hide_live_replay_subjects(&mut self, visible: bool) {
        if let Some(mut ball) = self.try_ball() {
            ball.set_visible(visible);
        }

        for (_, mut pin_root) in self.pin_roots() {
            pin_root.set_visible(visible);
        }
    }

    fn pin_roots(&self) -> Vec<(usize, Gd<Node3D>)> {
        let mut roots = Vec::new();

        let Some(pins_root) = self.try_pins_root() else {
            return roots;
        };

        for pin_root in pins_root.get_children().iter_shared() {
            let pin_index = pin_root.get("pin_index").try_to::<i32>().unwrap_or(-1);
            if pin_index < 0 {
                continue;
            }

            if let Ok(pin_root) = pin_root.try_cast::<Node3D>() {
                roots.push((pin_index as usize, pin_root));
            }
        }

        roots.sort_by_key(|(pin_index, _)| *pin_index);
        roots
    }

    fn try_pins_root(&self) -> Option<Gd<Node3D>> {
        let game_manager = self.try_game_manager()?;

        for child in game_manager.get_children().iter_shared() {
            let Ok(node3d) = child.try_cast::<Node3D>() else {
                continue;
            };

            for pin in node3d.get_children().iter_shared() {
                let pin_index = pin.get("pin_index").try_to::<i32>().unwrap_or(-1);
                if pin_index >= 0 {
                    return Some(node3d);
                }
            }
        }

        None
    }

    fn pin_bodies(&self) -> Vec<(usize, Gd<RigidBody3D>)> {
        let mut bodies = Vec::new();

        let Some(pins_root) = self.try_pins_root() else {
            return bodies;
        };

        for pin_root in pins_root.get_children().iter_shared() {
            let pin_index = pin_root.get("pin_index").try_to::<i32>().unwrap_or(-1);
            if pin_index < 0 {
                continue;
            }

            let mut body = None;
            for child in pin_root.get_children().iter_shared() {
                if let Ok(rigid_body) = child.try_cast::<RigidBody3D>() {
                    body = Some(rigid_body);
                    break;
                }
            }

            if let Some(body) = body {
                bodies.push((pin_index as usize, body));
            }
        }

        bodies.sort_by_key(|(pin_index, _)| *pin_index);
        bodies
    }

    fn copy_mesh_instances_recursive(&self, source: Gd<Node>, ghost_parent: &mut Gd<Node3D>) {
        for child in source.get_children().iter_shared() {
            if let Ok(mesh) = child.clone().try_cast::<MeshInstance3D>() {
                let mut ghost_mesh = MeshInstance3D::new_alloc();
                ghost_mesh.set("mesh", &mesh.get("mesh"));
                ghost_mesh.set_transform(mesh.get_transform());
                ghost_parent.add_child(&ghost_mesh);
            }

            self.copy_mesh_instances_recursive(child, ghost_parent);
        }
    }

    fn spawn_replay_ghosts(&mut self) {
        self.clear_replay_ghosts();

        let Some(mut ghost_root) = self.try_replay_ghost_root() else {
            return;
        };

        if let Some(ball) = self.try_ball() {
            let mut ball_ghost = Node3D::new_alloc();
            ball_ghost.set_global_transform(ball.get_global_transform());
            self.copy_mesh_instances_recursive(ball.clone().upcast::<Node>(), &mut ball_ghost);
            ghost_root.add_child(&ball_ghost);
            self.replay_ball_ghost = Some(ball_ghost);
        }

        for (_, pin_root) in self.pin_roots() {
            let mut pin_ghost = Node3D::new_alloc();
            pin_ghost.set_global_transform(pin_root.get_global_transform());
            self.copy_mesh_instances_recursive(pin_root.clone().upcast::<Node>(), &mut pin_ghost);
            ghost_root.add_child(&pin_ghost);
            self.replay_pin_ghosts.push(pin_ghost);
        }
    }

    fn interpolate_transform(a: Transform3D, b: Transform3D, t: f32) -> Transform3D {
        let mut out = a;
        out.origin = a.origin.lerp(b.origin, t);
        out.basis = a.basis.slerp(&b.basis, t);
        out
    }

    fn update_replay_ghosts_interpolated(&mut self, from: &ReplayFrame, to: &ReplayFrame, t: f32) {
        let clamped_t = t.clamp(0.0, 1.0);

        if let Some(ball_ghost) = self.replay_ball_ghost.as_mut() {
            ball_ghost.set_global_transform(Self::interpolate_transform(from.ball, to.ball, clamped_t));
        }

        for (ghost, (from_pin, to_pin)) in self
            .replay_pin_ghosts
            .iter_mut()
            .zip(from.pins.iter().zip(to.pins.iter()))
        {
            ghost.set_global_transform(Self::interpolate_transform(*from_pin, *to_pin, clamped_t));
        }
    }

    fn set_replay_subjects_frozen(&mut self, frozen: bool) {
        if let Some(mut ball) = self.try_ball() {
            ball.set_freeze_enabled(frozen);
            ball.set_sleeping(frozen);
        }

        for (_, mut pin_body) in self.pin_bodies() {
            pin_body.set_freeze_enabled(frozen);
            pin_body.set_sleeping(frozen);
        }
    }

    fn set_replay_camera_priorities(&mut self, active_camera_index: Option<usize>) {
        for (camera_index, replay_camera) in self.replay_cameras.iter_mut().enumerate() {
            let priority = if Some(camera_index) == active_camera_index {
                4
            } else {
                0
            };

            replay_camera.node.set("priority", &priority.to_variant());
        }

        if active_camera_index.is_none() {
            self.set_zoomed_in(self.replay_saved_zoomed_in);
        }
    }

    fn capture_replay_frame(&mut self) {
        let Some(ball) = self.try_ball() else {
            return;
        };

        let mut pins = Vec::new();
        for (_, pin_body) in self.pin_bodies() {
            pins.push(pin_body.get_global_transform());
        }

        self.replay_samples.push(ReplayFrame {
            ball: ball.get_global_transform(),
            pins,
        });
    }

    fn update_replay_recording(&mut self) {
        let GamePhase::TakingShot {
            launched,
            waiting_report: _,
            recording_started,
            ..
        } = &self.game_phase
        else {
            return;
        };

        if !*launched {
            return;
        }

        let Some(ball) = self.try_ball() else {
            return;
        };

        if !ball.bind().is_launched() {
            return;
        }

        let Some(marker) = self.try_replay_start_marker() else {
            if !self.replay_recording_missing_marker_warned {
                self.replay_recording_missing_marker_warned = true;
                godot_warn!(
                    "ReplayStartMarker missing under GameManager; replay capture disabled for this shot"
                );
            }
            return;
        };

        let ball_x = ball.get_global_position().x;
        let marker_x = marker.get_global_position().x;

        let mut should_capture = true;
        let recording_started = *recording_started;

        if !recording_started {
            if ball_x <= marker_x {
                godot_print!(
                    "replay not started yet: ball_x={}, marker_x={}",
                    ball_x,
                    marker_x
                );
                should_capture = false;
            } else {
                if let GamePhase::TakingShot {
                    recording_started, ..
                } = &mut self.game_phase
                {
                    *recording_started = true;
                }
                self.replay_samples.clear();
                godot_print!(
                    "start recording replay samples: ball_x={}, marker_x={}",
                    ball_x,
                    marker_x
                );
            }
        }

        if should_capture {
            self.capture_replay_frame();
        }
    }

    fn begin_shot_replay(&mut self, announcement: String) {
        let replay_audio_path = Self::replay_audio_path(&announcement);
        self.game_phase = GamePhase::AdvertiseResult {
            announcement: announcement.clone(),
        };
        self.info_text = announcement;
        self.replay_saved_zoomed_in = self.zoomed_in;

        self.clear_controller_input_state();

        if self.replay_samples.is_empty() {
            godot_print!("replay skipped: no samples recorded");
            self.finish_replay();
            return;
        }

        if let Some(mut audio) = self.try_replay_audio_player() {
            let stream = load::<AudioStream>(replay_audio_path);
            audio.call("stop", &[]);
            audio.set("stream", &stream.to_variant());
            audio.call("play", &[]);
            self.replay_audio_waiting_for_finish = true;
        } else {
            godot_warn!("ReplayAudioPlayer missing under GameManager; starting replay immediately");
            self.start_replay_playback();
        }
    }

    fn start_replay_playback(&mut self) {
        let announcement = match &self.game_phase {
            GamePhase::AdvertiseResult { announcement } => announcement.clone(),
            _ => return,
        };

        self.prepare_replay_cameras();

        if self.replay_samples.is_empty() {
            self.finish_replay();
            return;
        }

        if self.replay_cameras.is_empty() {
            self.finish_replay();
            return;
        }

        godot_print!("start replay playback");
        godot_print!("replay cameras discovered: {}", self.replay_cameras.len());
        for (index, camera) in self.replay_cameras.iter().enumerate() {
            godot_print!(
                "replay camera {}: {} ({})",
                index,
                camera.name,
                if camera.half_speed {
                    "half speed"
                } else {
                    "normal speed"
                }
            );
        }
        self.set_replay_subjects_frozen(true);
        self.spawn_replay_ghosts();
        self.hide_live_replay_subjects(false);
        self.set_replay_camera_priorities(Some(0));
        self.game_phase = GamePhase::PlayReplay {
            announcement,
            camera_index: 0,
            frame_index: 0,
            frame_t: 0.0,
        };
    }

    fn update_replay_playback(&mut self, delta: f64) {
        let (announcement, camera_index, frame_index, frame_t) = match &self.game_phase {
            GamePhase::PlayReplay {
                announcement,
                camera_index,
                frame_index,
                frame_t,
            } => (
                announcement.clone(),
                *camera_index,
                *frame_index,
                *frame_t,
            ),
            _ => return,
        };

        if camera_index >= self.replay_cameras.len() {
            self.finish_replay();
            return;
        }

        let camera_half_speed = self.replay_cameras[camera_index].half_speed;

        if frame_index >= self.replay_samples.len() {
            let next_camera = camera_index + 1;
            if next_camera >= self.replay_cameras.len() {
                self.finish_replay();
                return;
            }

            godot_print!("advance replay camera: {} -> {}", camera_index, next_camera);
            self.set_replay_camera_priorities(Some(next_camera));
            self.game_phase = GamePhase::PlayReplay {
                announcement,
                camera_index: next_camera,
                frame_index: 0,
                frame_t: 0.0,
            };
            return;
        }

        let next_frame_index = frame_index + 1;
        if next_frame_index >= self.replay_samples.len() {
            let next_camera = camera_index + 1;
            if next_camera >= self.replay_cameras.len() {
                self.finish_replay();
                return;
            }

            godot_print!("advance replay camera: {} -> {}", camera_index, next_camera);
            self.set_replay_camera_priorities(Some(next_camera));
            self.game_phase = GamePhase::PlayReplay {
                announcement,
                camera_index: next_camera,
                frame_index: 0,
                frame_t: 0.0,
            };
            return;
        }

        let from = self.replay_samples[frame_index].clone();
        let to = self.replay_samples[next_frame_index].clone();
        self.update_replay_ghosts_interpolated(&from, &to, frame_t);

        let playback_speed = if camera_half_speed { 0.5 } else { 1.0 };
        let next_frame_t = frame_t + (delta as f32 * 60.0 * playback_speed as f32);

        let (out_frame_index, out_frame_t) = if next_frame_t >= 1.0 {
            (next_frame_index, next_frame_t - 1.0)
        } else {
            (frame_index, next_frame_t)
        };

        self.game_phase = GamePhase::PlayReplay {
            announcement,
            camera_index,
            frame_index: out_frame_index,
            frame_t: out_frame_t,
        };
    }

    fn finish_replay(&mut self) {
        let was_active = self.is_replay_active();
        godot_print!("end replay playback");
        self.set_replay_camera_priorities(None);

        self.clear_replay_ghosts();
        self.hide_live_replay_subjects(true);

        if let Some(previous) = self.replay_pending_scoreboard_previous.take() {
            self.sync_host_lane_to_scoreboard(&previous);
        }

        self.set_replay_subjects_frozen(false);

        self.replay_samples.clear();
        self.replay_cameras.clear();
        self.game_phase = GamePhase::TakingShot {
            launched: false,
            waiting_report: false,
            settle_secs: 0.0,
            start_fallen: 0,
            recording_started: false,
        };

        if was_active && self.is_host() {
            self.send_message(ClientMessage::ReplayComplete);
        }
    }

    fn handle_music_finished(&mut self) {
        if self.music_order.is_empty() {
            return;
        }

        self.music_order_index += 1;
        if self.music_order_index >= self.music_order.len() {
            self.music_order.shuffle(&mut rand::rng());
            self.music_order_index = 0;
        }

        self.play_current_music_track();
    }

    fn start_ball_setup_hold(&mut self, action: BallSetupAction) {
        if !self.can_accept_controller_gameplay_input() || self.calibration_active {
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

    fn consume_controller_input_for_replay(&mut self) -> bool {
        self.stop_ball_setup_hold();
        self.clear_controller_input_state();
        if self.is_replay_active() {
            if self.is_local_player_turn() {
                self.send_message(ClientMessage::SkipReplay);
            }
            return true;
        }

        false
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

        self.retarget_live_cameras_to_ball(&ball);

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
        self.game_phase = GamePhase::TakingShot {
            launched: false,
            waiting_report: false,
            settle_secs: 0.0,
            start_fallen: 0,
            recording_started: false,
        };
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

        let Some(ball) = self.try_ball() else {
            return;
        };

        let (launched, mut waiting_report, mut settle_secs, start_fallen, recording_started) =
            match &self.game_phase {
            GamePhase::TakingShot {
                launched,
                waiting_report,
                settle_secs,
                start_fallen,
                recording_started,
                ..
            } => (
                *launched,
                *waiting_report,
                *settle_secs,
                *start_fallen,
                *recording_started,
            ),
            _ => return,
        };

        let passed_end = {
            let ball = ball.bind();
            ball.has_passed_end()
        };

        if launched && !waiting_report && passed_end {
            waiting_report = true;
            settle_secs = 0.0;
        }

        if waiting_report {
            settle_secs += delta;

            if settle_secs < THROW_SETTLE_SECS {
                self.game_phase = GamePhase::TakingShot {
                    launched,
                    waiting_report,
                    settle_secs,
                    start_fallen,
                    recording_started,
                };
                return;
            }

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

            godot_print!(
                "stop recording replay samples: {} frames",
                self.replay_samples.len()
            );

            self.game_phase = GamePhase::Shot;

            if let Some(mut ball) = self.try_ball() {
                ball.bind_mut().finish_throw();
            }
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
        } else {
            self.reset_lane_for_turn();
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
        "ws://127.0.0.1:9000/ws".to_string()
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
                self.reset_replay_flow();
                self.restore_ball_start_point_transform();
                self.reset_lane_for_turn();
                self.screen = self.gameplay_screen();
            }
            ServerMessage::TurnChanged { current_player_id } => {
                self.current_player_id = current_player_id;
                self.clear_controller_input_state();
                self.stop_ball_setup_hold();
                self.set_zoomed_in(false);
                self.reset_replay_flow();
                self.screen = self.gameplay_screen();
            }
            ServerMessage::ThrowEvent {
                player_id: _,
                force,
                direction_x,
                direction_z,
            } => {
                if self.is_replay_active() {
                    return;
                }

                self.clear_controller_input_state();
                self.stop_ball_setup_hold();
                self.replay_recording_missing_marker_warned = false;
                if self.is_host()
                    && let Some(mut ball) = self.try_ball()
                {
                    ball.bind_mut().reset_ball();
                    ball.bind_mut()
                        .launch_throw(force, direction_x, direction_z);
                    self.restore_ball_start_point_transform();
                    self.game_phase = GamePhase::TakingShot {
                        launched: true,
                        waiting_report: false,
                        settle_secs: 0.0,
                        start_fallen: self.current_fallen_count(),
                        recording_started: false,
                    };
                }
            }
            ServerMessage::ShotResolved { announcement } => {
                self.begin_shot_replay(announcement);
            }
            ServerMessage::SkipReplay => {
                if self.is_replay_active() {
                    self.finish_replay();
                }
            }
            ServerMessage::AdjustBallSetup {
                player_id,
                move_z_delta,
                rotate_y_delta_deg,
            } => {
                if self.is_replay_active() {
                    return;
                }

                if self.is_host() {
                    self.apply_ball_setup_adjustment(move_z_delta, rotate_y_delta_deg);
                    self.push_ball_setup_log(&player_id, move_z_delta, rotate_y_delta_deg);
                }
            }
            ServerMessage::ToggleZoom {
                player_id,
                zoomed_in,
            } => {
                if self.is_replay_active() {
                    return;
                }

                if self.is_host() {
                    self.set_zoomed_in(zoomed_in);
                    godot_print!("zoom toggle from {}: {}", player_id, zoomed_in);
                }
            }
            ServerMessage::ScoreboardUpdated { scoreboard } => {
                let previous_scoreboard = self.scoreboard.clone();
                self.current_player_id = scoreboard.current_player_id.clone();
                self.scoreboard = scoreboard;
                if matches!(
                    self.game_phase,
                    GamePhase::TakingShot {
                        launched: false,
                        waiting_report: false,
                        ..
                    }
                ) {
                    self.sync_host_lane_to_scoreboard(&previous_scoreboard);
                } else if self.replay_pending_scoreboard_previous.is_none() {
                    self.replay_pending_scoreboard_previous = Some(previous_scoreboard);
                }
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
                self.stop_ball_setup_hold();
                self.set_zoomed_in(false);
                self.reset_replay_flow();
                self.screen = if self.is_host() {
                    Screen::Host
                } else {
                    Screen::Info
                };
            }
            ServerMessage::Error { message } => {
                godot_error!("SERVER ERROR: {}", message);
                self.info_text = format!("Error: {message}");

                self.clear_controller_input_state();
                self.stop_ball_setup_hold();
                self.set_zoomed_in(false);

                self.reset_replay_flow();

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
        self.clear_controller_input_state();
        self.calibration_active = false;
        self.calibration_samples.clear();
        self.reset_replay_flow();
        self.sync_username_input();
        self.screen = Screen::MainMenu;
    }
}

#[godot_api]
impl GameState {
    #[func]
    fn on_settings_pressed(&mut self) {
        if self.is_mobile || self.screen != Screen::MainMenu {
            return;
        }
        self.set_settings_panel_visible(true);
    }

    #[func]
    fn on_settings_close_pressed(&mut self) {
        self.set_settings_panel_visible(false);
    }

    #[func]
    fn on_master_volume_changed(&mut self, value: f64) {
        self.master_volume = (value as f32).clamp(0.0, 1.0);
        self.apply_audio_settings();
        self.save_audio_settings();
    }

    #[func]
    fn on_music_volume_changed(&mut self, value: f64) {
        self.music_volume = (value as f32).clamp(0.0, 1.0);
        self.apply_audio_settings();
        self.save_audio_settings();
    }

    #[func]
    fn on_sfx_volume_changed(&mut self, value: f64) {
        self.sfx_volume = (value as f32).clamp(0.0, 1.0);
        self.apply_audio_settings();
        self.save_audio_settings();
    }

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
            self.screen = Screen::MainMenu;
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
        let code = self
            .base()
            .get_node_as::<LineEdit>("UiManager/CenterContainer/VBoxContainer/Desktop/PanelContainer/MarginContainer/VBoxContainer/LobbyCode")
            .get_text()
            .to_string()
            .trim()
            .to_string();
        if code.len() != 5 || !code.chars().all(|c| c.is_ascii_digit()) {
            self.info_text = "Lobby code must be 5 digits".to_string();
            self.screen = Screen::Info;
            return;
        }
        self.lobby_code = code;
        self.screen = Screen::Game;
    }

    #[func]
    fn on_leave_game_pressed(&mut self) {
        if !self.is_spectator() {
            return;
        }
        self.leave_game();
    }

    #[func]
    fn on_info_back_pressed(&mut self) {
        self.screen = Screen::MainMenu;
    }

    #[func]
    fn on_leave_button_down(&mut self) {
        self.stop_ball_setup_hold();

        if !self.is_mobile {
            self.leave_holding = false;
            self.leave_hold_secs = 0.0;
            self.leave_game();
            return;
        }

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
        if self.consume_controller_input_for_replay() {
            return;
        }
        if self.zoomed_in {
            return;
        }
        self.stop_ball_setup_hold();
        if !self.can_accept_controller_gameplay_input() {
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
        if self.consume_controller_input_for_replay() {
            return;
        }
        if !self.controller_holding {
            return;
        }

        self.controller_holding = false;
        let force = self.controller_force.clamp(0.0, 1.0);
        let direction = self.controller_direction;
        self.clear_controller_input_state();
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
        if self.consume_controller_input_for_replay() {
            return;
        }
        if !self.can_accept_controller_gameplay_input() || self.calibration_active {
            return;
        }

        self.set_zoomed_in(!self.zoomed_in);
        self.send_message(ClientMessage::ToggleZoom {
            zoomed_in: self.zoomed_in,
        });
    }

    #[func]
    fn on_move_left_button_down(&mut self) {
        if self.consume_controller_input_for_replay() {
            return;
        }
        self.start_ball_setup_hold(BallSetupAction::MoveLeft);
    }

    #[func]
    fn on_move_right_button_down(&mut self) {
        if self.consume_controller_input_for_replay() {
            return;
        }
        self.start_ball_setup_hold(BallSetupAction::MoveRight);
    }

    #[func]
    fn on_move_button_up(&mut self) {
        self.stop_ball_setup_hold();
    }

    #[func]
    fn on_rotate_left_button_down(&mut self) {
        if self.consume_controller_input_for_replay() {
            return;
        }
        self.start_ball_setup_hold(BallSetupAction::RotateLeft);
    }

    #[func]
    fn on_rotate_right_button_down(&mut self) {
        if self.consume_controller_input_for_replay() {
            return;
        }
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
        self.clear_controller_input_state();
        self.screen = Screen::Controller;
    }
}
