use godot::classes::{INode3D, Node, Node3D, PackedScene};
use godot::prelude::*;

#[derive(GodotClass, Debug)]
#[class(base=Node3D)]
pub struct GameManager {
    #[export]
    spawn_position: NodePath,

    #[export]
    all_pins_scene: Option<Gd<PackedScene>>,

    pins_root: Option<Gd<Node3D>>,
    pins_fallen: Vec<bool>,

    base: Base<Node3D>,
}

#[godot_api]
impl INode3D for GameManager {
    fn init(base: Base<Node3D>) -> Self {
        Self {
            spawn_position: NodePath::default(),
            all_pins_scene: None,
            pins_root: None,
            pins_fallen: Vec::new(),
            base,
        }
    }

    fn ready(&mut self) {
        self.spawn_pins();
    }
}

#[godot_api]
impl GameManager {
    #[func]
    pub fn spawn_pins(&mut self) {
        self.clear_pins();

        let Some(scene) = self.all_pins_scene.clone() else {
            godot_error!("AllPins scene is not assigned!");
            return;
        };

        let spawn_node = self.base().get_node_as::<Node3D>(&self.spawn_position);

        let mut pins_root = scene.instantiate_as::<Node3D>();

        pins_root.set_global_transform(spawn_node.get_global_transform());

        self.base_mut().add_child(&pins_root);

        self.pins_fallen.clear();

        for mut child in pins_root.get_children().iter_shared() {
            let pin_index = child.get("pin_index").try_to::<i32>().unwrap_or(-1);

            if pin_index < 0 {
                godot_warn!("Pin child has no valid pin_index.");
                continue;
            }

            let pin_index = pin_index as usize;

            if self.pins_fallen.len() <= pin_index {
                self.pins_fallen.resize(pin_index + 1, false);
            }

            child.connect(
                "fell_over",
                &Callable::from_object_method(
                    &self.base().clone().upcast::<Object>(),
                    "on_pin_fell_over",
                ),
            );
        }

        self.pins_root = Some(pins_root);
    }

    #[func]
    pub fn clear_pins(&mut self) {
        if let Some(mut pins_root) = self.pins_root.take() {
            pins_root.queue_free();
        }

        self.pins_fallen.clear();
        godot_print!("pins clearned, pins_fallen={:?}", self.pins_fallen);
    }

    #[func]
    fn on_pin_fell_over(&mut self, pin: Gd<Node>) {
        let pin_index = pin.get("pin_index").try_to::<i32>().unwrap_or(-1);

        if pin_index < 0 {
            godot_warn!("Fallen pin has invalid pin_index.");
            return;
        }

        let pin_index = pin_index as usize;

        if self.pins_fallen.len() <= pin_index {
            self.pins_fallen.resize(pin_index + 1, false);
        }

        self.pins_fallen[pin_index] = true;

        godot_print!("pin fell over, pins_fallen={:?}", self.pins_fallen);
    }

    #[func]
    pub fn is_pin_fallen(&self, pin_index: i32) -> bool {
        pin_index >= 0
            && self
                .pins_fallen
                .get(pin_index as usize)
                .copied()
                .unwrap_or(false)
    }

    #[func]
    pub fn fallen_count(&self) -> i32 {
        self.pins_fallen.iter().filter(|fallen| **fallen).count() as i32
    }

    #[func]
    pub fn standing_count(&self) -> i32 {
        self.pins_fallen.iter().filter(|fallen| !**fallen).count() as i32
    }
}
