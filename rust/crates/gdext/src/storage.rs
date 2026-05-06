use godot::prelude::*;

use crate::game_state::GameState;

pub const STORAGE_ROLE: &str = "session_role";
pub const STORAGE_TOKEN: &str = "session_token";
pub const STORAGE_CODE: &str = "lobby_code";
pub const STORAGE_USERNAME: &str = "username";
pub const STORAGE_CALIBRATION_X: &str = "calibration_x";

pub fn get_value(state: &GameState, key: &str) -> String {
    let Some(mut bridge) = state.base().get_node_or_null("WebBridge") else {
        return String::new();
    };
    bridge
        .call("get_local_value", &[key.to_variant()])
        .try_to::<GString>()
        .unwrap_or_default()
        .to_string()
}

pub fn set_value(state: &GameState, key: &str, value: &str) {
    let Some(mut bridge) = state.base().get_node_or_null("WebBridge") else {
        return;
    };
    bridge.call(
        "set_local_value",
        &[key.to_variant(), GString::from(value).to_variant()],
    );
}

pub fn clear_value(state: &GameState, key: &str) {
    let Some(mut bridge) = state.base().get_node_or_null("WebBridge") else {
        return;
    };
    bridge.call("clear_local_value", &[key.to_variant()]);
}
