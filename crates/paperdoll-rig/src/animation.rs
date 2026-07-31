use crate::camera::CameraTarget;
use crate::pose::{JointTarget, Pose};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Easing curve applied to a keyframe segment's linear progress before it drives
/// interpolation, so motion doesn't feel robotic.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Easing {
    #[default]
    Linear,
    EaseIn,
    EaseOut,
    EaseInOut,
    Step,
}

impl Easing {
    /// Every variant, in declaration order. Used by callers that need to enumerate
    /// valid easing values (e.g. the HTTP API's `GET /capabilities`, which serializes
    /// each one to get its exact `snake_case` wire name instead of hand-duplicating
    /// the string list and risking it drifting from this enum).
    pub const ALL: [Easing; 5] = [
        Easing::Linear,
        Easing::EaseIn,
        Easing::EaseOut,
        Easing::EaseInOut,
        Easing::Step,
    ];

    pub fn apply(self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        match self {
            Easing::Linear => t,
            Easing::EaseIn => t * t,
            Easing::EaseOut => t * (2.0 - t),
            Easing::EaseInOut => t * t * (3.0 - 2.0 * t), // smoothstep
            Easing::Step => {
                if t >= 1.0 {
                    1.0
                } else {
                    0.0
                }
            }
        }
    }
}

/// One keyframe as authored in an `assets/animations/*.yaml` file: either a reference
/// to a named pose, an inline set of joint targets, and/or a camera patch. Camera may
/// appear alongside a pose/joints reference (orbit while holding a pose) or alone
/// (camera-only move; joints stay wherever they are via an empty pose).
///
/// `hold: true` makes the keyframe a sparse OVERLAY: only its listed joints move,
/// and every unlisted joint keeps its current value instead of resetting to rest —
/// this is how a blink or a gaze shift can ride on top of an ongoing body pose.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyframeSpec {
    #[serde(default)]
    pub pose: Option<String>,
    #[serde(default)]
    pub joints: Option<HashMap<String, JointTarget>>,
    #[serde(default)]
    pub camera: Option<CameraTarget>,
    #[serde(default)]
    pub hold: Option<bool>,
    #[serde(default)]
    pub duration_ms: u32,
    #[serde(default)]
    pub easing: Easing,
}

/// The raw shape of an `assets/animations/*.yaml` file, before pose references are
/// resolved against a loaded pose registry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnimationFile {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default, rename = "loop")]
    pub looping: bool,
    pub keyframes: Vec<KeyframeSpec>,
}

/// A keyframe with its pose fully resolved (name reference or inline joints already
/// materialized into a concrete [`Pose`]).
#[derive(Debug, Clone)]
pub struct Keyframe {
    pub pose: Pose,
    pub duration_ms: u32,
    pub easing: Easing,
}

/// An ordered sequence of resolved keyframes, ready to be played by
/// [`crate::interpolation::PlaybackState`].
#[derive(Debug, Clone)]
pub struct Animation {
    pub name: String,
    pub description: Option<String>,
    pub looping: bool,
    pub keyframes: Vec<Keyframe>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn easing_endpoints_are_stable() {
        for easing in [
            Easing::Linear,
            Easing::EaseIn,
            Easing::EaseOut,
            Easing::EaseInOut,
            Easing::Step,
        ] {
            assert_eq!(easing.apply(0.0), 0.0, "{easing:?} at t=0");
            assert_eq!(easing.apply(1.0), 1.0, "{easing:?} at t=1");
        }
    }

    #[test]
    fn easing_clamps_out_of_range_input() {
        assert_eq!(Easing::Linear.apply(-1.0), 0.0);
        assert_eq!(Easing::Linear.apply(2.0), 1.0);
    }

    #[test]
    fn deserializes_animation_file_from_yaml() {
        let yaml = r#"
name: wave_animation
description: "Raise arm, wave twice, lower arm"
loop: false
keyframes:
  - pose: t_pose
    duration_ms: 0
    easing: linear
  - pose: wave
    duration_ms: 500
    easing: ease_in_out
  - pose: t_pose
    duration_ms: 600
    easing: ease_out
"#;
        let file: AnimationFile = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(file.name, "wave_animation");
        assert_eq!(file.keyframes.len(), 3);
        assert_eq!(file.keyframes[1].easing, Easing::EaseInOut);
        assert!(!file.looping);
    }
}
