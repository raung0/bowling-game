mod ball;
mod controller;
mod game_manager;
mod game_state;
mod rendering;
mod storage;
mod ui_manager;

use godot::prelude::*;

struct GDExt;

#[gdextension]
unsafe impl ExtensionLibrary for GDExt {}
