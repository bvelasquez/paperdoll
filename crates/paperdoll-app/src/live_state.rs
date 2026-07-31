//! Shared live snapshot of the rig + camera, written every frame by Bevy systems and
//! read by the HTTP thread for `GET /state`. Mirrors the Arc<RwLock<_>> pattern used
//! for pose/animation libraries: pure data, no ECS access required on the read side.

use bevy::prelude::Resource;
use paperdoll_rig::{EulerDeg, PlaybackMode, ResolvedCamera, ResolvedPose, Skeleton, DEFAULT_CAMERA};
use serde::Serialize;
use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

/// Agent-friendly snapshot published for `GET /state`.
#[derive(Debug, Clone, Serialize)]
pub struct LiveStateSnapshot {
    pub playback: PlaybackStatus,
    pub joints: BTreeMap<String, JointState>,
    pub camera: ResolvedCamera,
}

#[derive(Debug, Clone, Serialize)]
pub struct PlaybackStatus {
    pub mode: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub animation: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keyframe_index: Option<usize>,
}

#[derive(Debug, Clone, Serialize)]
pub struct JointState {
    pub rotation_deg: EulerDeg,
}

impl Default for LiveStateSnapshot {
    fn default() -> Self {
        Self {
            playback: PlaybackStatus {
                mode: "idle",
                animation: None,
                keyframe_index: None,
            },
            joints: BTreeMap::new(),
            camera: DEFAULT_CAMERA,
        }
    }
}

/// Shared handle held by both Bevy (`Resource`) and the HTTP `ApiState`.
#[derive(Resource, Clone)]
pub struct LiveState(pub Arc<RwLock<LiveStateSnapshot>>);

impl LiveState {
    pub fn new() -> Self {
        Self(Arc::new(RwLock::new(LiveStateSnapshot::default())))
    }

    pub fn publish(&self, skeleton: &Skeleton, pose: &ResolvedPose, mode: &PlaybackMode) {
        let mut joints = BTreeMap::new();
        for (id, rot) in &pose.joint_rotations {
            let name = skeleton.joint(*id).name.clone();
            joints.insert(
                name,
                JointState {
                    rotation_deg: EulerDeg::from_quat(*rot),
                },
            );
        }
        let playback = match mode {
            PlaybackMode::Idle => PlaybackStatus {
                mode: "idle",
                animation: None,
                keyframe_index: None,
            },
            PlaybackMode::Transitioning => PlaybackStatus {
                mode: "transitioning",
                animation: None,
                keyframe_index: None,
            },
            PlaybackMode::Playing {
                animation,
                keyframe_index,
            } => PlaybackStatus {
                mode: "playing",
                animation: Some(animation.clone()),
                keyframe_index: Some(*keyframe_index),
            },
        };
        *self.0.write().unwrap() = LiveStateSnapshot {
            playback,
            joints,
            camera: pose.camera,
        };
    }
}

impl Default for LiveState {
    fn default() -> Self {
        Self::new()
    }
}
