shader_type spatial;

render_mode unshaded, blend_mix;

uniform vec4 albedo_color : source_color = vec4(1.0, 0.0, 0.0, 1.0);

void fragment() {
	ALBEDO = albedo_color.rgb;
	ALPHA = albedo_color.a;
}

//void light() {
//	// Called for every pixel for every light affecting the material.
//	// Uncomment to replace the default light processing function with this one.
//}
