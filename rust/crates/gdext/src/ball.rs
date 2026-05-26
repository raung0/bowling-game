use godot::{
    classes::{IRigidBody3D, RigidBody3D},
    prelude::*,
};

#[derive(GodotClass)]
#[class(base=RigidBody3D)]
pub struct Ball {
    base: Base<RigidBody3D>,

    #[export]
    track_start: NodePath,

    #[export]
    track_end: NodePath,

    #[export]
    reset_when_past_end: bool,

    launched: bool,
    shot_complete: bool,
}

#[godot_api]
impl IRigidBody3D for Ball {
    fn init(base: Base<RigidBody3D>) -> Self {
        Self {
            base,
            track_start: NodePath::default(),
            track_end: NodePath::default(),
            reset_when_past_end: false,
            launched: false,
            shot_complete: false,
        }
    }

    fn ready(&mut self) {
        self.base_mut().set_freeze_enabled(true);
    }

    fn physics_process(&mut self, _delta: f64) {
        if !self.launched {
            if self.shot_complete {
                return;
            }

            if let Some(start) = self.track_start_node() {
                let mut rb = self.base_mut();
                rb.set_global_transform(start.get_global_transform());
                rb.set_linear_velocity(Vector3::ZERO);
                rb.set_angular_velocity(Vector3::ZERO);
                rb.set_freeze_enabled(true);
            }

            return;
        }

        let Some(end) = self.track_end_node() else {
            return;
        };

        let reset_when_past_end = self.reset_when_past_end;

        let should_reset = {
            let mut rb = self.base_mut();
            let mut velocity = rb.get_linear_velocity();
            let position = rb.get_global_position();

            // Let the release spin gradually bend the ball once it is back down near the lane.
            if position.y <= 0.25 && velocity.y <= 0.0 && velocity.x > 0.1 {
                let spin = rb.get_angular_velocity().y;
                let hook = velocity.x * spin * 0.01;
                velocity.z = (velocity.z + hook).clamp(-8.0, 8.0);
                rb.set_linear_velocity(velocity);
            }

            let ball_x = position.x;
            let end_x = end.get_global_position().x;
            let horizontal_speed = Vector2::new(velocity.x, velocity.z).length();

            if reset_when_past_end {
                (ball_x > end_x) || (ball_x < end_x && horizontal_speed < 0.05)
            } else {
                false
            }
        };

        if should_reset {
            self.launched = false;
            self.base_mut().queue_free();
        }
    }
}

#[godot_api]
impl Ball {
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
        self.shot_complete = false;

        let force = force.clamp(0.0, 1.0);
        let speed = 4.1666665 + (8.333333 - 4.1666665) * force;
        let lift = direction_z.clamp(-1.0, 1.0) * 4.0;
        let spin = direction_x.clamp(-1.0, 1.0);
        let aim_direction = self
            .track_start_node()
            .map(|start| {
                let forward = start.get_global_transform().basis * Vector3::RIGHT;
                if forward.length_squared() > 0.0 {
                    Vector3::new(forward.x, 0.0, forward.z).normalized()
                } else {
                    Vector3::RIGHT
                }
            })
            .unwrap_or(Vector3::RIGHT);

        let mut rb = self.base_mut();

        rb.set_freeze_enabled(false);
        rb.set_linear_velocity(aim_direction * speed + Vector3::UP * lift);

        rb.set_angular_velocity(Vector3::new(0.0, speed * spin * 0.1, -speed));

        rb.set_sleeping(false);
    }

    #[func]
    pub fn reset_ball(&mut self) {
        self.launched = false;
        self.shot_complete = false;

        let Some(start) = self.track_start_node() else {
            return;
        };

        let mut rb = self.base_mut();

        rb.set_global_transform(start.get_global_transform());
        rb.set_linear_velocity(Vector3::ZERO);
        rb.set_angular_velocity(Vector3::ZERO);
        rb.set_freeze_enabled(true);
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

        let start_x = start.get_global_position().x;
        let end_x = end.get_global_position().x;
        let ball_x = self.base().get_global_position().x;

        ((ball_x - start_x) / (end_x - start_x)).clamp(0.0, 1.0)
    }

    #[func]
    pub fn is_launched(&self) -> bool {
        self.launched
    }

    #[func]
    pub fn finish_throw(&mut self) {
        self.launched = false;
        self.shot_complete = true;

        let mut rb = self.base_mut();
        rb.set_linear_velocity(Vector3::ZERO);
        rb.set_angular_velocity(Vector3::ZERO);
        rb.set_freeze_enabled(true);
        rb.set_sleeping(true);
    }

    #[func]
    pub fn is_settled(&self) -> bool {
        let rb = self.base();
        rb.get_linear_velocity().length() < 0.05 && rb.get_angular_velocity().length() < 0.05
    }

    #[func]
    pub fn has_passed_end(&self) -> bool {
        let Some(end) = self.track_end_node() else {
            return false;
        };

        self.base().get_global_position().x > end.get_global_position().x
    }
}
