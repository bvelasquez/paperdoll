use glam::Vec3;
use serde::{Deserialize, Serialize};

/// Default orbit framing used by paperdoll-app's startup camera
/// (`eye ≈ (2.5, 1.6, 3.5)`, `look_at ≈ (0, 0.9, 0)`).
pub const DEFAULT_CAMERA: ResolvedCamera = ResolvedCamera {
    yaw_deg: 35.5,
    pitch_deg: 9.2,
    distance: 4.36,
    look_at: [0.0, 0.9, 0.0],
};

/// Sparse, authorable camera target for a pose or animation keyframe. Omitted fields
/// keep whatever the live camera already is when the pose/keyframe is applied (same
/// sparse spirit as joint targets). Orbit framing — yaw/pitch/distance around
/// `look_at` — rather than a raw eye position, so agents can pan/tilt/yaw/zoom
/// without doing trig.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct CameraTarget {
    /// Horizontal orbit angle in degrees around `look_at` (0 = +Z, positive toward +X).
    #[serde(default)]
    pub yaw_deg: Option<f32>,
    /// Vertical orbit angle in degrees. Positive looks down from above; clamped at
    /// apply time so the camera never flips through the poles.
    #[serde(default)]
    pub pitch_deg: Option<f32>,
    /// Orbit radius / zoom. Smaller = tighter close-up.
    #[serde(default)]
    pub distance: Option<f32>,
    /// World-space point the camera looks at. Shifting this is a pan.
    #[serde(default)]
    pub look_at: Option<[f32; 3]>,
}

/// Fully-resolved camera state used for blending and for driving the Bevy camera.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct ResolvedCamera {
    pub yaw_deg: f32,
    pub pitch_deg: f32,
    pub distance: f32,
    pub look_at: [f32; 3],
}

impl Default for ResolvedCamera {
    fn default() -> Self {
        DEFAULT_CAMERA
    }
}

impl CameraTarget {
    /// Every field set to [`DEFAULT_CAMERA`] — use on idle / baseline poses so revert
    /// and `/pose idle` restore the default stage, not the last animation camera.
    pub fn full_default_stage() -> Self {
        Self {
            yaw_deg: Some(DEFAULT_CAMERA.yaw_deg),
            pitch_deg: Some(DEFAULT_CAMERA.pitch_deg),
            distance: Some(DEFAULT_CAMERA.distance),
            look_at: Some(DEFAULT_CAMERA.look_at),
        }
    }
}

impl ResolvedCamera {
    pub fn look_at_vec(self) -> Vec3 {
        Vec3::from_array(self.look_at)
    }

    /// World-space eye position implied by this orbit framing.
    pub fn eye(self) -> Vec3 {
        let pitch = self.pitch_deg.to_radians().clamp(
            -89.0_f32.to_radians(),
            89.0_f32.to_radians(),
        );
        let yaw = self.yaw_deg.to_radians();
        let look_at = self.look_at_vec();
        let distance = self.distance.max(0.25);
        let x = distance * pitch.cos() * yaw.sin();
        let y = distance * pitch.sin();
        let z = distance * pitch.cos() * yaw.cos();
        look_at + Vec3::new(x, y, z)
    }

    /// Apply a sparse authored patch on top of this camera (omitted fields keep
    /// the current value).
    pub fn with_patch(self, patch: &CameraTarget) -> Self {
        Self {
            yaw_deg: patch.yaw_deg.unwrap_or(self.yaw_deg),
            pitch_deg: patch.pitch_deg.unwrap_or(self.pitch_deg),
            distance: patch.distance.unwrap_or(self.distance),
            look_at: patch.look_at.unwrap_or(self.look_at),
        }
    }
}

/// Linearly interpolate two resolved cameras. Yaw takes the short arc so a
/// 350→10 transition goes the 20° way, not the long way around.
pub fn blend_cameras(from: ResolvedCamera, to: ResolvedCamera, t: f32) -> ResolvedCamera {
    let t = t.clamp(0.0, 1.0);
    let yaw = lerp_angle_deg(from.yaw_deg, to.yaw_deg, t);
    let pitch = from.pitch_deg + (to.pitch_deg - from.pitch_deg) * t;
    let distance = from.distance + (to.distance - from.distance) * t;
    let look_at = [
        from.look_at[0] + (to.look_at[0] - from.look_at[0]) * t,
        from.look_at[1] + (to.look_at[1] - from.look_at[1]) * t,
        from.look_at[2] + (to.look_at[2] - from.look_at[2]) * t,
    ];
    ResolvedCamera {
        yaw_deg: yaw,
        pitch_deg: pitch,
        distance,
        look_at,
    }
}

fn lerp_angle_deg(from: f32, to: f32, t: f32) -> f32 {
    let mut delta = (to - from) % 360.0;
    if delta > 180.0 {
        delta -= 360.0;
    } else if delta < -180.0 {
        delta += 360.0;
    }
    from + delta * t
}

/// Merge two sparse camera patches (overlay wins per-field when set).
pub fn merge_camera_targets(
    base: Option<CameraTarget>,
    overlay: Option<CameraTarget>,
) -> Option<CameraTarget> {
    match (base, overlay) {
        (None, None) => None,
        (Some(b), None) => Some(b),
        (None, Some(o)) => Some(o),
        (Some(b), Some(o)) => Some(CameraTarget {
            yaw_deg: o.yaw_deg.or(b.yaw_deg),
            pitch_deg: o.pitch_deg.or(b.pitch_deg),
            distance: o.distance.or(b.distance),
            look_at: o.look_at.or(b.look_at),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_arc_yaw_blend_does_not_spin_the_long_way() {
        let from = ResolvedCamera {
            yaw_deg: 350.0,
            ..DEFAULT_CAMERA
        };
        let to = ResolvedCamera {
            yaw_deg: 10.0,
            ..DEFAULT_CAMERA
        };
        let mid = blend_cameras(from, to, 0.5);
        // Short arc mid should be around 0°, not 180°.
        let wrapped = ((mid.yaw_deg % 360.0) + 360.0) % 360.0;
        assert!(wrapped < 20.0 || wrapped > 340.0, "mid yaw was {wrapped}");
    }

    #[test]
    fn sparse_patch_keeps_unspecified_fields() {
        let base = DEFAULT_CAMERA;
        let patched = base.with_patch(&CameraTarget {
            yaw_deg: Some(90.0),
            ..Default::default()
        });
        assert_eq!(patched.yaw_deg, 90.0);
        assert_eq!(patched.pitch_deg, base.pitch_deg);
        assert_eq!(patched.distance, base.distance);
        assert_eq!(patched.look_at, base.look_at);
    }

    #[test]
    fn eye_at_default_is_roughly_startup_camera() {
        let eye = DEFAULT_CAMERA.eye();
        assert!((eye.x - 2.5).abs() < 0.15, "x={}", eye.x);
        assert!((eye.y - 1.6).abs() < 0.15, "y={}", eye.y);
        assert!((eye.z - 3.5).abs() < 0.15, "z={}", eye.z);
    }
}
