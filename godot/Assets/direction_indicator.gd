extends Node3D

@export var path: Path3D
@export var indicator_scene: PackedScene
@export var spacing := 1.0
@export var offset := 0.0

const POINT_EPSILON := 0.001

var _has_points := false
var _last_start := Vector3.ZERO
var _last_finish := Vector3.ZERO

func _ready():
	_sync_indicators()

func set_points(start: Vector3, finish: Vector3) -> void:
	if path == null:
		return

	var points_changed := (not _has_points) \
		or _last_start.distance_to(start) > POINT_EPSILON \
		or _last_finish.distance_to(finish) > POINT_EPSILON

	if not points_changed:
		return

	_has_points = true
	_last_start = start
	_last_finish = finish

	var curve := path.curve
	if curve == null:
		curve = Curve3D.new()
		path.curve = curve

	curve.clear_points()
	curve.add_point(start)
	curve.add_point(finish)
	_sync_indicators()

func set_fade(progress: float) -> void:
	var alpha: float = clampf(1.0 - progress, 0.0, 1.0)
	for child in get_children():
		var mesh := child as MeshInstance3D
		if mesh == null:
			continue
		if mesh.has_method("set_external_fade"):
			mesh.call("set_external_fade", alpha)

func _sync_indicators() -> void:
	if path == null or indicator_scene == null:
		return

	var curve := path.curve
	if curve == null:
		return

	var length := curve.get_baked_length()
	var target_count := _target_indicator_count(length)
	_ensure_indicator_count(target_count)

	var indicators := _indicator_nodes()
	var distance := offset
	for indicator in indicators:
		if distance >= length:
			break

		var pos := curve.sample_baked(distance)
		var next_pos := curve.sample_baked(min(distance + 0.1, length))

		indicator.global_position = path.global_transform * pos
		indicator.look_at(path.global_transform * next_pos, Vector3.UP)

		distance += spacing

func _target_indicator_count(length: float) -> int:
	if spacing <= 0.0 or length <= 0.0 or offset >= length:
		return 0

	var count := 0
	var distance := offset
	while distance < length:
		count += 1
		distance += spacing
	return count

func _indicator_nodes() -> Array[MeshInstance3D]:
	var indicators: Array[MeshInstance3D] = []
	for child in get_children():
		var indicator := child as MeshInstance3D
		if indicator != null:
			indicators.push_back(indicator)
	return indicators

func _ensure_indicator_count(target_count: int) -> void:
	var indicators := _indicator_nodes()
	if indicators.size() < target_count:
		for _i in range(indicators.size(), target_count):
			var indicator := indicator_scene.instantiate() as MeshInstance3D
			indicator.top_level = true
			add_child(indicator)
	elif indicators.size() > target_count:
		for i in range(target_count, indicators.size()):
			indicators[i].queue_free()
