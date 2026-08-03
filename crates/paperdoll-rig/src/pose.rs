use crate::camera::{CameraTarget, ResolvedCamera, DEFAULT_CAMERA};
use crate::skeleton::{JointId, Skeleton};
use glam::{Quat, Vec3};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Euler rotation in degrees, the human-authorable form used in pose YAML files.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct EulerDeg {
    #[serde(default)]
    pub x: f32,
    #[serde(default)]
    pub y: f32,
    #[serde(default)]
    pub z: f32,
}

impl EulerDeg {
    pub fn to_quat(self) -> Quat {
        Quat::from_euler(
            glam::EulerRot::XYZ,
            self.x.to_radians(),
            self.y.to_radians(),
            self.z.to_radians(),
        )
    }

    pub fn from_quat(q: Quat) -> Self {
        let (x, y, z) = q.to_euler(glam::EulerRot::XYZ);
        Self {
            x: x.to_degrees(),
            y: y.to_degrees(),
            z: z.to_degrees(),
        }
    }
}

/// A single joint's authored target within a [`Pose`]. Either field may be omitted;
/// an omitted field means "leave this joint's rest value alone."
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct JointTarget {
    #[serde(default)]
    pub rotation_deg: Option<EulerDeg>,
    /// glTF/VRMA local rotation (xyzw). Preferred over `rotation_deg` when set (VRMA import).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rotation_quat: Option<[f32; 4]>,
    #[serde(default)]
    pub translation: Option<[f32; 3]>,
}

/// A named, sparse set of joint targets loaded from `assets/poses/*.yaml`. Joints not
/// listed keep the skeleton's rest transform. Optional [`camera`] is likewise sparse:
/// omitted fields (or a missing `camera` block) leave the live camera alone.
///
/// Optional [`expressions`] are VRM face morph preset weights (v2). Keys are preset
/// names (`happy`, `blink`, …); values are in `[0, 1]`. Unlisted presets blend toward
/// 0 unless `hold_joints` is set (sparse overlay — see animation keyframes).
///
/// `hold_joints` is set by the animation resolver for camera-only / expression-only
/// keyframes so an empty `joints` map means "keep the current body" rather than
/// "reset to T-pose".
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Pose {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub joints: HashMap<String, JointTarget>,
    #[serde(default)]
    pub camera: Option<CameraTarget>,
    /// VRM expression preset weights (v2). Soft-validated at HTTP registration time
    /// against the live catalog; the rig accepts any string keys.
    #[serde(default)]
    pub expressions: HashMap<String, f32>,
    /// When true, blending into this pose keeps the previous joint rotations instead
    /// of treating empty `joints` as a rest-pose reset. Not authored in YAML — set by
    /// [`crate::yaml::resolve_animation`] for camera-only keyframes.
    #[serde(default, skip_serializing)]
    pub hold_joints: bool,
}

/// A [`Pose`] with joint names resolved to [`JointId`]s and rotations/translations
/// converted to runtime types, ready for interpolation. `camera` is always fully
/// resolved in live/held state so sparse authored patches have a concrete base to
/// blend against. `expressions` carries VRM morph weights through the same blend.
#[derive(Debug, Clone)]
pub struct ResolvedPose {
    pub joint_rotations: HashMap<JointId, Quat>,
    pub joint_translations: HashMap<JointId, Vec3>,
    pub camera: ResolvedCamera,
    pub expressions: HashMap<String, f32>,
}

impl Default for ResolvedPose {
    fn default() -> Self {
        Self {
            joint_rotations: HashMap::new(),
            joint_translations: HashMap::new(),
            camera: DEFAULT_CAMERA,
            expressions: HashMap::new(),
        }
    }
}

