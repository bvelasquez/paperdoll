//! Per-keyframe joint editing: base pose + delta overrides.

use crate::editor::session::euler_for_joint;
use crate::editor::symmetry::{counterpart_joint, mirror_euler};
use paperdoll_rig::{EulerDeg, JointTarget, KeyframeSpec, Pose};
use std::collections::HashMap;

/// Whether this keyframe can be joint-edited in the UI (not VRMA quaternion clips).
pub fn keyframe_joint_editing_enabled(draft: &paperdoll_rig::AnimationFile) -> bool {
    !draft.vrm_local_rotations
}

pub fn keyframe_has_pose_ref(kf: &KeyframeSpec) -> bool {
    kf.pose.is_some()
}

pub fn keyframe_delta_count(kf: &KeyframeSpec) -> usize {
    kf.joints.as_ref().map(|j| j.len()).unwrap_or(0)
}

pub fn keyframe_joint_modified(kf: &KeyframeSpec, joint: &str) -> bool {
    kf.joints
        .as_ref()
        .is_some_and(|j| j.contains_key(joint))
}

/// Effective euler for the inspector: delta if present, else base pose library value.
pub fn euler_for_keyframe_joint(
    kf: &KeyframeSpec,
    poses: &HashMap<String, Pose>,
    joint: &str,
) -> EulerDeg {
    if let Some(joints) = &kf.joints {
        if let Some(target) = joints.get(joint) {
            return target.rotation_deg.unwrap_or_default();
        }
    }
    if let Some(name) = &kf.pose {
        if let Some(base) = poses.get(name) {
            return euler_for_joint(base, joint);
        }
    }
    if let Some(joints) = &kf.joints {
        if let Some(target) = joints.get(joint) {
            return target.rotation_deg.unwrap_or_default();
        }
    }
    EulerDeg::default()
}

fn ensure_joints_map(kf: &mut KeyframeSpec) -> &mut HashMap<String, JointTarget> {
    if kf.pose.is_some() {
        kf.joints.get_or_insert_with(HashMap::new)
    } else {
        kf.joints.get_or_insert_with(HashMap::new)
    }
}

pub fn set_keyframe_joint_euler(
    kf: &mut KeyframeSpec,
    _poses: &HashMap<String, Pose>,
    joint: &str,
    euler: EulerDeg,
    symmetrical: bool,
) {
    let joints = ensure_joints_map(kf);
    let entry = joints
        .entry(joint.to_string())
        .or_insert_with(|| JointTarget {
            rotation_deg: None,
            rotation_quat: None,
            translation: None,
        });
    entry.rotation_deg = Some(euler);

    if symmetrical {
        if let Some(other) = counterpart_joint(joint) {
            let mirrored = mirror_euler(euler);
            joints
                .entry(other)
                .or_insert_with(|| JointTarget {
                    rotation_deg: None,
                    rotation_quat: None,
                    translation: None,
                })
                .rotation_deg = Some(mirrored);
        }
    }
}

pub fn clear_keyframe_joint(kf: &mut KeyframeSpec, joint: &str, symmetrical: bool) {
    if let Some(joints) = kf.joints.as_mut() {
        joints.remove(joint);
        if symmetrical {
            if let Some(other) = counterpart_joint(joint) {
                joints.remove(&other);
            }
        }
        if joints.is_empty() {
            kf.joints = None;
        }
    }
}

pub fn clear_keyframe_joint_overrides(kf: &mut KeyframeSpec) {
    kf.joints = None;
}

/// Materialize pose ref + deltas into a fully inline keyframe.
pub fn bake_keyframe_inline(kf: &mut KeyframeSpec, poses: &HashMap<String, Pose>) {
    let mut merged = HashMap::new();
    if let Some(name) = &kf.pose {
        if let Some(base) = poses.get(name) {
            merged = base.joints.clone();
        }
    }
    if let Some(deltas) = &kf.joints {
        for (k, v) in deltas {
            merged.insert(k.clone(), v.clone());
        }
    }
    kf.pose = None;
    kf.joints = if merged.is_empty() { None } else { Some(merged) };
    kf.hold = None;
}
