use godot::{classes::RigidBody3D, prelude::*};

#[derive(GodotClass)]
#[class(base=Node3D)]
pub struct Ball {
    base: Base<Node3D>,

    #[export]
    track_start: NodePath,

    #[export]
    track_end: NodePath,

    #[export]
    speed: f32,

    #[export]
    reset_when_past_end: bool,

    launched: bool,
}

#[godot_api]
impl INode3D for Ball {
    fn init(base: Base<Node3D>) -> Self {
        Self {
            base,
            track_start: NodePath::default(),
            track_end: NodePath::default(),
            speed: 5.0,
            reset_when_past_end: true,
            launched: false,
        }
    }

    fn ready(&mut self) {
        self.reset_ball();
    }

    fn physics_process(&mut self, _delta: f64) {
        if !self.launched {
            return;
        }

        let Some(end) = self.track_end_node() else {
            return;
        };

        let rb = self.rigidbody();
        let ball_x = rb.get_global_position().x;
        let end_x = end.get_global_position().x;

        if self.reset_when_past_end && ball_x > end_x {
            self.launched = false;
            self.reset_ball();
        }
    }
}

#[godot_api]
impl Ball {
    fn rigidbody(&self) -> Gd<RigidBody3D> {
        self.base().get_node_as::<RigidBody3D>("RigidBody3D")
    }

    fn track_start_node(&self) -> Option<Gd<Node3D>> {
        if self.track_start.is_empty() {
            return None;
        }
        Some(self.base().get_node_as::<Node3D>(&self.track_start))
    }

    fn track_end_node(&self) -> Option<Gd<Node3D>> {
        if self.track_end.is_empty() {
            return None;
        }
        Some(self.base().get_node_as::<Node3D>(&self.track_end))
    }

    #[func]
    pub fn launch_throw(&mut self, force: f32, direction_x: f32, direction_z: f32) {
        self.launched = true;

        let force = force.clamp(0.0, 1.0);
        let direction = Vector2::new(direction_x, direction_z).normalized();
        let forward = direction.y.max(0.35);
        let lateral = direction.x.clamp(-0.85, 0.85);
        let speed = 1.5 + self.speed * force;
        let mut rb = self.rigidbody();

        rb.set_linear_velocity(Vector3::new(speed * forward, 0.0, speed * lateral * 0.35));
        rb.set_angular_velocity(Vector3::new(0.0, speed * lateral * 0.6, -speed));
        rb.set_sleeping(false);
    }

    #[func]
    pub fn reset_ball(&mut self) {
        self.launched = false;

        let Some(start) = self.track_start_node() else {
            return;
        };

        let pos = start.get_global_position();
        let mut rb = self.rigidbody();

        rb.set_global_position(pos);
        rb.set_linear_velocity(Vector3::ZERO);
        rb.set_angular_velocity(Vector3::ZERO);
        rb.set_sleeping(false);
    }

    #[func]
    pub fn track_progress(&self) -> f32 {
        let Some(start) = self.track_start_node() else {
            return 0.0;
        };

        let Some(end) = self.track_end_node() else {
            return 0.0;
        };

        let rb = self.rigidbody();

        let start_x = start.get_global_position().x;
        let end_x = end.get_global_position().x;
        let ball_x = rb.get_global_position().x;

        ((ball_x - start_x) / (end_x - start_x)).clamp(0.0, 1.0)
    }

    #[func]
    pub fn is_launched(&self) -> bool {
        self.launched
    }
}
