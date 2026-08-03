//! Bilateral joint naming and euler mirroring for the pose editor.

use paperdoll_rig::{EulerDeg, JointTarget, Pose};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BodySide {
    Left,
    Right,
}

impl BodySide {
    pub fn prefix(self) -> &'static str {
        match self {
            BodySide::Left => "left_",
            BodySide::Right => "right_",
        }
    }

    pub fn opposite(self) -> Self {
        match self {
            BodySide::Left => BodySide::Right,
            BodySide::Right => BodySide::Left,
        }
    }
}

/// Infer side from a joint name (`left_shoulder` → Left).
pub fn side_from_joint(name: &str) -> Option<BodySide> {
    if let Some(rest) = name.strip_prefix("left_") {
        if !rest.is_empty() {
            return Some(BodySide::Left);
        }
    }
    if let Some(rest) = name.strip_prefix("right_") {
        if !rest.is_empty() {
            return Some(BodySide::Right);
        }
    }
    None
}

/// Swap `left_` ↔ `right_` on a bilateral joint name.
pub fn counterpart_joint(name: &str) -> Option<String> {
    if let Some(rest) = name.strip_prefix("left_") {
        return Some(format!("right_{rest}"));
    }
    if let Some(rest) = name.strip_prefix("right_") {
        return Some(format!("left_{rest}"));
    }
    None
}

/// Mirror euler across the character's sagittal plane (Y-up, facing +Z).
pub fn mirror_euler(e: EulerDeg) -> EulerDeg {
    EulerDeg {
        x: e.x,
        y: -e.y,
        z: -e.z,
    }
}

pub fn mirror_joint_target(t: &JointTarget) -> JointTarget {
    JointTarget {
        rotation_deg: t.rotation_deg.map(mirror_euler),
        rotation_quat: t.rotation_quat,
        translation: t.translation.map(|[x, y, z]| [-x, y, z]),
    }
}

/// Write `euler` on `joint`, optionally mirroring to the opposite side.
pub fn set_joint_euler_with_symmetry(
    pose: &mut Pose,
    joint: &str,
    euler: EulerDeg,
    symmetrical: bool,
) {
    pose.joints
        .entry(joint.to_string())
        .or_default()
        .rotation_deg = Some(euler);

    if symmetrical {
        if let Some(other) = counterpart_joint(joint) {
            pose.joints
                .entry(other)
                .or_default()
                .rotation_deg = Some(mirror_euler(euler));
        }
    }
}

/// Copy a sparse joint map authored for one side onto the other (mirrored).
pub fn mirror_joint_map_to_other_side(map: &HashMap<String, JointTarget>) -> HashMap<String, JointTarget> {
    let mut out = HashMap::new();
    for (name, target) in map {
        if let Some(other) = counterpart_joint(name) {
            out.insert(other, mirror_joint_target(target));
        }
    }
    out
}

/// Remove every joint key on `side` that appears in `suffixes` (e.g. finger chains).
pub fn clear_side_joints(pose: &mut Pose, side: BodySide, suffixes: &[&str]) {
    let prefix = side.prefix();
    pose.joints.retain(|name, _| {
        if !name.starts_with(prefix) {
            return true;
        }
        let rest = &name[prefix.len()..];
        !suffixes.iter().any(|s| rest == *s || rest.starts_with(&format!("{s}_")))
    });
}
