use godot::classes::{Control, IControl};
use godot::prelude::*;

use crate::game_state::GameState;

#[derive(GodotClass)]
#[class(base=Control)]
struct UiManager {
    base: Base<Control>,
}

#[godot_api]
impl IControl for UiManager {
    fn init(base: Base<Control>) -> Self {
        Self { base }
    }

    fn ready(&mut self) {
        GameState::get();
    }
}
