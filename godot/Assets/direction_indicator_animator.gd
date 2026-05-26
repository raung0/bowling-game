extends MeshInstance3D

@export var fade_curve: Curve
@export var duration := 1.0
@export var shader_param := "albedo_color"
@export_range(0.0, 1.0, 0.01) var external_fade := 1.0

var time := 0.0
var mat: ShaderMaterial

func _ready():
	mat = get_active_material(0) as ShaderMaterial
	#print(mat)

func _process(delta):
	if mat == null or fade_curve == null:
		#print("this shant happen")
		return

	time = fmod(time + delta, duration)

	var t: float = time / duration
	var alpha: float = fade_curve.sample(t) * external_fade

	var color: Color = mat.get_shader_parameter(shader_param)
	color.a = alpha

	mat.set_shader_parameter(shader_param, color)

func set_external_fade(value: float) -> void:
	external_fade = clampf(value, 0.0, 1.0)
