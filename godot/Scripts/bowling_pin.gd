extends Node3D

signal fell_over(pin)

@export var pin_index: int = 0

var fallen := false


func _ready():
	rotation.y = randf_range(0.0, TAU)


func on_pin_fell():
	if fallen: return
	fallen = true
	emit_signal("fell_over", self)
