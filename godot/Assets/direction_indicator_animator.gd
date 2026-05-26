extends MeshInstance3D

@export var fade_curve: Curve
@export var duration := 1.0
@export var shader_param := "albedo_color"

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

	var t := time / duration
	var alpha := fade_curve.sample(t)

	var color: Color = mat.get_shader_parameter(shader_param)
	color.a = alpha

	mat.set_shader_parameter(shader_param, color)