impl ResolvedPose {
    pub fn empty() -> Self {
        Self::default()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum PoseError {
    #[error("pose references unknown joint '{0}'")]
    UnknownJoint(String),
}

impl Pose {
    pub fn resolve(&self, skeleton: &Skeleton) -> Result<ResolvedPose, PoseError> {
        let mut resolved = ResolvedPose::empty();
        for (name, target) in &self.joints {
            let id = skeleton
                .joint_by_name(name)
                .ok_or_else(|| PoseError::UnknownJoint(name.clone()))?;
            if let Some(q) = target.rotation_quat {
                resolved
                    .joint_rotations
                    .insert(id, Quat::from_xyzw(q[0], q[1], q[2], q[3]).normalize());
            } else if let Some(rot) = target.rotation_deg {
                resolved.joint_rotations.insert(id, rot.to_quat());
            }
            if let Some(t) = target.translation {
                resolved.joint_translations.insert(id, Vec3::from(t));
            }
        }
        if let Some(patch) = &self.camera {
            resolved.camera = resolved.camera.with_patch(patch);
        }
        resolved.expressions = self
            .expressions
            .iter()
            .map(|(k, v)| (k.clone(), (*v).clamp(0.0, 1.0)))
            .collect();
        Ok(resolved)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_maps_joint_names_to_ids_and_converts_degrees_to_radians() {
        let skeleton = Skeleton::humanoid_default();
        let mut joints = HashMap::new();
        joints.insert(
            "right_shoulder".to_string(),
            JointTarget {
                rotation_deg: Some(EulerDeg {
                    x: 0.0,
                    y: 0.0,
                    z: -90.0,
                }),
                rotation_quat: None,
                translation: None,
            },
        );
        let pose = Pose {
            name: "test".into(),
            description: None,
            joints,
            camera: None,
            expressions: HashMap::new(),
            hold_joints: false,
        };

        let resolved = pose.resolve(&skeleton).unwrap();
        let id = skeleton.joint_by_name("right_shoulder").unwrap();
        let rot = resolved.joint_rotations.get(&id).unwrap();
        let expected = Quat::from_rotation_z(-std::f32::consts::FRAC_PI_2);
        // angle_between loses precision near-parallel quaternions (acos'(1) blows up),
        // so use a tolerance well above float noise rather than 1e-4.
        assert!(rot.angle_between(expected) < 1e-2);
    }

    #[test]
    fn resolve_rejects_unknown_joint_names() {
        let skeleton = Skeleton::humanoid_default();
        let mut joints = HashMap::new();
        joints.insert("not_a_real_joint".to_string(), JointTarget::default());
        let pose = Pose {
            name: "bad".into(),
            description: None,
            joints,
            camera: None,
            expressions: HashMap::new(),
            hold_joints: false,
        };
        assert!(matches!(
            pose.resolve(&skeleton),
            Err(PoseError::UnknownJoint(_))
        ));
    }

    #[test]
    fn deserializes_from_yaml() {
        let yaml = r#"
name: wave
description: "Right arm raised, waving"
joints:
  right_shoulder:
    rotation_deg: { x: 0.0, y: 0.0, z: -80.0 }
  right_elbow:
    rotation_deg: { x: -30.0, y: 0.0, z: 0.0 }
expressions:
  happy: 0.8
"#;
        let pose: Pose = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(pose.name, "wave");
        assert_eq!(pose.joints.len(), 2);
        assert!((pose.expressions["happy"] - 0.8).abs() < 1e-6);
    }

    #[test]
    fn resolve_clamps_expression_weights() {
        let skeleton = Skeleton::humanoid_default();
        let mut expressions = HashMap::new();
        expressions.insert("happy".into(), 1.5);
        expressions.insert("blink".into(), -0.2);
        let pose = Pose {
            name: "face".into(),
            description: None,
            joints: HashMap::new(),
            camera: None,
            expressions,
            hold_joints: false,
        };
        let resolved = pose.resolve(&skeleton).unwrap();
        assert!((resolved.expressions["happy"] - 1.0).abs() < 1e-6);
        assert!((resolved.expressions["blink"] - 0.0).abs() < 1e-6);
    }
}
