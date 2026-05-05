window.godotMotion = {
	x: 0, y: 0, z: 0,
	supported: "DeviceMotionEvent" in window,
	active: false,

	is_mobile: false,

	async start() {
		if (!this.supported) return false;

		if (typeof DeviceMotionEvent.requestPermission === "function") {
			const res = await DeviceMotionEvent.requestPermission();
			if (res !== "granted") return false;
		}

		window.addEventListener("devicemotion", (e) => {
			const a = e.accelerationIncludingGravity ?? e.acceleration;
			if (!a) return;

			this.x = a.x ?? 0;
			this.y = a.y ?? 0;
			this.z = a.z ?? 0;
		});

		this.active = true;
		return true;
	}
};

function detectMobileDevice() {
	const ua = navigator.userAgent || "";
	const mobileUa = /Android|iPhone|iPad|iPod|IEMobile|Opera Mini|Mobile/i.test(ua);
	const coarsePointer = window.matchMedia ? window.matchMedia("(pointer: coarse)").matches : false;
	const touchCapable = (navigator.maxTouchPoints || 0) > 1;

	return mobileUa || coarsePointer || touchCapable;
}

function updateMobileFlag() {
	window.godotMotion.is_mobile = detectMobileDevice();
}

updateMobileFlag();
window.addEventListener("resize", updateMobileFlag);
window.addEventListener("orientationchange", updateMobileFlag);
