mod ball;
mod game_manager; // I know this has a bad name but I don't have any better ideas rn lmao
mod game_state;
mod ui_manager;

use godot::prelude::*;

struct GDExt;

#[gdextension]
unsafe impl ExtensionLibrary for GDExt {}
