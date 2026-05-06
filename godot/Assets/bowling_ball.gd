extends Node3D

@onready var body: RigidBody3D = $RigidBody3D

func _ready():
	body.angular_velocity = Vector3(0, 0, -100.0)
	body.linear_velocity = Vector3(2.5, 0, 0)
