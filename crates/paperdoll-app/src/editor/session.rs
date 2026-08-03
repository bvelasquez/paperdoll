use bevy::prelude::Resource;
use paperdoll_rig::{AnimationFile, EulerDeg, Pose};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EditorTab {
    #[default]
    Pose,
    Animation,
}

#[derive(Resource)]
pub struct EditorSession {
    pub open: bool,
    pub tab: EditorTab,
    pub pose: PoseEditorState,
    pub animation: AnimationEditorState,
    pub status: String,
}

impl Default for EditorSession {
    fn default() -> Self {
        Self {
            open: false,
            tab: EditorTab::Pose,
            pose: PoseEditorState::default(),
            animation: AnimationEditorState::default(),
            status: "Play mode — F2 opens the pose/animation editor.".into(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct PoseEditorState {
    pub draft: Pose,
    pub selected_joint: Option<String>,
    pub joint_filter: String,
    pub show_camera: bool,
    pub show_expressions: bool,
    /// When true, editing a left/right pair mirrors euler to the counterpart joint.
    pub symmetrical: bool,
}

impl Default for PoseEditorState {
    fn default() -> Self {
        Self {
            draft: Pose {
                name: "new_pose".into(),
                description: None,
                joints: HashMap::new(),
                camera: None,
                expressions: HashMap::new(),
                hold_joints: false,
            },
            selected_joint: None,
            joint_filter: String::new(),
            show_camera: false,
            show_expressions: false,
            symmetrical: false,
        }
    }
}

#[derive(Debug, Clone)]
pub struct AnimationEditorState {
    pub draft: AnimationFile,
    pub playhead_ms: u32,
    pub playing: bool,
    pub loop_playback: bool,
    pub selected_keyframe: usize,
}

impl Default for AnimationEditorState {
    fn default() -> Self {
        Self {
            draft: AnimationFile {
                name: "new_animation".into(),
                description: None,
                looping: false,
                vrm_local_rotations: false,
                play_automatically: false,
                keyframes: vec![paperdoll_rig::KeyframeSpec {
                    pose: Some("idle".into()),
                    joints: None,
                    camera: None,
                    expressions: None,
                    hold: None,
                    duration_ms: 400,
                    easing: paperdoll_rig::Easing::EaseInOut,
                }],
            },
            playhead_ms: 0,
            playing: false,
            loop_playback: false,
            selected_keyframe: 0,
        }
    }
}

pub fn euler_for_joint(pose: &Pose, joint: &str) -> EulerDeg {
    pose.joints
        .get(joint)
        .and_then(|t| t.rotation_deg)
        .unwrap_or_default()
}

pub fn set_joint_euler(pose: &mut Pose, joint: &str, euler: EulerDeg, symmetrical: bool) {
    crate::editor::symmetry::set_joint_euler_with_symmetry(pose, joint, euler, symmetrical);
}
