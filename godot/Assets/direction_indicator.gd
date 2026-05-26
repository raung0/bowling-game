extends Node3D

@export var path: Path3D
@export var indicator_scene: PackedScene
@export var spacing := 1.0
@export var offset := 0.0

func _ready():
	_spawn_indicators()

func _spawn_indicators():
	if path == null or indicator_scene == null:
		return

	var curve := path.curve
	if curve == null:
		return

	var length := curve.get_baked_length()
	var distance := offset

	while distance < length:
		var indicator := indicator_scene.instantiate() as MeshInstance3D
		add_child(indicator)

		var pos := curve.sample_baked(distance)
		var next_pos := curve.sample_baked(min(distance + 0.1, length))

		indicator.global_position = path.global_transform * pos
		indicator.look_at(path.global_transform * next_pos, Vector3.UP)

		distance += spacing
