window.godotMotion = {
	x: 0, y: 0, z: 0,
	supported: "DeviceMotionEvent" in window,
	active: false,

	is_mobile: /Android|iPhone|iPad|iPod/i.test(navigator.userAgent) ||
	           (navigator.maxTouchPoints > 1 && window.innerWidth < 1024),

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
