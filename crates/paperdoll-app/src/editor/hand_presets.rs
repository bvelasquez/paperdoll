//! Finger / hand shape presets for the pose editor (v2 finger chains).

use super::symmetry::{mirror_joint_map_to_other_side, BodySide, clear_side_joints};
use paperdoll_rig::{CameraTarget, EulerDeg, JointTarget, Pose};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandPreset {
    Relaxed,
    Fist,
    Open,
    Point,
    HighFive,
    Peace,
}

impl HandPreset {
    pub const ALL: [HandPreset; 6] = [
        HandPreset::Relaxed,
        HandPreset::Fist,
        HandPreset::Open,
        HandPreset::Point,
        HandPreset::HighFive,
        HandPreset::Peace,
    ];

    pub fn label(self) -> &'static str {
        match self {
            HandPreset::Relaxed => "relaxed",
            HandPreset::Fist => "fist",
            HandPreset::Open => "open",
            HandPreset::Point => "point",
            HandPreset::HighFive => "high five",
            HandPreset::Peace => "peace",
        }
    }

    pub fn shortcut_index(self) -> u8 {
        match self {
            HandPreset::Relaxed => 1,
            HandPreset::Fist => 2,
            HandPreset::Open => 3,
            HandPreset::Point => 4,
            HandPreset::HighFive => 5,
            HandPreset::Peace => 6,
        }
    }
}

pub fn preset_from_shortcut(digit: u8) -> Option<HandPreset> {
    HandPreset::ALL
        .into_iter()
        .find(|p| p.shortcut_index() == digit)
}

/// Close-up orbit for a raised right hand; prefers [`new_pose`] camera when loaded.
pub fn raised_right_hand_shot_camera() -> CameraTarget {
    CameraTarget {
        yaw_deg: Some(50.0),
        pitch_deg: Some(4.0),
        distance: Some(1.20),
        look_at: None,
    }
}

pub const HAND_SHOT_POSE_NAME: &str = "new_pose";

fn rot(x: f32, y: f32, z: f32) -> JointTarget {
    JointTarget {
        rotation_deg: Some(EulerDeg { x, y, z }),
        rotation_quat: None,
        translation: None,
    }
}

fn finger_only_fist_right() -> HashMap<String, JointTarget> {
    let mut joints = HashMap::from([
        ("right_index_proximal".into(), rot(0.0, 0.0, 80.0)),
        ("right_index_intermediate".into(), rot(0.0, 0.0, 90.0)),
        ("right_index_distal".into(), rot(0.0, 0.0, 70.0)),
        ("right_middle_proximal".into(), rot(0.0, 0.0, 85.0)),
        ("right_middle_intermediate".into(), rot(0.0, 0.0, 95.0)),
        ("right_middle_distal".into(), rot(0.0, 0.0, 70.0)),
        ("right_ring_proximal".into(), rot(0.0, 0.0, 85.0)),
        ("right_ring_intermediate".into(), rot(0.0, 0.0, 95.0)),
        ("right_ring_distal".into(), rot(0.0, 0.0, 70.0)),
        ("right_little_proximal".into(), rot(0.0, 0.0, 80.0)),
        ("right_little_intermediate".into(), rot(0.0, 0.0, 90.0)),
        ("right_little_distal".into(), rot(0.0, 0.0, 70.0)),
    ]);
    apply_raised_arm_fist_thumb_overlay(&mut joints, BodySide::Right);
    joints
}

/// Thumb chain for a closed fist when the arm is raised (local Z-only curl reads as “thumb up”).
fn apply_raised_arm_fist_thumb_overlay(joints: &mut HashMap<String, JointTarget>, side: BodySide) {
    let p = side.prefix();
    joints.remove(&format!("{p}hand"));
    joints.remove(&format!("{p}thumb_metacarpal"));
    joints.remove(&format!("{p}thumb_proximal"));
    joints.remove(&format!("{p}thumb_distal"));
    joints.insert(
        format!("{p}thumb_metacarpal"),
        rot(52.0, 44.0, 6.0),
    );
    joints.insert(
        format!("{p}thumb_proximal"),
        rot(24.0, 32.0, 38.0),
    );
    joints.insert(format!("{p}thumb_distal"), rot(0.0, 14.0, 44.0));
}

fn without_thumb_and_hand_roll(
    mut joints: HashMap<String, JointTarget>,
    side: BodySide,
) -> HashMap<String, JointTarget> {
    let prefix = side.prefix();
    joints.retain(|name, _| {
        if !name.starts_with(prefix) {
            return true;
        }
        !(name.contains("thumb") || name == &format!("{prefix}hand"))
    });
    joints
}

