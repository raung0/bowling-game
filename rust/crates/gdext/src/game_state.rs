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
        #[cfg(target_arch = "wasm32")]
        let is_mobile = IS_MOBILE;

        #[cfg(not(target_arch = "wasm32"))]
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
        #[cfg(target_arch = "wasm32")]
        let accel = WASM::browser_accel();

        #[cfg(not(target_arch = "wasm32"))]
        let accel = Vector3::ZERO;

        self.accel = accel;

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
mod WASM {
    use wasm_bindgen::prelude::*;

    #[wasm_bindgen]
    extern "C" {
        #[wasm_bindgen(js_namespace = ["window", "godotMotion"], js_name = x)]
        static MOTION_X: f64;

        #[wasm_bindgen(js_namespace = ["window", "godotMotion"], js_name = y)]
        static MOTION_Y: f64;

        #[wasm_bindgen(js_namespace = ["window", "godotMotion"], js_name = z)]
        static MOTION_Z: f64;

        #[wasm_bindgen(js_namespace = ["window", "godotMotion"], js_name = is_mobile)]
        static IS_MOBILE: bool;
    }

    fn browser_accel() -> godot::builtin::Vector3 {
        unsafe { godot::builtin::Vector3::new(MOTION_X as f32, MOTION_Y as f32, MOTION_Z as f32) }
    }
}
