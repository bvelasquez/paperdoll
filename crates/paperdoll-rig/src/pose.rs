use crate::camera::{CameraTarget, ResolvedCamera, DEFAULT_CAMERA};
use crate::skeleton::{JointId, Skeleton};
use glam::{Quat, Vec3};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Euler rotation in degrees, the human-authorable form used in pose YAML files.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct JointTarget {
    #[serde(default)]
    pub rotation_deg: Option<EulerDeg>,
    #[serde(default)]
    pub translation: Option<[f32; 3]>,
}

/// A named, sparse set of joint targets loaded from `assets/poses/*.yaml`. Joints not
/// listed keep the skeleton's rest transform. Optional [`camera`] is likewise sparse:
/// omitted fields (or a missing `camera` block) leave the live camera alone.
///
/// `hold_joints` is set by the animation resolver for camera-only keyframes so an
/// empty `joints` map means "keep the current body" rather than "reset to T-pose".
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pose {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub joints: HashMap<String, JointTarget>,
    #[serde(default)]
    pub camera: Option<CameraTarget>,
    /// When true, blending into this pose keeps the previous joint rotations instead
    /// of treating empty `joints` as a rest-pose reset. Not authored in YAML — set by
    /// [`crate::yaml::resolve_animation`] for camera-only keyframes.
    #[serde(default, skip_serializing)]
    pub hold_joints: bool,
}

/// A [`Pose`] with joint names resolved to [`JointId`]s and rotations/translations
/// converted to runtime types, ready for interpolation. `camera` is always fully
/// resolved in live/held state so sparse authored patches have a concrete base to
/// blend against.
#[derive(Debug, Clone)]
pub struct ResolvedPose {
    pub joint_rotations: HashMap<JointId, Quat>,
    pub joint_translations: HashMap<JointId, Vec3>,
    pub camera: ResolvedCamera,
}

impl Default for ResolvedPose {
    fn default() -> Self {
        Self {
            joint_rotations: HashMap::new(),
            joint_translations: HashMap::new(),
            camera: DEFAULT_CAMERA,
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
            if let Some(rot) = target.rotation_deg {
                resolved.joint_rotations.insert(id, rot.to_quat());
            }
            if let Some(t) = target.translation {
                resolved.joint_translations.insert(id, Vec3::from(t));
            }
        }
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
                translation: None,
            },
        );
        let pose = Pose {
            name: "test".into(),
            description: None,
            joints,
            camera: None,
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
"#;
        let pose: Pose = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(pose.name, "wave");
        assert_eq!(pose.joints.len(), 2);
    }
}
