extends RigidBody3D

func _physics_process(_delta):
	if global_transform.basis.y.dot(Vector3.UP) < 0.85 or global_position.y < -0.1:
		get_parent().on_pin_fell()
