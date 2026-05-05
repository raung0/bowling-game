mod game_state;
mod ui_manager;

use godot::prelude::*;

struct GDExt;

#[gdextension]
unsafe impl ExtensionLibrary for GDExt {}
