use getset::Getters;
use godot::{
    classes::{Button, Label, Node},
    prelude::*,
};

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
        let is_mobile = false;

        Self {
            base,
            is_mobile,
            accel: Vector3::ZERO,
            screen: Screen::default(),
        }
    }

    fn ready(&mut self) {
        self.base_mut().set_process(true);

        let mut join_button = self
            .base()
            .get_node_as::<Button>("UiManager/Mobile/VBoxContainer/Join");
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
        self.accel = self.browser_accel();
        self.is_mobile = self.is_mobile_web();

        let mut accel_label = self
            .base_mut()
            .get_node_as::<Label>("UiManager/Mobile/VBoxContainer/Accel");
        accel_label.set_text(&format!(
            "Accel: x={:.2} y={:.2} z={:.2}",
            self.accel.x, self.accel.y, self.accel.z
        ));

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

    #[cfg(target_arch = "wasm32")]
    fn browser_accel(&self) -> Vector3 {
        let bridge = self.base().get_node_or_null("WebBridge");
        let Some(mut bridge) = bridge else {
            return Vector3::ZERO;
        };

        let value = bridge.call("get_accelerometer", &[]);
        value.try_to::<Vector3>().unwrap_or(Vector3::ZERO)
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn browser_accel(&self) -> Vector3 {
        Vector3::ZERO
    }

    #[cfg(target_arch = "wasm32")]
    fn is_mobile_web(&self) -> bool {
        let bridge = self.base().get_node_or_null("WebBridge");
        let Some(mut bridge) = bridge else {
            return false;
        };

        let value = bridge.call("is_mobile", &[]);
        value.try_to::<bool>().unwrap_or(false)
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn is_mobile_web(&self) -> bool {
        false
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
