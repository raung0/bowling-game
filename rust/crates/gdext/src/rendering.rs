use common::{PlayerInfo, PlayerScoreView, ScoreboardState};
use godot::{
    classes::{Button, HBoxContainer, Label, PanelContainer, ProgressBar, VBoxContainer},
    global::HorizontalAlignment,
    prelude::*,
};

use crate::game_state::GameState;

pub fn render_mobile_accel(state: &mut GameState, accel: Vector3) {
    let mut accel_label = state
        .base_mut()
        .get_node_as::<Label>("UiManager/Mobile/VBoxContainer/Accel");
    accel_label.set_text(&format!(
        "Accel: x={:.2} y={:.2} z={:.2}",
        accel.x, accel.y, accel.z
    ));
}

pub fn render_calibration_status(
    state: &mut GameState,
    calibration_active: bool,
    calibration_samples_len: usize,
    calibration_offset_x: f32,
    calibration_throw_count: usize,
) {
    let mut calibration_label = state
        .base_mut()
        .get_node_as::<Label>("UiManager/Mobile/VBoxContainer/CalibrationStatus");
    let text = if calibration_active {
        format!(
            "Calibration in progress: {}/{}",
            calibration_samples_len, calibration_throw_count
        )
    } else {
        format!("Calibration offset: {:+.3}", calibration_offset_x)
    };
    calibration_label.set_text(&GString::from(text.as_str()));
}

pub fn render_controller_ui(
    state: &mut GameState,
    status: &str,
    force: f32,
    direction: Vector2,
    enabled: bool,
    zoomed_in: bool,
) {
    let mut status_label = state
        .base_mut()
        .get_node_as::<Label>("UiManager/Controller/MarginContainer/VBoxContainer/Status");
    status_label.set_text(&GString::from(status));

    let mut strength_label = state
        .base_mut()
        .get_node_as::<Label>("UiManager/Controller/MarginContainer/VBoxContainer/StrengthLabel");
    strength_label.set_text(&format!(
        "Force: {:.0}%  Dir: ({:.2}, {:.2})",
        force * 100.0,
        direction.x,
        direction.y,
    ));

    let mut strength_bar = state.base_mut().get_node_as::<ProgressBar>(
        "UiManager/Controller/MarginContainer/VBoxContainer/StrengthBar",
    );
    strength_bar.set_value((force * 100.0) as f64);

    let mut hold_button = state
        .base_mut()
        .get_node_as::<Button>("UiManager/Controller/MarginContainer/VBoxContainer/HoldButton");
    hold_button.set_disabled(!enabled || zoomed_in);
}

pub fn render_game_ui(
    state: &mut GameState,
    current_player: &str,
    lobby_code: &str,
    is_host: bool,
    show_spectate_leave: bool,
) {
    let mut turn_label = state
        .base_mut()
        .get_node_as::<Label>("UiManager/GameHud/MarginContainer/VBoxContainer/TurnLabel");
    turn_label.set_text(&GString::from(current_player));

    let lobby_line = if lobby_code.is_empty() {
        "Lobby: -".to_string()
    } else {
        format!("Lobby: {}", lobby_code)
    };
    let mut lobby_label = state
        .base_mut()
        .get_node_as::<Label>("UiManager/GameHud/MarginContainer/VBoxContainer/LobbyLabel");
    lobby_label.set_text(&GString::from(lobby_line.as_str()));

    let mut host_kick_row = state.base_mut().get_node_as::<HBoxContainer>(
        "UiManager/GameHud/MarginContainer/VBoxContainer/HostKickRow",
    );
    host_kick_row.set_visible(is_host);

    let mut stop_game_button = state
        .base_mut()
        .get_node_as::<Button>("UiManager/GameHud/MarginContainer/VBoxContainer/StopGameButton");
    stop_game_button.set_visible(is_host);

    let mut leave_game_button = state
        .base_mut()
        .get_node_as::<Button>("UiManager/GameHud/MarginContainer/VBoxContainer/LeaveGameButton");
    leave_game_button.set_visible(show_spectate_leave);
}

