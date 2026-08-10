use bevy::prelude::Resource;
use paperdoll_rig::{AnimationFile, EulerDeg, Pose};
use std::collections::{HashMap, HashSet, VecDeque};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EditorTab {
    #[default]
    Pose,
    Animation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusKind {
    Info,
    Success,
    Error,
}

/// A destructive or ambiguous action waiting on the user's confirmation dialog.
#[derive(Debug, Clone)]
pub enum Confirm {
    /// About to throw away unsaved draft edits (New / Load / close editor).
    DiscardChanges { action: PendingAction },
    /// Draft name already belongs to a *different* saved pose — saving would clobber it.
    OverwritePose { name: String },
    OverwriteAnimation { name: String },
    DeletePose { name: String },
    DeleteAnimation { name: String },
    /// "Capture scene" replaces every joint in the pose draft.
    CaptureScene,
    ClearAllJoints,
}

#[derive(Debug, Clone)]
pub enum PendingAction {
    NewPose,
    LoadPose(String),
    NewAnimation,
    LoadAnimation(String),
    CloseEditor,
}

impl Confirm {
    pub fn prompt(&self) -> String {
        match self {
            Confirm::DiscardChanges { .. } => {
                "Discard unsaved changes to the current draft?".into()
            }
            Confirm::OverwritePose { name } => format!(
                "'{name}' already exists in the library and you didn't load it from there. Overwrite it?"
            ),
            Confirm::OverwriteAnimation { name } => format!(
                "'{name}' already exists in the library and you didn't load it from there. Overwrite it?"
            ),
            Confirm::DeletePose { name } => {
                format!("Delete pose '{name}' (removes the YAML file from assets/poses)?")
            }
            Confirm::DeleteAnimation { name } => format!(
                "Delete animation '{name}' (removes the YAML file from assets/animations)?"
            ),
            Confirm::CaptureScene => {
                "Replace ALL joints in the pose draft with the live rig state?".into()
            }
            Confirm::ClearAllJoints => "Clear every joint edit from the pose draft?".into(),
        }
    }

    pub fn confirm_label(&self) -> &'static str {
        match self {
            Confirm::DiscardChanges { .. } => "Discard",
            Confirm::OverwritePose { .. } | Confirm::OverwriteAnimation { .. } => "Overwrite",
            Confirm::DeletePose { .. } | Confirm::DeleteAnimation { .. } => "Delete",
            Confirm::CaptureScene => "Capture",
            Confirm::ClearAllJoints => "Clear",
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct JointInspectorState {
    pub selected_joint: Option<String>,
    pub joint_filter: String,
    pub modified_only: bool,
    pub symmetrical: bool,
}

#[derive(Resource)]
pub struct EditorSession {
    pub open: bool,
    pub tab: EditorTab,
    pub pose: PoseEditorState,
    pub animation: AnimationEditorState,
    pub joints: JointInspectorState,
    /// Latest status line; colored by [`Self::status_kind`].
    pub status: String,
    pub status_kind: StatusKind,
    /// Previous messages, most recent last (capped).
    pub status_history: VecDeque<String>,
    pub pending_confirm: Option<Confirm>,
    pub show_help: bool,
}

impl Default for EditorSession {
    fn default() -> Self {
        // `PAPERDOLL_EDITOR_OPEN=1` launches straight into the editor (handy for
        // visual testing / agent workflows that drive the app over HTTP).
        let open_by_env = std::env::var("PAPERDOLL_EDITOR_OPEN")
            .map(|v| v == "1")
            .unwrap_or(false);
        Self {
            open: open_by_env,
            tab: EditorTab::Pose,
            pose: PoseEditorState::default(),
            animation: AnimationEditorState::default(),
            joints: JointInspectorState::default(),
            status: "Play mode — F2 opens the pose/animation editor.".into(),
            status_kind: StatusKind::Info,
            status_history: VecDeque::new(),
            pending_confirm: None,
            show_help: false,
        }
    }
}

impl EditorSession {
    pub fn set_status(&mut self, kind: StatusKind, msg: impl Into<String>) {
        let msg = msg.into();
        if !self.status.is_empty() && self.status != msg {
            self.status_history.push_back(std::mem::replace(&mut self.status, msg));
        } else {
            self.status = msg;
        }
        while self.status_history.len() > 2 {
            self.status_history.pop_front();
        }
        self.status_kind = kind;
    }

    pub fn info(&mut self, msg: impl Into<String>) {
        self.set_status(StatusKind::Info, msg);
    }

    pub fn success(&mut self, msg: impl Into<String>) {
        self.set_status(StatusKind::Success, msg);
    }

    pub fn error(&mut self, msg: impl Into<String>) {
        self.set_status(StatusKind::Error, msg);
    }

    /// Unsaved edits on the draft of the currently visible tab.
    pub fn active_dirty(&self) -> bool {
        match self.tab {
            EditorTab::Pose => self.pose.dirty(),
            EditorTab::Animation => self.animation.dirty(),
        }
    }

    /// Guard a draft-replacing action behind a discard confirmation when dirty.
    /// Returns true if the caller should run the action now, false if a confirm
    /// dialog was queued instead.
    pub fn guard_dirty(&mut self, action: PendingAction) -> bool {
        if self.active_dirty() {
            self.pending_confirm = Some(Confirm::DiscardChanges { action });
            false
        } else {
            true
        }
    }
}

/// Pick `base`, or `base_2`, `base_3`, … when the name is already taken, so a fresh
/// draft can never silently collide with an existing library entry.
pub fn unique_name(base: &str, existing: &HashSet<String>) -> String {
    if !existing.contains(base) {
        return base.to_string();
    }
    for n in 2..1000 {
        let candidate = format!("{base}_{n}");
        if !existing.contains(&candidate) {
            return candidate;
        }
    }
    format!("{base}_{}", std::process::id())
}

#[derive(Debug, Clone)]
pub struct PoseEditorState {
    pub draft: Pose,
    /// Last checkpoint (created / loaded / saved). `dirty()` compares against this.
    pub checkpoint: Option<Pose>,
    /// Library name the draft was loaded from (None = never loaded/saved).
    pub loaded_name: Option<String>,
    pub show_camera: bool,
    pub show_expressions: bool,
    /// Name for the "save current hand as gesture" inline field.
    pub publish_hand_name: String,
    /// Set once the empty default draft has been auto-filled from `idle` so a
    /// deliberate "clear all joints" doesn't get silently refilled.
    pub auto_fill_done: bool,
}

impl PoseEditorState {
    pub fn dirty(&self) -> bool {
        self.checkpoint.as_ref() != Some(&self.draft)
    }

    /// Mark the current draft as the clean baseline.
    pub fn checkpoint(&mut self) {
        self.checkpoint = Some(self.draft.clone());
    }
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
            checkpoint: None,
            loaded_name: None,
            show_camera: false,
            show_expressions: false,
            auto_fill_done: false,
            publish_hand_name: String::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct AnimationEditorState {
    pub draft: AnimationFile,
    pub checkpoint: Option<AnimationFile>,
    pub loaded_name: Option<String>,
    pub playhead_ms: u32,
    pub playing: bool,
    pub loop_playback: bool,
    pub selected_keyframe: usize,
}

impl AnimationEditorState {
    pub fn dirty(&self) -> bool {
        self.checkpoint.as_ref() != Some(&self.draft)
    }

    pub fn checkpoint(&mut self) {
        self.checkpoint = Some(self.draft.clone());
    }
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
            checkpoint: None,
            loaded_name: None,
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

pub fn set_active_joint_euler(
    session: &mut EditorSession,
    poses: &HashMap<String, Pose>,
    joint: &str,
    euler: EulerDeg,
) {
    let symmetrical = session.joints.symmetrical;
    match session.tab {
        EditorTab::Pose => set_joint_euler(&mut session.pose.draft, joint, euler, symmetrical),
        EditorTab::Animation => {
            let idx = session.animation.selected_keyframe;
            session.animation.playhead_ms = crate::editor::timeline::keyframe_arrival_ms(
                &session.animation.draft.keyframes,
                idx,
            );
            session.animation.playing = false;
            if let Some(kf) = session.animation.draft.keyframes.get_mut(idx) {
                crate::editor::keyframe_joints::set_keyframe_joint_euler(
                    kf, poses, joint, euler, symmetrical,
                );
            }
        }
    }
}

pub fn euler_for_active_joint(
    session: &EditorSession,
    poses: &HashMap<String, Pose>,
    joint: &str,
) -> EulerDeg {
    match session.tab {
        EditorTab::Pose => euler_for_joint(&session.pose.draft, joint),
        EditorTab::Animation => session
            .animation
            .draft
            .keyframes
            .get(session.animation.selected_keyframe)
            .map(|kf| crate::editor::keyframe_joints::euler_for_keyframe_joint(kf, poses, joint))
            .unwrap_or_default(),
    }
}

pub fn joint_editing_active(session: &EditorSession) -> bool {
    match session.tab {
        EditorTab::Pose => true,
        EditorTab::Animation => crate::editor::keyframe_joints::keyframe_joint_editing_enabled(
            &session.animation.draft,
        ),
    }
}