fn finger_only_point_right() -> HashMap<String, JointTarget> {
    HashMap::from([
        ("right_thumb_metacarpal".into(), rot(18.0, 0.0, 38.0)),
        ("right_thumb_proximal".into(), rot(0.0, 0.0, 45.0)),
        ("right_thumb_distal".into(), rot(0.0, 0.0, 35.0)),
        ("right_index_proximal".into(), rot(0.0, 0.0, 5.0)),
        ("right_index_intermediate".into(), rot(0.0, 0.0, 5.0)),
        ("right_index_distal".into(), rot(0.0, 0.0, 5.0)),
        ("right_middle_proximal".into(), rot(0.0, 0.0, 82.0)),
        ("right_middle_intermediate".into(), rot(0.0, 0.0, 90.0)),
        ("right_middle_distal".into(), rot(0.0, 0.0, 68.0)),
        ("right_ring_proximal".into(), rot(0.0, 0.0, 85.0)),
        ("right_ring_intermediate".into(), rot(0.0, 0.0, 92.0)),
        ("right_ring_distal".into(), rot(0.0, 0.0, 70.0)),
        ("right_little_proximal".into(), rot(0.0, 0.0, 80.0)),
        ("right_little_intermediate".into(), rot(0.0, 0.0, 88.0)),
        ("right_little_distal".into(), rot(0.0, 0.0, 65.0)),
    ])
}

fn finger_only_peace_right() -> HashMap<String, JointTarget> {
    HashMap::from([
        ("right_thumb_metacarpal".into(), rot(15.0, 0.0, 40.0)),
        ("right_thumb_proximal".into(), rot(0.0, 0.0, 50.0)),
        ("right_thumb_distal".into(), rot(0.0, 0.0, 40.0)),
        ("right_index_proximal".into(), rot(0.0, 0.0, 5.0)),
        ("right_index_intermediate".into(), rot(0.0, 0.0, 5.0)),
        ("right_index_distal".into(), rot(0.0, 0.0, 5.0)),
        ("right_middle_proximal".into(), rot(0.0, 0.0, 5.0)),
        ("right_middle_intermediate".into(), rot(0.0, 0.0, 5.0)),
        ("right_middle_distal".into(), rot(0.0, 0.0, 5.0)),
        ("right_ring_proximal".into(), rot(0.0, 0.0, 85.0)),
        ("right_ring_intermediate".into(), rot(0.0, 0.0, 95.0)),
        ("right_ring_distal".into(), rot(0.0, 0.0, 70.0)),
        ("right_little_proximal".into(), rot(0.0, 0.0, 85.0)),
        ("right_little_intermediate".into(), rot(0.0, 0.0, 95.0)),
        ("right_little_distal".into(), rot(0.0, 0.0, 70.0)),
    ])
}

fn finger_only_open_right() -> HashMap<String, JointTarget> {
    HashMap::from([
        ("right_thumb_metacarpal".into(), rot(0.0, 0.0, 10.0)),
        ("right_thumb_proximal".into(), rot(0.0, 0.0, 10.0)),
        ("right_index_proximal".into(), rot(0.0, 0.0, 0.0)),
        ("right_index_intermediate".into(), rot(0.0, 0.0, 0.0)),
        ("right_middle_proximal".into(), rot(0.0, 0.0, 0.0)),
        ("right_ring_proximal".into(), rot(0.0, 0.0, 0.0)),
        ("right_little_proximal".into(), rot(0.0, 0.0, 0.0)),
    ])
}

fn finger_only_high_five_right() -> HashMap<String, JointTarget> {
    HashMap::from([
        ("right_thumb_metacarpal".into(), rot(0.0, 0.0, -15.0)),
        ("right_thumb_proximal".into(), rot(0.0, 0.0, 5.0)),
        ("right_index_proximal".into(), rot(0.0, 0.0, 0.0)),
        ("right_index_intermediate".into(), rot(0.0, 0.0, 0.0)),
        ("right_middle_proximal".into(), rot(0.0, 0.0, 0.0)),
        ("right_middle_intermediate".into(), rot(0.0, 0.0, 0.0)),
        ("right_ring_proximal".into(), rot(0.0, 0.0, 0.0)),
        ("right_ring_intermediate".into(), rot(0.0, 0.0, 0.0)),
        ("right_little_proximal".into(), rot(0.0, 0.0, 0.0)),
        ("right_little_intermediate".into(), rot(0.0, 0.0, 0.0)),
    ])
}

