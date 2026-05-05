use godot::prelude::*;

#[derive(GodotClass)]
#[class(base=Node)]
struct GameState {
    is_mobile: bool,
    accel: Vector3,

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
        }
    }

    fn process(&mut self, _delta: f64) {
        #[cfg(target_arch = "wasm32")]
        let accel = browser_accel();

        #[cfg(not(target_arch = "wasm32"))]
        let accel = Vector3::ZERO;

        self.accel = accel;
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

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

#[cfg(target_arch = "wasm32")]
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

#[cfg(target_arch = "wasm32")]
fn browser_accel() -> godot::builtin::Vector3 {
    unsafe { godot::builtin::Vector3::new(MOTION_X as f32, MOTION_Y as f32, MOTION_Z as f32) }
}