pub fn render_scoreboard(state: &mut GameState, scoreboard: &ScoreboardState) {
    let players = scoreboard.players.clone();
    let current = players.first().cloned();
    let next_players = players.iter().skip(1).take(3).cloned().collect::<Vec<_>>();
    let remaining = players.iter().skip(4).cloned().collect::<Vec<_>>();

    let mut current_root = state.base_mut().get_node_as::<VBoxContainer>(
        "UiManager/GameHud/ScoreboardPanel/MarginContainer/VBoxContainer/CurrentTable",
    );
    clear_container(&mut current_root);
    if let Some(current) = current.as_ref() {
        add_scorecard(&mut current_root, current, true);
    }

    let mut next_root = state.base_mut().get_node_as::<VBoxContainer>(
        "UiManager/GameHud/ScoreboardPanel/MarginContainer/VBoxContainer/NextTables",
    );
    clear_container(&mut next_root);
    for player in &next_players {
        add_scorecard(&mut next_root, player, false);
    }

    let mut remaining_header = state.base_mut().get_node_as::<Label>(
        "UiManager/GameHud/ScoreboardPanel/MarginContainer/VBoxContainer/RemainingHeader",
    );
    remaining_header.set_visible(!remaining.is_empty());

    let mut remaining_root = state.base_mut().get_node_as::<VBoxContainer>(
        "UiManager/GameHud/ScoreboardPanel/MarginContainer/VBoxContainer/RemainingPlayers",
    );
    clear_container(&mut remaining_root);
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

fn add_scorecard(root: &mut Gd<VBoxContainer>, player: &PlayerScoreView, featured: bool) {
    let mut panel = PanelContainer::new_alloc();
    panel.set("size_flags_horizontal", &3.to_variant());
    panel.set("clip_contents", &true.to_variant());

    let mut wrapper = VBoxContainer::new_alloc();
    wrapper.set("size_flags_horizontal", &3.to_variant());
    wrapper.add_theme_constant_override("separation", if featured { 8 } else { 6 });
    panel.add_child(&wrapper);

    let mut header = HBoxContainer::new_alloc();
    header.set("size_flags_horizontal", &3.to_variant());
    let mut name = Label::new_alloc();
    name.set_text(&GString::from(player.username.as_str()));
    name.add_theme_font_size_override("font_size", if featured { 24 } else { 20 });
    header.add_child(&name);

    let mut status = Label::new_alloc();
    status.set_text(&GString::from(player.status_label.as_str()));
    status.add_theme_font_size_override("font_size", if featured { 18 } else { 15 });
    header.add_child(&status);

    let mut total = Label::new_alloc();
    total.set_text(&GString::from(format!("{}", player.total_score).as_str()));
    total.add_theme_font_size_override("font_size", if featured { 20 } else { 17 });
    header.add_child(&total);
    wrapper.add_child(&header);

    let mut frames = HBoxContainer::new_alloc();
    frames.set("size_flags_horizontal", &3.to_variant());
    frames.add_theme_constant_override("separation", if featured { 6 } else { 4 });
    for (idx, frame) in player.frames.iter().enumerate() {
        let mut frame_box = VBoxContainer::new_alloc();
        frame_box.set("size_flags_horizontal", &3.to_variant());
        frame_box.set_custom_minimum_size(Vector2::new(if featured { 36.0 } else { 30.0 }, 0.0));
        frame_box.add_theme_constant_override("separation", 2);

        let mut frame_label = Label::new_alloc();
        frame_label.set_text(&GString::from(format!("{}", idx + 1).as_str()));
        frame_label.set_horizontal_alignment(HorizontalAlignment::CENTER);
        frame_label.add_theme_font_size_override("font_size", if featured { 13 } else { 11 });
        frame_box.add_child(&frame_label);

        let mut rolls_label = Label::new_alloc();
        let rolls_text = if frame.rolls.is_empty() {
            String::from(" ")
        } else {
            frame.rolls.join(" ")
        };
        rolls_label.set_text(&GString::from(rolls_text.as_str()));
        rolls_label.set_horizontal_alignment(HorizontalAlignment::CENTER);
        rolls_label.add_theme_font_size_override("font_size", if featured { 15 } else { 12 });
        frame_box.add_child(&rolls_label);

        let mut score_label = Label::new_alloc();
        let score_text = frame
            .cumulative_score
            .map(|score| score.to_string())
            .unwrap_or_default();
        score_label.set_text(&GString::from(score_text.as_str()));
        score_label.set_horizontal_alignment(HorizontalAlignment::CENTER);
        score_label.add_theme_font_size_override("font_size", if featured { 15 } else { 12 });
        frame_box.add_child(&score_label);

        frames.add_child(&frame_box);
    }
    wrapper.add_child(&frames);
    root.add_child(&panel);
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

pub fn render_info_text(state: &mut GameState, text: &str) {
    let mut info_label = state
        .base_mut()
        .get_node_as::<Label>("UiManager/Info/VBoxContainer/Label");
    if text.is_empty() {
        info_label.set_text("Loading...");
    } else {
        info_label.set_text(&GString::from(text));
    }
}

pub fn update_lobby_ui(state: &mut GameState, code: &str, players: &[PlayerInfo]) {
    let mut room_code = state.base_mut().get_node_as::<Label>(
        "UiManager/CenterContainer/VBoxContainer/DesktopHost/PanelContainer/MarginContainer/VBoxContainer/RoomCodeValue",
    );
    room_code.set_text(&GString::from(code));

    let mut players_box = state.base_mut().get_node_as::<VBoxContainer>(
        "UiManager/CenterContainer/VBoxContainer/DesktopHost/PanelContainer/MarginContainer/VBoxContainer/Players",
    );
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
