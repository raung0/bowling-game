use getset::Getters;
use godot::{classes::Button, prelude::*};

use crate::ui_manager::UiManager;

#[derive(Clone, Copy)]
pub enum Screen {
    MainMenu,
    Host,
    Game,
}

impl Default for Screen {
    fn default() -> Self {
        Screen::MainMenu
    }
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

    base: Base<Node>,
}

#[godot_api]
impl INode for GameState {
    fn init(base: Base<Node>) -> Self {
        let is_mobile = is_mobile_web();

        Self {
            base,
            is_mobile,
            accel: Vector3::ZERO,
            screen: Screen::default(),
        }
    }

    fn ready(&mut self) {
        self.base_mut().set_process(true);

        let mut join_button = self.base().get_node_as::<Button>(
            "UiManager/CenterContainer/VBoxContainer/Mobile/VBoxContainer/Join",
        );
        let mut create_button = self.base().get_node_as::<Button>(
            "UiManager/CenterContainer/VBoxContainer/Desktop/VBoxContainer/Create",
        );
        let mut spectate_button = self.base().get_node_as::<Button>(
            "UiManager/CenterContainer/VBoxContainer/Desktop/VBoxContainer/Spectate",
        );

        join_button.connect("pressed", &self.base().callable("on_join_pressed"));
        create_button.connect("pressed", &self.base().callable("on_create_pressed"));
        spectate_button.connect("pressed", &self.base().callable("on_spectate_pressed"));
    }

    fn process(&mut self, _delta: f64) {
        self.accel = browser_accel();

        let mut ui_manager = self.base_mut().get_node_as::<UiManager>("UiManager");
        ui_manager
            .bind_mut()
            .set_screen(self.screen, self.is_mobile);
    }
}

impl GameState {
    pub fn get(node: &Node) -> Gd<GameState> {
        node.get_tree_or_null()
            .unwrap()
            .get_root()
            .unwrap()
            .get_node_as::<GameState>("GameState")
    }
}

#[godot_api]
impl GameState {
    #[func]
    fn on_join_pressed(&mut self) {
        godot_print!("Join clicked!");
        self.screen = Screen::Game;
    }

    #[func]
    fn on_create_pressed(&mut self) {
        godot_print!("Create clicked!");
        self.screen = Screen::Host;
    }

    #[func]
    fn on_spectate_pressed(&mut self) {
        godot_print!("Spectate clicked!");
        self.screen = Screen::Game;
    }
}

#[cfg(target_arch = "wasm32")]
fn browser_accel() -> Vector3 {
    Vector3::ZERO
}

#[cfg(not(target_arch = "wasm32"))]
fn browser_accel() -> Vector3 {
    Vector3::ZERO
}

#[cfg(target_arch = "wasm32")]
fn is_mobile_web() -> bool {
    false
}

#[cfg(not(target_arch = "wasm32"))]
fn is_mobile_web() -> bool {
    false
}
