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

/// One keyframe as authored in an `assets/animations/*.yaml` file.
///
/// **Base + delta:** `pose` names a library pose; optional `joints` are per-joint
/// overrides merged on top (listed joints win). With `hold: true`, only the listed
/// `joints` move as a sparse overlay — the base pose body is not applied.
///
/// **Fully inline:** omit `pose` and set `joints` for a standalone pose snapshot.
///
/// Camera may appear alongside pose/joints (orbit while holding a pose) or alone
/// (camera-only move; joints stay wherever they are via an empty pose).
///
/// Optional `expressions` overlay VRM morph weights onto the resolved pose (or alone
/// with `hold: true` for a face-only beat such as a blink).
///
/// `hold: true` makes the keyframe a sparse OVERLAY: only its listed joints /
/// expressions move, and every unlisted joint/expression keeps its current value
/// instead of resetting to rest / zero — this is how a blink can ride on top of an
/// ongoing body pose.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct KeyframeSpec {
    #[serde(default)]
    pub pose: Option<String>,
    #[serde(default)]
    pub joints: Option<HashMap<String, JointTarget>>,
    #[serde(default)]
    pub camera: Option<CameraTarget>,
    #[serde(default)]
    pub expressions: Option<HashMap<String, f32>>,
    #[serde(default)]
    pub hold: Option<bool>,
    #[serde(default)]
    pub duration_ms: u32,
    #[serde(default)]
    pub easing: Easing,
}

/// The raw shape of an `assets/animations/*.yaml` file, before pose references are
/// resolved against a loaded pose registry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnimationFile {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default, rename = "loop")]
    pub looping: bool,
    /// When true, joint rotations are VRM/glTF absolute local quaternions (VRMA import),
    /// not rest-relative Euler offsets used by hand-authored poses.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub vrm_local_rotations: bool,
    /// When true, the app may pick this clip at random while idle (“bored” autoplay).
    #[serde(default, rename = "play_automatically", skip_serializing_if = "std::ops::Not::not")]
    pub play_automatically: bool,
    pub keyframes: Vec<KeyframeSpec>,
}

/// A keyframe with its pose fully resolved (name reference or inline joints already
/// materialized into a concrete [`Pose`]). [`authoring`] preserves the original
/// [`KeyframeSpec`] for YAML / HTTP roundtrip (pose ref + delta joints, etc.).
#[derive(Debug, Clone)]
pub struct Keyframe {
    pub pose: Pose,
    pub duration_ms: u32,
    pub easing: Easing,
    pub authoring: KeyframeSpec,
}

impl Keyframe {
    /// Build a resolved keyframe for tests and internal callers that don't need
    /// roundtrip metadata beyond duration/easing.
    pub fn for_test(pose: Pose, duration_ms: u32, easing: Easing) -> Self {
        Self {
            authoring: KeyframeSpec {
                pose: None,
                joints: if pose.joints.is_empty() {
                    None
                } else {
                    Some(pose.joints.clone())
                },
                camera: pose.camera.clone(),
                expressions: if pose.expressions.is_empty() {
                    None
                } else {
                    Some(pose.expressions.clone())
                },
                hold: if pose.hold_joints {
                    Some(true)
                } else {
                    None
                },
                duration_ms,
                easing,
            },
            pose,
            duration_ms,
            easing,
        }
    }
}

/// An ordered sequence of resolved keyframes, ready to be played by
/// [`crate::interpolation::PlaybackState`].
#[derive(Debug, Clone)]
pub struct Animation {
    pub name: String,
    pub description: Option<String>,
    pub looping: bool,
    pub vrm_local_rotations: bool,
    pub play_automatically: bool,
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