fn finger_relaxed_right() -> HashMap<String, JointTarget> {
    HashMap::from([
        ("right_index_proximal".into(), rot(0.0, 0.0, 12.0)),
        ("right_middle_proximal".into(), rot(0.0, 0.0, 14.0)),
        ("right_ring_proximal".into(), rot(0.0, 0.0, 14.0)),
        ("right_little_proximal".into(), rot(0.0, 0.0, 10.0)),
    ])
}

fn finger_relaxed_left() -> HashMap<String, JointTarget> {
    HashMap::from([
        ("left_index_proximal".into(), rot(0.0, 0.0, 12.0)),
        ("left_middle_proximal".into(), rot(0.0, 0.0, 14.0)),
        ("left_ring_proximal".into(), rot(0.0, 0.0, 14.0)),
        ("left_little_proximal".into(), rot(0.0, 0.0, 10.0)),
    ])
}

fn remap_side(map: HashMap<String, JointTarget>, side: BodySide) -> HashMap<String, JointTarget> {
    match side {
        BodySide::Right => map,
        BodySide::Left => {
            let mut out = HashMap::new();
            for (name, target) in map {
                let new_name = name.replace("right_", "left_");
                out.insert(new_name, target);
            }
            out
        }
    }
}

fn preset_joints_for_side(preset: HandPreset, side: BodySide) -> Option<HashMap<String, JointTarget>> {
    let right = match preset {
        HandPreset::Relaxed => {
            return match side {
                BodySide::Right => Some(finger_relaxed_right()),
                BodySide::Left => Some(finger_relaxed_left()),
            };
        }
        HandPreset::Fist => finger_only_fist_right(),
        HandPreset::Open => finger_only_open_right(),
        HandPreset::Point => finger_only_point_right(),
        HandPreset::HighFive => finger_only_high_five_right(),
        HandPreset::Peace => finger_only_peace_right(),
    };
    Some(remap_side(right, side))
}

const FINGER_SUFFIXES: &[&str] = &[
    "hand",
    "thumb_metacarpal",
    "thumb_proximal",
    "thumb_distal",
    "index_proximal",
    "index_intermediate",
    "index_distal",
    "middle_proximal",
    "middle_intermediate",
    "middle_distal",
    "ring_proximal",
    "ring_intermediate",
    "ring_distal",
    "little_proximal",
    "little_intermediate",
    "little_distal",
    "wrist",
];

fn joint_on_side_fingers(side: BodySide, name: &str) -> bool {
    let prefix = side.prefix();
    if !name.starts_with(prefix) {
        return false;
    }
    let rest = &name[prefix.len()..];
    FINGER_SUFFIXES
        .iter()
        .any(|s| rest == *s || rest.starts_with(&format!("{s}_")))
}

/// Copy finger-chain joints for one side from a reference pose (e.g. `new_pose`).
pub fn finger_joints_from_pose(source: &Pose, side: BodySide) -> HashMap<String, JointTarget> {
    source
        .joints
        .iter()
        .filter(|(name, _)| joint_on_side_fingers(side, name))
        .map(|(name, target)| (name.clone(), target.clone()))
        .collect()
}

/// Merge a hand preset onto one side; optionally mirror to the other hand.
///
/// Fist on the right uses [`HAND_SHOT_POSE_NAME`] when `fist_reference` is provided.
pub fn apply_hand_preset(
    pose: &mut Pose,
    side: BodySide,
    preset: HandPreset,
    symmetrical: bool,
    fist_reference: Option<&Pose>,
) {
    clear_side_joints(pose, side, FINGER_SUFFIXES);
    if symmetrical {
        clear_side_joints(pose, side.opposite(), FINGER_SUFFIXES);
    }

    let joints = if preset == HandPreset::Fist {
        if side == BodySide::Right {
            if let Some(src) = fist_reference {
                let curled = finger_joints_from_pose(src, BodySide::Right);
                if curled.is_empty() {
                    preset_joints_for_side(preset, side).unwrap_or_default()
                } else {
                    let mut base =
                        without_thumb_and_hand_roll(curled, BodySide::Right);
                    apply_raised_arm_fist_thumb_overlay(&mut base, BodySide::Right);
                    base
                }
            } else {
                preset_joints_for_side(preset, side).unwrap_or_default()
            }
        } else {
            preset_joints_for_side(preset, side).unwrap_or_default()
        }
    } else {
        preset_joints_for_side(preset, side).unwrap_or_default()
    };

    for (name, target) in &joints {
        pose.joints.insert(name.clone(), target.clone());
    }

    if symmetrical {
        for (name, target) in mirror_joint_map_to_other_side(&joints) {
            pose.joints.insert(name, target);
        }
    }
}
