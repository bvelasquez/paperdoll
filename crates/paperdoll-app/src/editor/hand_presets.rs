//! Hand-shape helpers for the pose editor (v2 finger chains).
//!
//! Hand shapes are now **data-driven** [`HandGesture`]s loaded from
//! `assets/hands/*.yaml` (plus runtime `POST /hands`), not a hard-coded enum. A
//! gesture's `joints` are side-agnostic keys (`index_proximal`, `thumb_metacarpal`,
//! …) that this module splices onto the active hand's side and can mirror across.

use super::symmetry::{mirror_joint_map_to_other_side, BodySide, clear_side_joints};
use paperdoll_rig::{CameraTarget, HandGesture, Pose};

/// Every finger/hand joint a hand gesture may touch, plus `hand`/`wrist` so applying
/// a gesture also clears any leftover hand/wrist edits on that side (matching the old
/// hard-coded preset behavior).
pub const GESTURE_CLEAR_SUFFIXES: &[&str] = &[
    "hand",
    "wrist",
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
];

/// Close-up orbit for a raised right hand; prefers the [`HAND_SHOT_POSE_NAME`] pose's
/// camera block when that pose is loaded.
pub fn raised_right_hand_shot_camera() -> CameraTarget {
    CameraTarget {
        yaw_deg: Some(50.0),
        pitch_deg: Some(4.0),
        distance: Some(1.20),
        look_at: None,
    }
}

/// Reference pose (`assets/poses/raised_right_hand.yaml`) used for the hand-shot
/// camera preset and as the fist-shape source for the right hand.
pub const HAND_SHOT_POSE_NAME: &str = "raised_right_hand";

/// Capture the active hand's current finger joints from a pose as a side-agnostic
/// [`HandGesture`] (keyed `index_proximal`, … — no side prefix), ready to be saved to
/// `assets/hands/*.yaml` or POSTed to `/hands`. Returns `None` if the hand has no
/// finger edits.
pub fn capture_hand_gesture(pose: &Pose, side: BodySide, name: String) -> Option<HandGesture> {
    if pose.joints.is_empty() {
        return None;
    }
    let joints = HandGesture::strip_prefix_to_keys(side.prefix(), &pose.joints);
    if joints.is_empty() {
        return None;
    }
    Some(HandGesture {
        name,
        description: None,
        joints,
    })
}

/// Merge a named hand gesture onto one side of the pose; optionally mirror to the
/// other hand. The gesture's side-agnostic keys are prefixed with the active side's
/// `right_`/`left_`. When `symmetrical`, the opposite hand gets the mirrored copy.
pub fn apply_hand_gesture(
    pose: &mut Pose,
    side: BodySide,
    gesture: &HandGesture,
    symmetrical: bool,
) -> usize {
    clear_side_joints(pose, side, GESTURE_CLEAR_SUFFIXES);
    if symmetrical {
        clear_side_joints(pose, side.opposite(), GESTURE_CLEAR_SUFFIXES);
    }

    let base = gesture.resolve_for_prefix(side.prefix());
    let mut applied = 0;
    for (name, target) in &base {
        pose.joints.insert(name.clone(), target.clone());
        applied += 1;
    }

    if symmetrical {
        for (name, target) in mirror_joint_map_to_other_side(&base) {
            pose.joints.insert(name, target);
            applied += 1;
        }
    }
    applied
}

/// Build a labeled list of loaded gestures with their derived shortcuts. Shortcuts
/// are positional (1-based) over the sorted-by-name gesture list, so 1 is always the
/// alphabetically-first gesture regardless of what was added at runtime.
pub fn sorted_gestures(hands: &std::collections::HashMap<String, HandGesture>) -> Vec<HandGesture> {
    let mut v: Vec<HandGesture> = hands.values().cloned().collect();
    v.sort_by(|a, b| a.name.cmp(&b.name));
    v
}
