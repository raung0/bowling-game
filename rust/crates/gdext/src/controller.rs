use godot::prelude::*;
use std::collections::VecDeque;

pub const MOTION_HISTORY_LIMIT: usize = 12;
pub const MOTION_BASELINE_SAMPLES: usize = 4;
pub const MOTION_PEAK_WINDOW_SAMPLES: usize = 3;
pub const BOWLING_SWING_MIN_FORCE: f32 = 1.6;
pub const BOWLING_SWING_MAX_ANGLE_DEG: f32 = 45.0;
pub const BOWLING_SIDEWAYS_DEADZONE: f32 = 0.08;
pub const CALIBRATION_THROW_COUNT: usize = 3;

pub fn estimate_throw_from_history(
    history: &VecDeque<Vector3>,
    baseline_accel: Vector3,
) -> (f32, Vector2) {
    if history.is_empty() {
        return (0.0, Vector2::new(0.0, 1.0));
    }

    let baseline_sample_count = history.len().clamp(1, MOTION_BASELINE_SAMPLES);
    let baseline_divisor = baseline_sample_count as f32 + 1.0;
    let baseline = history
        .iter()
        .take(baseline_sample_count)
        .copied()
        .fold(baseline_accel, |acc, sample| acc + sample)
        / baseline_divisor;

    let mut relative_samples = Vec::with_capacity(history.len());
    let mut last = None;
    for sample in history {
        let smoothed = if let Some(prev) = last {
            (*sample + prev) * 0.5
        } else {
            *sample
        };
        last = Some(*sample);
        relative_samples.push(smoothed - baseline);
    }

    let mut best_forward = 0.0_f32;
    let mut best_index = 0usize;
    for (idx, relative) in relative_samples.iter().enumerate() {
        let forward = (-relative.y).max(0.0);
        if forward > best_forward {
            best_forward = forward;
            best_index = idx;
        }
    }

    if best_forward <= 0.0 {
        return (0.0, Vector2::new(0.0, 1.0));
    }

    let window_radius = MOTION_PEAK_WINDOW_SAMPLES / 2;
    let window_start = best_index.saturating_sub(window_radius);
    let window_end = (best_index + window_radius + 1).min(relative_samples.len());
    let mut sideways_total = 0.0_f32;
    let mut sideways_count = 0usize;
    for relative in &relative_samples[window_start..window_end] {
        sideways_total += relative.z;
        sideways_count += 1;
    }
    let mut best_sideways = if sideways_count > 0 {
        sideways_total / sideways_count as f32
    } else {
        0.0
    };
    if best_sideways.abs() < BOWLING_SIDEWAYS_DEADZONE {
        best_sideways = 0.0;
    }

    let max_sideways = best_forward * BOWLING_SWING_MAX_ANGLE_DEG.to_radians().tan();
    let clamped_sideways = best_sideways.clamp(-max_sideways, max_sideways);
    let direction = Vector2::new(clamped_sideways, best_forward).normalized();
    let force = ((best_forward - BOWLING_SWING_MIN_FORCE) / 8.0).clamp(0.0, 1.0);

    (force, direction)
}

pub fn soft_deadzone(value: f32, deadzone: f32) -> f32 {
    let abs = value.abs();
    if abs <= deadzone {
        return 0.0;
    }
    value.signum() * ((abs - deadzone) / (1.0 - deadzone)).clamp(0.0, 1.0)
}
