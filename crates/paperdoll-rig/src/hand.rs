//! Named hand shapes for the pose editor and HTTP API.
//!
//! A [`HandGesture`] is a *side-agnostic* bundle of finger-chain joint rotations
//! (keys like `index_proximal`, `thumb_metacarpal` — no `left_`/`right_` prefix).
//! The pose editor prefixes them with the active hand's side when applying, and the
//! same shape works for either hand. This replaces the old hard-coded `HandPreset`
//! enum: gestures now live as `assets/hands/*.yaml` files (plus runtime registration
//! via `POST /hands`), so anyone can author a new hand shape without editing Rust.

use crate::pose::JointTarget;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// The side-agnostic finger joints a gesture may set. Shared by the pose editor's
/// "save current hand as gesture" capture (which filters the active hand's joints to
/// exactly these keys) and by validation.
pub const HAND_GESTURE_JOINT_KEYS: &[&str] = &[
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

/// A named, reusable hand shape: sparse finger-chain joint rotations keyed without a
/// side prefix. Same authoring shape as a pose's `joints`, but scoped to one hand.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HandGesture {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub joints: HashMap<String, JointTarget>,
}

impl HandGesture {
    /// Map a gesture's side-agnostic joints onto one hand's fully-qualified joint
    /// names by prepending `right_`/`left_`. Returns only the keys present in the
    /// gesture (mirrors the built-in `preset_joints_for_side` behavior).
    pub fn resolve_for_prefix(&self, prefix: &str) -> HashMap<String, JointTarget> {
        self.joints
            .iter()
            .map(|(k, v)| (format!("{prefix}{k}"), v.clone()))
            .collect()
    }

    /// Consume a set of fully-qualified `prefix…`-hand joints and strip the prefix to
    /// rebuild a side-agnostic gesture map. Used by the editor's "capture current hand"
    /// flow so a saved gesture is symmetric-safe to replay onto either hand.
    pub fn strip_prefix_to_keys(prefix: &str, joints: &HashMap<String, JointTarget>) -> HashMap<String, JointTarget> {
        let mut out = HashMap::new();
        for (name, target) in joints {
            if let Some(rest) = name.strip_prefix(prefix) {
                if HAND_GESTURE_JOINT_KEYS.contains(&rest) {
                    out.insert(rest.to_string(), target.clone());
                }
            }
        }
        out
    }
}