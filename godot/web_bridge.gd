extends Node

var acceleration: Vector3 = Vector3.ZERO
var mobile: bool = false

func _ready() -> void:
	if not OS.has_feature("web"):
		return

	_init_motion_js()
	_update_mobile()
	_start_motion()

func _input(_event: InputEvent) -> void:
	if not OS.has_feature("web"):
		return

	_start_motion()

func _process(_delta: float) -> void:
	if not OS.has_feature("web"):
		return

	acceleration = Vector3(
		_to_float(JavaScriptBridge.eval("window.godotMotion ? window.godotMotion.x : 0", true)),
		_to_float(JavaScriptBridge.eval("window.godotMotion ? window.godotMotion.y : 0", true)),
		_to_float(JavaScriptBridge.eval("window.godotMotion ? window.godotMotion.z : 0", true))
	)
	_update_mobile()

func get_accelerometer() -> Vector3:
	return acceleration

func is_mobile() -> bool:
	return mobile

func _start_motion() -> void:
	JavaScriptBridge.eval("window.godotMotion && window.godotMotion.start && window.godotMotion.start();", true)

func _update_mobile() -> void:
	mobile = bool(JavaScriptBridge.eval("window.godotMotion && window.godotMotion.is_mobile ? true : false", true))

func _init_motion_js() -> void:
	JavaScriptBridge.eval("""
if (!window.godotMotion) {
  window.godotMotion = { x: 0, y: 0, z: 0, is_mobile: false, active: false };
}

function detectMobileDevice() {
  const ua = navigator.userAgent || '';
  const mobileUa = /Android|iPhone|iPad|iPod|IEMobile|Opera Mini|Mobile/i.test(ua);
  const coarsePointer = window.matchMedia ? window.matchMedia('(pointer: coarse)').matches : false;
  const touchCapable = (navigator.maxTouchPoints || 0) > 1;
  return mobileUa || coarsePointer || touchCapable;
}

function updateMobileFlag() {
  window.godotMotion.is_mobile = detectMobileDevice();
}

function registerMotionListener() {
  if (window.godotMotion.active) return;
  window.addEventListener('devicemotion', function (event) {
    const a = event.accelerationIncludingGravity || event.acceleration;
    if (!a) return;
    window.godotMotion.x = Number.isFinite(a.x) ? a.x : 0;
    window.godotMotion.y = Number.isFinite(a.y) ? a.y : 0;
    window.godotMotion.z = Number.isFinite(a.z) ? a.z : 0;
  });
  window.godotMotion.active = true;
}

window.godotMotion.start = async function () {
  try {
    if (typeof DeviceMotionEvent !== 'undefined' && typeof DeviceMotionEvent.requestPermission === 'function') {
      const state = await DeviceMotionEvent.requestPermission();
      if (state !== 'granted') return false;
    }
    registerMotionListener();
    return true;
  } catch (_err) {
    return false;
  }
};

updateMobileFlag();
window.addEventListener('resize', updateMobileFlag);
window.addEventListener('orientationchange', updateMobileFlag);
""", true)

func _to_float(value: Variant) -> float:
	if value == null:
		return 0.0
	if value is float:
		return value
	if value is int:
		return float(value)
	return 0.0
