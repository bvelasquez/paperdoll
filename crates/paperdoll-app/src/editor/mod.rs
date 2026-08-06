mod apply;
mod hand_presets;
mod joints;
mod posing_guide;
mod session;
mod symmetry;
mod viewport;

pub use apply::{
    capture_scene_to_pose_draft, editor_apply_preview, sync_editor_http_lock,
};
pub use session::{
    AnimationEditorState, Confirm, EditorSession, EditorTab, PendingAction, PoseEditorState,
    StatusKind,
};

use crate::editor::hand_presets::{
    apply_hand_gesture, capture_hand_gesture, raised_right_hand_shot_camera, sorted_gestures,
    HAND_SHOT_POSE_NAME,
};
use crate::editor::session::{euler_for_joint, set_joint_euler, unique_name};
use crate::editor::symmetry::{side_from_joint, BodySide};
use crate::rig_bridge::{
    ActiveVariant, AnimationLibrary, HandGestureLibrary, PoseLibrary, RigPlayback, RigSkeleton,
    ANIMATIONS_DIR, HANDS_DIR, POSES_DIR,
};
use crate::editor::viewport::ViewportPrefs;
use crate::v2_expressions::SharedExpressionState;
use bevy::input::keyboard::KeyCode;
use bevy::prelude::*;
use crate::camera_controls::{apply_viewport_camera_patch, ViewportCamera};
use bevy_egui::input::EguiWantsInput;
use bevy_egui::{egui, EguiContexts, EguiPostUpdateSet, EguiPrimaryContextPass};
use paperdoll_rig::{
    animation_yaml_path, hand_gesture_yaml_path, pose_yaml_path, resolve_animation,
    write_animation_yaml, write_hand_gesture_yaml, write_pose_yaml, AnimationFile, CameraTarget,
    Easing, HandGesture, KeyframeSpec, PlaybackState, Pose,
};
use paperdoll_rig::{animation_to_file, load_animation_file};
use std::collections::{HashMap, HashSet};
use std::path::Path;

pub struct EditorPlugin;

impl Plugin for EditorPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<EditorSession>()
            .init_resource::<viewport::ViewportPrefs>()
            .init_resource::<viewport::GizmoState>()
            .add_systems(Startup, viewport::setup_gizmo_config)
            .add_systems(Update, viewport::apply_hide_mesh)
            .add_systems(Update, (toggle_editor, sync_editor_http_lock, editor_keyboard_shortcuts))
            .add_systems(EguiPrimaryContextPass, (editor_ui, viewport::editor_corner_widget).chain())
            .add_systems(
                PostUpdate,
                (
                    editor_apply_preview,
                    crate::rig_bridge::advance_playback,
                )
                    .chain()
                    .after(EguiPostUpdateSet::ProcessOutput),
            )
            .add_systems(
                PostUpdate,
                (
                    crate::camera_controls::sync_viewport_from_choreography,
                    crate::camera_controls::viewport_camera_controls,
                )
                    .chain()
                    .after(crate::rig_bridge::advance_playback),
            )
            .add_systems(
                PostUpdate,
                viewport::editor_viewport_manip
                    .after(EguiPostUpdateSet::ProcessOutput)
                    .before(editor_apply_preview)
                    .before(crate::camera_controls::viewport_camera_controls),
            );
    }
}

pub fn toggle_editor(keys: Res<ButtonInput<KeyCode>>, mut session: ResMut<EditorSession>) {
    if keys.just_pressed(KeyCode::F2) {
        if session.open {
            if session.active_dirty() {
                session.pending_confirm = Some(Confirm::DiscardChanges {
                    action: PendingAction::CloseEditor,
                });
            } else {
                session.open = false;
                session.info("Play mode — F2 opens the pose/animation editor.");
            }
        } else {
            session.open = true;
            session.info("Editor open (F2 to close). Preview is live.");
        }
    }
}

pub fn editor_ui(
    mut contexts: EguiContexts,
    mut session: ResMut<EditorSession>,
    mut viewport: ResMut<ViewportCamera>,
    mut prefs: ResMut<ViewportPrefs>,
    mut ground: ResMut<crate::rig_bridge::GroundOffset>,
    poses: Res<PoseLibrary>,
    animations: Res<AnimationLibrary>,
    hands: Res<HandGestureLibrary>,
    skeleton: Res<RigSkeleton>,
    playback: Res<RigPlayback>,
    active_variant: Res<ActiveVariant>,
    expressions: Res<SharedExpressionState>,
) {
    if !session.open {
        let Ok(ctx) = contexts.ctx_mut() else {
            return;
        };
        egui::Area::new(egui::Id::new("editor_hint"))
            .fixed_pos(egui::pos2(12.0, 12.0))
            .show(ctx, |ui| {
                ui.label("Press F2 for pose/animation editor");
            });
        return;
    }

    let Ok(ctx) = contexts.ctx_mut() else {
        return;
    };

    let panel_frame = |frame: egui::Frame| {
        frame.fill(egui::Color32::from_rgba_unmultiplied(28, 28, 32, 230))
    };

    egui::TopBottomPanel::top("editor_top")
        .frame(panel_frame(egui::Frame::default()))
        .show(ctx, |ui| {
        ui.horizontal(|ui| {
            ui.label("Paperdoll editor");
            ui.separator();
            let pose_label = if session.pose.dirty() { "Pose ●" } else { "Pose" };
            let anim_label = if session.animation.dirty() { "Animation ●" } else { "Animation" };
            if ui
                .selectable_label(session.tab == EditorTab::Pose, pose_label)
                .on_hover_text("Edit a pose draft (joints, camera, face expressions)")
                .clicked()
            {
                session.tab = EditorTab::Pose;
            }
            if ui
                .selectable_label(session.tab == EditorTab::Animation, anim_label)
                .on_hover_text("Sequence saved poses into an animation clip")
                .clicked()
            {
                session.tab = EditorTab::Animation;
            }
            ui.separator();
            ui.label(format!("variant: {}", active_variant.0));
            ui.separator();
            if ui
                .selectable_label(session.show_help, "?")
                .on_hover_text("Shortcuts & how saving works")
                .clicked()
            {
                session.show_help = !session.show_help;
            }
            ui.separator();
            ui.weak("● = unsaved changes · Ctrl/Cmd+S saves");
        });
        for prev in &session.status_history {
            ui.label(egui::RichText::new(prev).small().weak());
        }
        if !session.status.is_empty() {
            let color = match session.status_kind {
                StatusKind::Info => ui.style().visuals.text_color(),
                StatusKind::Success => egui::Color32::from_rgb(120, 200, 120),
                StatusKind::Error => egui::Color32::LIGHT_RED,
            };
            ui.colored_label(color, &session.status);
        }
    });

    egui::SidePanel::left("editor_left")
        .default_width(300.0)
        .frame(panel_frame(egui::Frame::default()))
        .show(ctx, |ui| {
            match session.tab {
            EditorTab::Pose => pose_panel(
                ui,
                &mut session,
                &poses,
                &animations,
                &skeleton,
                &playback,
            ),
            EditorTab::Animation => {
                anim_panel(
                    ui,
                    &mut session,
                    &mut viewport,
                    &poses,
                    &animations,
                    &skeleton,
                    &playback,
                    &expressions,
                )
            }
        }
            ui.separator();
            ui.collapsing("Stage camera", |ui| {
                stage_camera_panel(ui, &mut session, &mut viewport, &mut prefs, &mut ground, &poses);
            });
        });

    if session.tab == EditorTab::Pose {
        egui::SidePanel::right("editor_right")
            .default_width(260.0)
            .frame(panel_frame(egui::Frame::default()))
            .show(ctx, |ui| {
                pose_joint_inspector(
                    ui,
                    &mut session,
                    &mut prefs,
                    &hands,
                    &expressions,
                );
            });
    }

    // Confirmation dialog for destructive / ambiguous actions.
    if let Some(confirm) = session.pending_confirm.clone() {
        let mut stay_open = true;
        egui::Window::new("Please confirm")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .open(&mut stay_open)
            .show(ctx, |ui| {
                ui.label(confirm.prompt());
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button(confirm.confirm_label()).clicked() {
                        execute_confirm(&confirm, &mut session, &poses, &animations, &skeleton, &playback);
                        session.pending_confirm = None;
                    }
                    if ui.button("Cancel").clicked() {
                        session.pending_confirm = None;
                    }
                });
            });
        if !stay_open {
            session.pending_confirm = None;
        }
    }

    if session.show_help {
        let mut open = true;
        egui::Window::new("Editor help")
            .resizable(false)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.heading("Shortcuts");
                ui.label("F2 — toggle editor (asks before discarding unsaved edits)");
                ui.label("Ctrl/Cmd+S — save the current draft");
                ui.label("Alt+1…7 — hand presets on the active hand (add Shift for the other hand)");
                ui.separator();
                ui.heading("Viewport controls");
                ui.label("Left-click a bone / joint marker — select that joint");
                ui.label("Drag an X / Y / Z row on the corner control — rotate the selected joint");
                ui.label("Selecting a joint dims the rest of the skeleton — its chain stays bright");
                ui.label("Left-drag empty space, or right/middle-drag — orbit the stage");
                ui.label("Scroll — zoom · 'Focus left/right hand' zooms into a hand for finger work");
                ui.label("H — hide/show the doll's skin (skeleton only view)");
                ui.label("Show skeleton/gizmo toggles live in the Stage camera panel.");
                ui.separator();
                ui.heading("How saving works");
                ui.label("Everything is a DRAFT until you press Save. ● marks unsaved edits.");
                ui.label("Save writes YAML to assets/poses or assets/animations AND updates the live library — one save, both places.");
                ui.label("Keyframe #0 is the start pose (t=0). 'blend ms' on later keyframes is the transition time FROM the previous one.");
                ui.label("Animations SNAPSHOT the poses they reference at save time. After editing a pose, use 'Used by animations → Re-save' to refresh them.");
            });
        if !open {
            session.show_help = false;
        }
    }
}

// ---------------------------------------------------------------------------
// Save / rename / delete / reference helpers
// ---------------------------------------------------------------------------

/// Saved animation drafts whose keyframes reference `pose_name`.
fn animations_referencing_pose(pose_name: &str) -> Vec<String> {
    let dir = Path::new(ANIMATIONS_DIR);
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("yaml") {
            continue;
        }
        let Ok(file) = load_animation_file(&path) else {
            continue;
        };
        if file.keyframes.iter().any(|kf| kf.pose.as_deref() == Some(pose_name)) {
            out.push(file.name);
        }
    }
    out.sort();
    out
}

/// Re-resolve and re-save every animation that references `pose_name`, so their
/// snapshotted poses pick up the latest edits. Returns how many were refreshed.
fn resave_animations_referencing(
    pose_name: &str,
    poses: &PoseLibrary,
    animations: &AnimationLibrary,
) -> Result<usize, String> {
    let names = animations_referencing_pose(pose_name);
    for name in &names {
        let path = animation_yaml_path(Path::new(ANIMATIONS_DIR), name);
        let file = load_animation_file(&path).map_err(|e| format!("{name}: {e}"))?;
        let resolved = {
            let poses_guard = poses.0.read().unwrap();
            resolve_animation(file.clone(), &poses_guard).map_err(|e| format!("{name}: {e}"))?
        };
        write_animation_yaml(&path, &file).map_err(|e| format!("{name}: {e}"))?;
        animations.0.write().unwrap().insert(name.clone(), resolved);
    }
    Ok(names.len())
}

/// Save the pose draft, guarding against clobbering a different library entry.
fn request_pose_save(session: &mut EditorSession, poses: &PoseLibrary) {
    let name = session.pose.draft.name.trim().to_string();
    if name.is_empty() {
        session.error("Name the pose before saving.");
        return;
    }
    session.pose.draft.name = name.clone();
    let exists = poses.0.read().unwrap().contains_key(&name);
    let is_own = session.pose.loaded_name.as_deref() == Some(name.as_str());
    if exists && !is_own {
        session.pending_confirm = Some(Confirm::OverwritePose { name });
    } else {
        do_save_pose(session, poses);
    }
}

fn do_save_pose(session: &mut EditorSession, poses: &PoseLibrary) {
    let name = session.pose.draft.name.clone();
    match save_pose(&session.pose.draft, poses) {
        Ok(path) => {
            session.pose.loaded_name = Some(name.clone());
            session.pose.checkpoint();
            let refs = animations_referencing_pose(&name);
            if refs.is_empty() {
                session.success(format!("Saved pose → {}", path.display()));
            } else {
                session.success(format!(
                    "Saved pose → {}. {} animation(s) reference '{name}' and still hold the old snapshot — use 'Used by animations → Re-save'.",
                    path.display(),
                    refs.len()
                ));
            }
        }
        Err(e) => session.error(e),
    }
}

fn request_animation_save(
    session: &mut EditorSession,
    poses: &PoseLibrary,
    animations: &AnimationLibrary,
) {
    let name = session.animation.draft.name.trim().to_string();
    if name.is_empty() {
        session.error("Name the animation before saving.");
        return;
    }
    session.animation.draft.name = name.clone();
    let exists = animations.0.read().unwrap().contains_key(&name);
    let is_own = session.animation.loaded_name.as_deref() == Some(name.as_str());
    if exists && !is_own {
        session.pending_confirm = Some(Confirm::OverwriteAnimation { name });
    } else {
        do_save_animation(session, poses, animations);
    }
}

fn do_save_animation(
    session: &mut EditorSession,
    poses: &PoseLibrary,
    animations: &AnimationLibrary,
) {
    let name = session.animation.draft.name.clone();
    match save_animation(&session.animation.draft, poses, animations) {
        Ok(path) => {
            session.animation.loaded_name = Some(name);
            session.animation.checkpoint();
            session.success(format!("Saved animation → {}", path.display()));
        }
        Err(e) => session.error(e),
    }
}

/// Rename the loaded pose to the draft's name field: writes the new YAML, removes
/// the old file, and rewrites every animation YAML that referenced the old name.
fn rename_loaded_pose(
    session: &mut EditorSession,
    poses: &PoseLibrary,
    animations: &AnimationLibrary,
) {
    let Some(old) = session.pose.loaded_name.clone() else {
        session.error("Load a pose from the library before renaming.");
        return;
    };
    let new = session.pose.draft.name.trim().to_string();
    if new.is_empty() || new == old {
        session.info("Name field is unchanged — nothing to rename.");
        return;
    }
    if poses.0.read().unwrap().contains_key(&new) {
        session.error(format!("'{new}' already exists — pick another name."));
        return;
    }
    session.pose.draft.name = new.clone();
    if let Err(e) = save_pose(&session.pose.draft, poses) {
        session.error(format!("rename failed writing '{new}': {e}"));
        return;
    }
    let old_path = pose_yaml_path(Path::new(POSES_DIR), &old);
    if old_path.exists() {
        let _ = std::fs::remove_file(&old_path);
    }
    poses.0.write().unwrap().remove(&old);

    // Repoint referencing animations at the new name and refresh their snapshots.
    let refs = animations_referencing_pose(&old);
    for name in &refs {
        let path = animation_yaml_path(Path::new(ANIMATIONS_DIR), name);
        let Ok(mut file) = load_animation_file(&path) else {
            continue;
        };
        for kf in &mut file.keyframes {
            if kf.pose.as_deref() == Some(old.as_str()) {
                kf.pose = Some(new.clone());
            }
        }
        let resolved = {
            let poses_guard = poses.0.read().unwrap();
            resolve_animation(file.clone(), &poses_guard)
        };
        if let Ok(resolved) = resolved {
            if write_animation_yaml(&path, &file).is_ok() {
                animations.0.write().unwrap().insert(name.clone(), resolved);
            }
        }
    }

    session.pose.loaded_name = Some(new.clone());
    session.pose.checkpoint();
    if refs.is_empty() {
        session.success(format!("Renamed '{old}' → '{new}'."));
    } else {
        session.success(format!(
            "Renamed '{old}' → '{new}' and updated {} referencing animation(s).",
            refs.len()
        ));
    }
}

fn rename_loaded_animation(session: &mut EditorSession, animations: &AnimationLibrary) {
    let Some(old) = session.animation.loaded_name.clone() else {
        session.error("Load an animation from the library before renaming.");
        return;
    };
    let new = session.animation.draft.name.trim().to_string();
    if new.is_empty() || new == old {
        session.info("Name field is unchanged — nothing to rename.");
        return;
    }
    if animations.0.read().unwrap().contains_key(&new) {
        session.error(format!("'{new}' already exists — pick another name."));
        return;
    }
    let Some(resolved) = animations.0.read().unwrap().get(&old).cloned() else {
        session.error(format!("'{old}' is no longer in the library."));
        return;
    };
    session.animation.draft.name = new.clone();
    let new_path = animation_yaml_path(Path::new(ANIMATIONS_DIR), &new);
    if let Err(e) = write_animation_yaml(&new_path, &session.animation.draft) {
        session.error(format!("rename failed writing '{new}': {e}"));
        return;
    }
    let old_path = animation_yaml_path(Path::new(ANIMATIONS_DIR), &old);
    if old_path.exists() {
        let _ = std::fs::remove_file(&old_path);
    }
    let mut resolved = resolved;
    resolved.name = new.clone();
    let mut guard = animations.0.write().unwrap();
    guard.remove(&old);
    guard.insert(new.clone(), resolved);
    drop(guard);
    session.animation.loaded_name = Some(new.clone());
    session.animation.checkpoint();
    session.success(format!("Renamed '{old}' → '{new}'."));
}

fn delete_pose(session: &mut EditorSession, poses: &PoseLibrary, name: &str) {
    let path = pose_yaml_path(Path::new(POSES_DIR), name);
    if path.exists() {
        if let Err(e) = std::fs::remove_file(&path) {
            session.error(format!("failed to delete {}: {e}", path.display()));
            return;
        }
    }
    poses.0.write().unwrap().remove(name);
    if session.pose.loaded_name.as_deref() == Some(name) {
        session.pose.loaded_name = None;
        session.pose.checkpoint = None; // draft is now an unsaved copy
    }
    let refs = animations_referencing_pose(name);
    if refs.is_empty() {
        session.success(format!("Deleted pose '{name}'."));
    } else {
        session.error(format!(
            "Deleted pose '{name}' — WARNING: {} animation(s) still reference it: {}.",
            refs.len(),
            refs.join(", ")
        ));
    }
}

fn delete_animation(session: &mut EditorSession, animations: &AnimationLibrary, name: &str) {
    let path = animation_yaml_path(Path::new(ANIMATIONS_DIR), name);
    if path.exists() {
        if let Err(e) = std::fs::remove_file(&path) {
            session.error(format!("failed to delete {}: {e}", path.display()));
            return;
        }
    }
    animations.0.write().unwrap().remove(name);
    if session.animation.loaded_name.as_deref() == Some(name) {
        session.animation.loaded_name = None;
        session.animation.checkpoint = None;
    }
    session.success(format!("Deleted animation '{name}'."));
}

/// Run an action that was held back by a discard-changes confirmation.
fn run_pending_action(
    session: &mut EditorSession,
    action: &PendingAction,
    poses: &PoseLibrary,
    animations: &AnimationLibrary,
) {
    match action {
        PendingAction::NewPose => {
            let existing: HashSet<String> = poses.0.read().unwrap().keys().cloned().collect();
            let mut state = PoseEditorState::default();
            if let Some(idle) = poses.0.read().unwrap().get("idle") {
                state.draft = idle.clone();
            }
            state.draft.name = unique_name("new_pose", &existing);
            state.auto_fill_done = true;
            state.symmetrical = session.pose.symmetrical;
            session.pose = state;
            session.pose.checkpoint();
            session.info(format!("New pose draft '{}'.", session.pose.draft.name));
        }
        PendingAction::LoadPose(name) => {
            let loaded = poses.0.read().unwrap().get(name).cloned();
            if let Some(p) = loaded {
                session.pose.draft = p;
                session.pose.loaded_name = Some(name.clone());
                session.pose.checkpoint();
                session.info(format!("Loaded pose '{name}'."));
            } else {
                session.error(format!("Pose '{name}' not found."));
            }
        }
        PendingAction::NewAnimation => {
            let existing: HashSet<String> =
                animations.0.read().unwrap().keys().cloned().collect();
            let mut state = AnimationEditorState::default();
            state.draft.name = unique_name("new_animation", &existing);
            session.animation = state;
            session.animation.checkpoint();
            session.info(format!(
                "New animation draft '{}'.",
                session.animation.draft.name
            ));
        }
        PendingAction::LoadAnimation(name) => {
            let path = animation_yaml_path(Path::new(ANIMATIONS_DIR), name);
            session.animation.draft = load_animation_file(&path).unwrap_or_else(|_| {
                animations
                    .0
                    .read()
                    .unwrap()
                    .get(name)
                    .map(animation_to_file)
                    .unwrap_or_else(|| AnimationEditorState::default().draft)
            });
            session.animation.loaded_name = Some(name.clone());
            session.animation.checkpoint();
            session.animation.playhead_ms = 0;
            session.animation.playing = false;
            session.animation.selected_keyframe = 0;
            session.info(format!("Loaded animation '{name}'."));
        }
        PendingAction::CloseEditor => {
            session.open = false;
            session.info("Play mode — F2 opens the pose/animation editor.");
        }
    }
}

fn execute_confirm(
    confirm: &Confirm,
    session: &mut EditorSession,
    poses: &PoseLibrary,
    animations: &AnimationLibrary,
    skeleton: &RigSkeleton,
    playback: &RigPlayback,
) {
    match confirm {
        Confirm::DiscardChanges { action } => {
            run_pending_action(session, action, poses, animations)
        }
        Confirm::OverwritePose { .. } => do_save_pose(session, poses),
        Confirm::OverwriteAnimation { .. } => do_save_animation(session, poses, animations),
        Confirm::DeletePose { name } => delete_pose(session, poses, name),
        Confirm::DeleteAnimation { name } => delete_animation(session, animations, name),
        Confirm::CaptureScene => {
            let snap = playback.0.current_snapshot(&skeleton.0);
            capture_scene_to_pose_draft(&skeleton.0, &snap, &mut session.pose.draft);
            session.info("Captured current rig into the pose draft.");
        }
        Confirm::ClearAllJoints => {
            session.pose.draft.joints.clear();
            session.info("Cleared all joints from the pose draft.");
        }
    }
}

// ---------------------------------------------------------------------------
// Stage camera
// ---------------------------------------------------------------------------

fn stage_camera_panel(
    ui: &mut egui::Ui,
    session: &mut EditorSession,
    viewport: &mut ViewportCamera,
    prefs: &mut ViewportPrefs,
    ground: &mut crate::rig_bridge::GroundOffset,
    poses: &PoseLibrary,
) {
    // ── Viewport direct-manipulation tools ──────────────────────────────────
    ui.collapsing("Viewport tools", |ui| {
        ui.checkbox(&mut prefs.show_skeleton, "Show skeleton")
            .on_hover_text("Draw the rig's bones and joint markers over the doll");
        ui.checkbox(&mut prefs.show_gizmo, "Rotation controls (corner)")
            .on_hover_text("Show the corner X/Y/Z drag rows — drag a row to rotate the selected joint");
        ui.checkbox(&mut prefs.hide_mesh, "Hide skin (skeleton only)")
            .on_hover_text("Hide the doll's body mesh so only the bones show — great for reaching joints buried in the body");
        ui.checkbox(&mut prefs.left_drag_orbit, "Left-drag orbits empty stage")
            .on_hover_text("Click to select a bone; dragging on empty space orbits (right/middle-drag works too)");
        ui.add_space(4.0);
        ui.label(egui::RichText::new("Model height:").small().weak());
        let mut h = ground.manual;
        if ui
            .add(egui::Slider::new(&mut h, -0.5..=0.5).text("raise/lower"))
            .on_hover_text(
                "The doll auto-stands on the ground. Nudge it up or down here \
                 (e.g. for poses that lift the feet, or to sit it lower).",
            )
            .changed()
        {
            ground.manual = h;
        }
        ui.label(egui::RichText::new("Focus:").small().weak());
        ui.horizontal(|ui| {
            if ui
                .button("Left hand")
                .on_hover_text("Zoom the camera into the left hand for finger work")
                .clicked()
            {
                prefs.pending_focus = Some("left_hand".into());
            }
            if ui
                .button("Right hand")
                .on_hover_text("Zoom the camera into the right hand for finger work")
                .clicked()
            {
                prefs.pending_focus = Some("right_hand".into());
            }
            if session.tab == EditorTab::Pose {
                if let Some(joint) = session.pose.selected_joint.clone() {
                    if ui
                        .button("Selected joint")
                        .on_hover_text("Frame the currently selected joint")
                        .clicked()
                    {
                        prefs.pending_focus = Some(joint);
                    }
                }
            }
        });
    });

    ui.label("Viewport: left/right/middle-drag to orbit · scroll to zoom.");

    let mut yaw = viewport.orbit.yaw_deg;
    let mut pitch = viewport.orbit.pitch_deg;
    let mut distance = viewport.orbit.distance;
    let mut look = viewport.orbit.look_at;

    let mut changed = false;
    changed |= ui
        .add(egui::Slider::new(&mut yaw, -180.0..=180.0).text("yaw_deg"))
        .changed();
    changed |= ui
        .add(egui::Slider::new(&mut pitch, -80.0..=80.0).text("pitch_deg"))
        .changed();
    changed |= ui
        .add(egui::Slider::new(&mut distance, 1.2..=12.0).text("distance"))
        .changed();
    ui.horizontal(|ui| {
        ui.label("look_at");
        changed |= ui
            .add(egui::DragValue::new(&mut look[0]).speed(0.02))
            .changed();
        changed |= ui
            .add(egui::DragValue::new(&mut look[1]).speed(0.02))
            .changed();
        changed |= ui
            .add(egui::DragValue::new(&mut look[2]).speed(0.02))
            .changed();
    });

    if changed {
        viewport.orbit.yaw_deg = yaw;
        viewport.orbit.pitch_deg = pitch;
        viewport.orbit.distance = distance;
        viewport.orbit.look_at = look;
        viewport.user_orbiting = true;
    }

    ui.label(egui::RichText::new("Apply → viewport (preview only)").small().weak());
    ui.horizontal(|ui| {
        if ui
            .button("Default stage")
            .on_hover_text("Reset the viewport to the default framing")
            .clicked()
        {
            viewport.orbit = paperdoll_rig::DEFAULT_CAMERA;
            viewport.user_orbiting = false;
        }
        if ui
            .button("Hand-shot")
            .on_hover_text(format!(
                "Close-up on the raised right hand (camera from '{HAND_SHOT_POSE_NAME}')"
            ))
            .clicked()
        {
            let poses_guard = poses.0.read().unwrap();
            let patch = poses_guard
                .get(HAND_SHOT_POSE_NAME)
                .and_then(|p| p.camera.as_ref())
                .cloned()
                .unwrap_or_else(raised_right_hand_shot_camera);
            drop(poses_guard);
            apply_viewport_camera_patch(&mut viewport.orbit, &patch);
            viewport.user_orbiting = false;
            session.info(format!(
                "Camera from '{HAND_SHOT_POSE_NAME}' (or default hand-shot orbit)."
            ));
        }
        if session.tab == EditorTab::Pose
            && ui
                .button("Pose draft")
                .on_hover_text("Preview the camera block stored in the pose draft")
                .clicked()
        {
            if let Some(patch) = &session.pose.draft.camera {
                apply_viewport_camera_patch(&mut viewport.orbit, patch);
                viewport.user_orbiting = false;
                session.info("Applied pose draft camera to viewport.");
            } else {
                session.info("Pose draft has no camera block yet.");
            }
        }
    });
    if session.tab == EditorTab::Pose {
        ui.label(egui::RichText::new("Capture ← viewport (stores in draft)").small().weak());
        if ui
            .button("Capture → pose draft")
            .on_hover_text("Store the current viewport framing in the pose draft's camera block")
            .clicked()
        {
            session.pose.show_camera = true;
            session.pose.draft.camera = Some(CameraTarget {
                yaw_deg: Some(viewport.orbit.yaw_deg),
                pitch_deg: Some(viewport.orbit.pitch_deg),
                distance: Some(viewport.orbit.distance),
                look_at: Some(viewport.orbit.look_at),
            });
            session.info("Captured viewport camera into the pose draft (save to persist).");
        }
    } else {
        ui.weak("Keyframe camera capture lives in the keyframe detail below.");
    }
}

// ---------------------------------------------------------------------------
// Pose tab
// ---------------------------------------------------------------------------

fn pose_panel(
    ui: &mut egui::Ui,
    session: &mut EditorSession,
    poses: &PoseLibrary,
    animations: &AnimationLibrary,
    skeleton: &RigSkeleton,
    playback: &RigPlayback,
) {
    ui.heading("Pose draft");
    ui.horizontal(|ui| {
        ui.label("name");
        ui.text_edit_singleline(&mut session.pose.draft.name);
        if session.pose.dirty() {
            ui.label(egui::RichText::new("●").color(egui::Color32::from_rgb(230, 170, 60)))
                .on_hover_text("Unsaved changes");
        }
    });

    // Draft lifecycle: new / load / duplicate / rename / delete.
    ui.horizontal(|ui| {
        if ui
            .button("New")
            .on_hover_text("Start a fresh pose from the idle template (auto-named)")
            .clicked()
            && session.guard_dirty(PendingAction::NewPose)
        {
            run_pending_action(session, &PendingAction::NewPose, poses, animations);
        }
        let mut load_request: Option<String> = None;
        egui::ComboBox::from_id_salt("pose_load")
            .selected_text("(load…)")
            .show_ui(ui, |ui| {
                let mut names: Vec<_> = poses.0.read().unwrap().keys().cloned().collect();
                names.sort();
                for name in names {
                    let is_current = session.pose.loaded_name.as_deref() == Some(name.as_str());
                    if ui.selectable_label(is_current, &name).clicked() && !is_current {
                        load_request = Some(name);
                    }
                }
            });
        if let Some(name) = load_request {
            let action = PendingAction::LoadPose(name);
            if session.guard_dirty(action.clone()) {
                run_pending_action(session, &action, poses, animations);
            }
        }
        if ui
            .button("Duplicate")
            .on_hover_text("Copy this draft under a new auto-generated name")
            .clicked()
        {
            let existing: HashSet<String> = poses.0.read().unwrap().keys().cloned().collect();
            let base = format!("{}_copy", session.pose.draft.name);
            session.pose.draft.name = unique_name(&base, &existing);
            session.pose.loaded_name = None;
            session.pose.checkpoint();
            session.info(format!(
                "Duplicated as '{}' — edit, then Save.",
                session.pose.draft.name
            ));
        }
    });
    ui.horizontal(|ui| {
        let loaded = session.pose.loaded_name.is_some();
        if ui
            .add_enabled(loaded, egui::Button::new("Rename"))
            .on_hover_text(
                "Rename the loaded pose to the name field — also updates animations that reference it",
            )
            .clicked()
        {
            rename_loaded_pose(session, poses, animations);
        }
        if ui
            .add_enabled(loaded, egui::Button::new("Delete"))
            .on_hover_text("Delete the loaded pose's YAML file and remove it from the library")
            .clicked()
        {
            if let Some(name) = session.pose.loaded_name.clone() {
                session.pending_confirm = Some(Confirm::DeletePose { name });
            }
        }
        if ui
            .button("Capture scene")
            .on_hover_text("Replace ALL draft joints with the live rig's current state (asks first)")
            .clicked()
        {
            session.pending_confirm = Some(Confirm::CaptureScene);
        }
    });

    // Save.
    ui.horizontal(|ui| {
        let dirty = session.pose.dirty();
        let label = if dirty { "Save ●" } else { "Save" };
        let button = egui::Button::new(label);
        let button = if dirty {
            button.fill(egui::Color32::from_rgb(58, 84, 52))
        } else {
            button
        };
        if ui
            .add(button)
            .on_hover_text("Write YAML to assets/poses AND update the live library (Ctrl/Cmd+S)")
            .clicked()
        {
            request_pose_save(session, poses);
        }
        ui.weak(format!(
            "→ {}",
            pose_yaml_path(Path::new(POSES_DIR), &session.pose.draft.name).display()
        ));
    });

    // Which saved animations snapshot this pose?
    ui.collapsing("Used by animations", |ui| {
        let name = session.pose.draft.name.clone();
        let refs = animations_referencing_pose(&name);
        if refs.is_empty() {
            ui.weak("No saved animation references this pose.");
        } else {
            ui.label(format!("{} reference(s): {}", refs.len(), refs.join(", ")));
            ui.weak("Animations snapshot poses at save time — re-save to pick up edits.");
            if ui
                .button(format!("Re-save {} referencing animation(s)", refs.len()))
                .on_hover_text("Re-resolve and re-write each referencing animation with the current pose")
                .clicked()
            {
                match resave_animations_referencing(&name, poses, animations) {
                    Ok(n) => session.success(format!("Re-saved {n} animation(s).")),
                    Err(e) => session.error(e),
                }
            }
        }
    });

    ui.separator();
    ui.checkbox(&mut session.pose.symmetrical, "Symmetrical (mirror left ↔ right)")
        .on_hover_text("Editing a left/right joint also writes the mirrored rotation to the other side");
    ui.checkbox(&mut session.pose.show_camera, "Camera block")
        .on_hover_text("Edit the pose's optional stage-camera framing (right panel)");
    ui.checkbox(&mut session.pose.show_expressions, "Expressions (v2)")
        .on_hover_text("Edit VRM face expression weights (right panel)");

    ui.separator();
    ui.horizontal(|ui| {
        ui.label("Joints");
        ui.checkbox(&mut session.pose.modified_only, "modified only")
            .on_hover_text("Only list joints already edited in this draft");
    });
    ui.text_edit_singleline(&mut session.pose.joint_filter)
        .on_hover_text("Filter joints by name");
    egui::ScrollArea::vertical().show(ui, |ui| {
        for (group, names) in joints::GROUPS {
            ui.collapsing(*group, |ui| {
                for name in *names {
                    if !joints::joint_matches_filter(name, &session.pose.joint_filter) {
                        continue;
                    }
                    let modified = session.pose.draft.joints.contains_key(*name);
                    if session.pose.modified_only && !modified {
                        continue;
                    }
                    let selected = session.pose.selected_joint.as_deref() == Some(*name);
                    let text = if modified {
                        egui::RichText::new(format!("● {name}")).strong()
                    } else {
                        egui::RichText::new(*name)
                    };
                    if ui
                        .selectable_label(selected, text)
                        .on_hover_text(if modified {
                            "Edited in this draft"
                        } else {
                            "Not yet edited — click to pose it"
                        })
                        .clicked()
                    {
                        session.pose.selected_joint = Some((*name).to_string());
                    }
                }
            });
        }
    });
    if ui
        .button("Clear all joints")
        .on_hover_text("Remove every joint edit from the draft (asks first)")
        .clicked()
    {
        session.pending_confirm = Some(Confirm::ClearAllJoints);
    }

    let _ = skeleton;
    let _ = playback;
}

fn pose_joint_inspector(
    ui: &mut egui::Ui,
    session: &mut EditorSession,
    prefs: &mut ViewportPrefs,
    hands: &HandGestureLibrary,
    expressions: &SharedExpressionState,
) {
    if let Some(joint) = session.pose.selected_joint.clone() {
        ui.horizontal(|ui| {
            ui.heading(&joint);
            if ui
                .button("🔍 Frame")
                .on_hover_text("Move the camera to frame this joint (great for tiny finger bones)")
                .clicked()
            {
                prefs.pending_focus = Some(joint.clone());
            }
        });
        ui.label(
            egui::RichText::new("Drag the X/Y/Z rings on the stage to rotate this joint with the mouse.")
                .small()
                .weak(),
        );
        if let Some(hint) = posing_guide::hint_for_joint(&joint) {
            ui.label(egui::RichText::new(hint).small().weak());
        }
        let mut euler = euler_for_joint(&session.pose.draft, &joint);
        let mut changed = false;
        ui.horizontal(|ui| {
            ui.label("X");
            changed |= ui
                .add(egui::Slider::new(&mut euler.x, -180.0..=180.0).suffix("°"))
                .changed();
        });
        ui.horizontal(|ui| {
            ui.label("Y");
            changed |= ui
                .add(egui::Slider::new(&mut euler.y, -180.0..=180.0).suffix("°"))
                .changed();
        });
        ui.horizontal(|ui| {
            ui.label("Z");
            changed |= ui
                .add(egui::Slider::new(&mut euler.z, -180.0..=180.0).suffix("°"))
                .changed();
        });
        if changed {
            set_joint_euler(
                &mut session.pose.draft,
                &joint,
                euler,
                session.pose.symmetrical,
            );
        }
        if ui
            .button("Clear joint (back to rest)")
            .on_hover_text("Remove this joint's edit so it returns to the rest pose")
            .clicked()
        {
            session.pose.draft.joints.remove(&joint);
            if session.pose.symmetrical {
                if let Some(other) = crate::editor::symmetry::counterpart_joint(&joint) {
                    session.pose.draft.joints.remove(&other);
                }
            }
        }
        ui.separator();
        ui.heading("Hand shapes");
        ui.label(egui::RichText::new(
            "Alt+1…N applies a saved gesture to the active hand (add Shift for the other). \
             Gestures load from assets/hands/*.yaml or POST /hands.",
        ).small().weak());
        let active_side = session
            .pose
            .selected_joint
            .as_deref()
            .and_then(side_from_joint)
            .unwrap_or(BodySide::Right);
        ui.label(format!("active hand: {}", active_side.prefix().trim_end_matches('_')));
        let gestures = {
            let guard = hands.0.read().unwrap();
            sorted_gestures(&guard)
        };
        if gestures.is_empty() {
            ui.label(
                egui::RichText::new("No hand gestures loaded — add assets/hands/*.yaml or POST /hands.")
                    .weak(),
            );
        } else {
            ui.horizontal_wrapped(|ui| {
                for (i, gesture) in gestures.iter().enumerate() {
                    if ui
                        .button(&gesture.name)
                        .on_hover_text(format!("Alt+{}", i + 1))
                        .clicked()
                    {
                        let n = apply_hand_gesture(
                            &mut session.pose.draft,
                            active_side,
                            gesture,
                            session.pose.symmetrical,
                        );
                        session.success(format!(
                            "Hand gesture '{}' on {} hand{} ({} joints)",
                            gesture.name,
                            active_side.prefix().trim_end_matches('_'),
                            if session.pose.symmetrical { ", mirrored" } else { "" },
                            n
                        ));
                    }
                }
            });
        }
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut session.pose.publish_hand_name)
                    .hint_text("new gesture name")
                    .desired_width(140.0),
            );
            let can_save = !session.pose.publish_hand_name.trim().is_empty()
                && gestures.iter().any(|g| g.name != session.pose.publish_hand_name.trim());
            if ui
                .add_enabled(can_save, egui::Button::new("Save active hand as gesture"))
                .on_hover_text(
                    "Capture the active hand's current finger joints as a new named gesture",
                )
                .clicked()
            {
                let name = session.pose.publish_hand_name.trim().to_string();
                match capture_hand_gesture(&session.pose.draft, active_side, name.clone()) {
                    Some(gesture) => {
                        let saved = save_hand_gesture(&gesture, &hands);
                        session.success(
                            saved.as_deref()
                                .map(|p| format!("Saved hand gesture '{}' → {}", name, p))
                                .unwrap_or_else(|| {
                                    format!("Registered hand gesture '{}'", name)
                                }),
                        );
                    }
                    None => session.error(
                        "The active hand has no finger joints set — edit some fingers first.",
                    ),
                }
            }
        });
    } else {
        ui.label("Select a joint from the list (or click a bone in the viewport).");
    }

    if session.pose.show_camera {
        ui.separator();
        ui.heading("Camera");
        let cam = session.pose.draft.camera.get_or_insert_with(CameraTarget::default);
        cam_option_f32(ui, "yaw_deg", &mut cam.yaw_deg, -180.0..=180.0);
        cam_option_f32(ui, "pitch_deg", &mut cam.pitch_deg, -80.0..=80.0);
        cam_option_f32(ui, "distance", &mut cam.distance, 1.2..=12.0);
        if ui
            .button("Reset camera block")
            .on_hover_text("Remove the camera block — this pose then leaves the camera alone")
            .clicked()
        {
            session.pose.draft.camera = None;
        }
    }

    if session.pose.show_expressions {
        ui.separator();
        ui.heading("Expressions");
        let snap = expressions.snapshot();
        if !snap.ready {
            ui.label("Load v2 VRM for expression presets.");
        } else {
            for preset in &snap.available {
                let mut weight = session.pose.draft.expressions.get(preset).copied().unwrap_or(0.0);
                if ui
                    .add(egui::Slider::new(&mut weight, 0.0..=1.0).text(preset))
                    .changed()
                {
                    if weight <= 1e-4 {
                        session.pose.draft.expressions.remove(preset);
                    } else {
                        session.pose.draft.expressions.insert(preset.clone(), weight);
                    }
                }
            }
        }
    }
}

fn cam_option_f32(ui: &mut egui::Ui, label: &str, slot: &mut Option<f32>, range: std::ops::RangeInclusive<f32>) {
    let mut v = slot.unwrap_or(*range.start());
    if ui
        .add(egui::Slider::new(&mut v, range).text(label))
        .changed()
    {
        *slot = Some(v);
    }
}

fn cam_look_at(ui: &mut egui::Ui, slot: &mut Option<[f32; 3]>) {
    let mut look = slot.unwrap_or([0.0, 1.0, 0.0]);
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.label("look_at");
        changed |= ui
            .add(egui::DragValue::new(&mut look[0]).speed(0.02))
            .changed();
        changed |= ui
            .add(egui::DragValue::new(&mut look[1]).speed(0.02))
            .changed();
        changed |= ui
            .add(egui::DragValue::new(&mut look[2]).speed(0.02))
            .changed();
    });
    if changed {
        *slot = Some(look);
    }
}

// ---------------------------------------------------------------------------
// Animation tab
// ---------------------------------------------------------------------------

fn default_keyframe() -> KeyframeSpec {
    KeyframeSpec {
        pose: Some("idle".into()),
        joints: None,
        camera: None,
        expressions: None,
        hold: None,
        duration_ms: 400,
        easing: Easing::EaseInOut,
    }
}

fn anim_panel(
    ui: &mut egui::Ui,
    session: &mut EditorSession,
    viewport: &mut ViewportCamera,
    poses: &PoseLibrary,
    animations: &AnimationLibrary,
    skeleton: &RigSkeleton,
    playback: &RigPlayback,
    expressions: &SharedExpressionState,
) {
    ui.heading("Animation draft");
    ui.horizontal(|ui| {
        ui.label("name");
        ui.text_edit_singleline(&mut session.animation.draft.name);
        if session.animation.dirty() {
            ui.label(egui::RichText::new("●").color(egui::Color32::from_rgb(230, 170, 60)))
                .on_hover_text("Unsaved changes");
        }
    });

    // Draft lifecycle.
    ui.horizontal(|ui| {
        if ui
            .button("New")
            .on_hover_text("Start a fresh animation (auto-named)")
            .clicked()
            && session.guard_dirty(PendingAction::NewAnimation)
        {
            run_pending_action(session, &PendingAction::NewAnimation, poses, animations);
        }
        let mut load_request: Option<String> = None;
        egui::ComboBox::from_id_salt("anim_load")
            .selected_text("(load…)")
            .show_ui(ui, |ui| {
                let lib = animations.0.read().unwrap();
                let mut names: Vec<_> = lib.keys().cloned().collect();
                names.sort();
                for name in names {
                    let is_current =
                        session.animation.loaded_name.as_deref() == Some(name.as_str());
                    if ui.selectable_label(is_current, &name).clicked() && !is_current {
                        load_request = Some(name);
                    }
                }
            });
        if let Some(name) = load_request {
            let action = PendingAction::LoadAnimation(name);
            if session.guard_dirty(action.clone()) {
                run_pending_action(session, &action, poses, animations);
            }
        }
        if ui
            .button("Duplicate")
            .on_hover_text("Copy this draft under a new auto-generated name")
            .clicked()
        {
            let existing: HashSet<String> =
                animations.0.read().unwrap().keys().cloned().collect();
            let base = format!("{}_copy", session.animation.draft.name);
            session.animation.draft.name = unique_name(&base, &existing);
            session.animation.loaded_name = None;
            session.animation.checkpoint();
            session.info(format!(
                "Duplicated as '{}' — edit, then Save.",
                session.animation.draft.name
            ));
        }
    });
    ui.horizontal(|ui| {
        let loaded = session.animation.loaded_name.is_some();
        if ui
            .add_enabled(loaded, egui::Button::new("Rename"))
            .on_hover_text("Rename the loaded animation to the name field")
            .clicked()
        {
            rename_loaded_animation(session, animations);
        }
        if ui
            .add_enabled(loaded, egui::Button::new("Delete"))
            .on_hover_text("Delete the loaded animation's YAML file and remove it from the library")
            .clicked()
        {
            if let Some(name) = session.animation.loaded_name.clone() {
                session.pending_confirm = Some(Confirm::DeleteAnimation { name });
            }
        }
    });

    // File settings vs preview settings — previously three lookalike checkboxes.
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("Saved to YAML:").small().weak());
        ui.checkbox(&mut session.animation.draft.looping, "loop")
            .on_hover_text("Whether this clip loops when played through the API");
        ui.checkbox(&mut session.animation.draft.play_automatically, "bored autoplay")
            .on_hover_text("Allow the bored-idle system to pick this clip at random");
    });
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("Preview:").small().weak());
        ui.checkbox(&mut session.animation.loop_playback, "loop preview")
            .on_hover_text("Editor-only: loop the timeline preview (not saved)");
    });

    let resolved = resolve_editor_animation(&session.animation.draft, poses);
    let resolve_err = resolved.as_ref().err().cloned();
    let total_ms = resolved
        .as_ref()
        .map(PlaybackState::animation_playable_duration_ms)
        .unwrap_or(0);

    if let Some(ref e) = resolve_err {
        ui.colored_label(egui::Color32::LIGHT_RED, e);
    } else if total_ms == 0 && session.animation.draft.keyframes.len() < 2 {
        ui.weak("Add a second keyframe — 'blend ms' only counts on #1 and later.");
    } else if total_ms == 0 {
        ui.weak("Set blend ms on keyframe #1+ (#0 is the start pose at t=0).");
    }

    ui.label(format!(
        "Timeline: {total_ms} ms playable (#0 = start pose; blend ms applies from #1 on)"
    ));
    ui.horizontal(|ui| {
        let can_play = total_ms > 0 && resolve_err.is_none();
        if ui
            .add_enabled(can_play, egui::Button::new(if session.animation.playing {
                "Pause"
            } else {
                "Play"
            }))
            .on_hover_text("Preview the draft on the live rig")
            .clicked()
        {
            if session.animation.playing {
                session.animation.playing = false;
            } else {
                if session.animation.playhead_ms >= total_ms {
                    session.animation.playhead_ms = 0;
                }
                session.animation.playing = true;
            }
        }
        if ui
            .button("Stop")
            .on_hover_text("Halt preview and rewind to t=0")
            .clicked()
        {
            session.animation.playing = false;
            session.animation.playhead_ms = 0;
        }
        ui.label(format!("{} / {} ms", session.animation.playhead_ms, total_ms));
    });

    if total_ms > 0 {
        session.animation.playhead_ms = session.animation.playhead_ms.min(total_ms);
        let mut playhead = session.animation.playhead_ms as f32;
        if ui
            .add(egui::Slider::new(&mut playhead, 0.0..=total_ms as f32).text("scrub"))
            .changed()
        {
            session.animation.playhead_ms = playhead.round() as u32;
            session.animation.playing = false;
        }
    } else {
        session.animation.playhead_ms = 0;
        ui.add_enabled(false, egui::Slider::new(&mut 0.0f32, 0.0..=1.0).text("scrub"));
    }

    // Save.
    ui.horizontal(|ui| {
        let dirty = session.animation.dirty();
        let label = if dirty { "Save ●" } else { "Save" };
        let button = egui::Button::new(label);
        let button = if dirty {
            button.fill(egui::Color32::from_rgb(58, 84, 52))
        } else {
            button
        };
        if ui
            .add(button)
            .on_hover_text(
                "Write YAML to assets/animations AND update the live library (Ctrl/Cmd+S). \
                 Pose references are snapshotted into the saved clip.",
            )
            .clicked()
        {
            request_animation_save(session, poses, animations);
        }
        ui.weak(format!(
            "→ {}",
            animation_yaml_path(Path::new(ANIMATIONS_DIR), &session.animation.draft.name).display()
        ));
    });

    ui.separator();
    ui.horizontal(|ui| {
        ui.label("Keyframes");
        if ui
            .button("+ Add")
            .on_hover_text("Insert a new keyframe after the selected one (references 'idle')")
            .clicked()
        {
            let len = session.animation.draft.keyframes.len();
            let idx = (session.animation.selected_keyframe + 1).min(len);
            session.animation.draft.keyframes.insert(idx, default_keyframe());
            session.animation.selected_keyframe = idx;
        }
        if ui
            .button("Duplicate")
            .on_hover_text("Copy the selected keyframe, inserted right after it")
            .clicked()
        {
            let i = session.animation.selected_keyframe;
            if let Some(kf) = session.animation.draft.keyframes.get(i).cloned() {
                session.animation.draft.keyframes.insert(i + 1, kf);
                session.animation.selected_keyframe = i + 1;
            }
        }
    });
    ui.horizontal(|ui| {
        if ui
            .button("Add pose draft")
            .on_hover_text(
                "Append a keyframe that references the Pose tab's draft by name",
            )
            .clicked()
        {
            let pose_name = session.pose.draft.name.clone();
            let known = poses.0.read().unwrap().contains_key(&pose_name);
            let mut kf = default_keyframe();
            kf.pose = Some(pose_name.clone());
            session.animation.draft.keyframes.push(kf);
            session.animation.selected_keyframe =
                session.animation.draft.keyframes.len() - 1;
            if !known || session.pose.dirty() {
                session.info(format!(
                    "Added keyframe → '{pose_name}'. It isn't saved yet — Save it in the Pose tab or the animation will fail to resolve."
                ));
            } else {
                session.info(format!("Added keyframe → '{pose_name}'."));
            }
        }
        if ui
            .button("Capture scene → keyframe")
            .on_hover_text(
                "Append a keyframe with INLINE joints copied from the rig's current state",
            )
            .clicked()
        {
            let snap = playback.0.current_snapshot(&skeleton.0);
            let mut tmp = Pose {
                name: "capture".into(),
                description: None,
                joints: HashMap::new(),
                camera: None,
                expressions: HashMap::new(),
                hold_joints: false,
            };
            capture_scene_to_pose_draft(&skeleton.0, &snap, &mut tmp);
            let kf = KeyframeSpec {
                pose: None,
                joints: Some(tmp.joints),
                camera: tmp.camera,
                expressions: if tmp.expressions.is_empty() {
                    None
                } else {
                    Some(tmp.expressions)
                },
                hold: None,
                duration_ms: 400,
                easing: Easing::EaseInOut,
            };
            session.animation.draft.keyframes.push(kf);
            session.animation.selected_keyframe =
                session.animation.draft.keyframes.len() - 1;
            session.info("Appended keyframe with inline joints from the live rig.");
        }
    });

    let pose_names: Vec<String> = {
        let mut v: Vec<_> = poses.0.read().unwrap().keys().cloned().collect();
        v.sort();
        v
    };

    let mut apply_kf_camera: Option<CameraTarget> = None;
    egui::ScrollArea::vertical().max_height(280.0).show(ui, |ui| {
        let mut remove_at: Option<usize> = None;
        let mut move_up: Option<usize> = None;
        let mut move_down: Option<usize> = None;
        let len = session.animation.draft.keyframes.len();
        for (i, kf) in session.animation.draft.keyframes.iter_mut().enumerate() {
            let selected = session.animation.selected_keyframe == i;
            ui.group(|ui| {
                ui.horizontal(|ui| {
                    let resp = ui.selectable_label(selected, format!("#{i}"));
                    if resp.clicked() {
                        session.animation.selected_keyframe = i;
                    }
                    if resp
                        .on_hover_text("Double-click to preview this keyframe's camera")
                        .double_clicked()
                    {
                        if let Some(cam) = kf.camera.clone() {
                            apply_kf_camera = Some(cam);
                        }
                    }
                    if i > 0 && ui.small_button("↑").on_hover_text("Move earlier").clicked() {
                        move_up = Some(i);
                    }
                    if i + 1 < len && ui.small_button("↓").on_hover_text("Move later").clicked() {
                        move_down = Some(i);
                    }
                    if i == 0 {
                        ui.label(egui::RichText::new("start").small().weak())
                            .on_hover_text("Entry pose at t=0 — its blend ms is ignored");
                    } else {
                        ui.label("blend");
                        ui.add(egui::DragValue::new(&mut kf.duration_ms).speed(10).suffix(" ms"))
                            .on_hover_text("Transition time FROM the previous keyframe");
                    }
                });
                ui.horizontal(|ui| {
                    ui.label("pose");
                    egui::ComboBox::from_id_salt(format!("kf_pose_{i}"))
                        .selected_text(kf.pose.as_deref().unwrap_or("(inline)"))
                        .show_ui(ui, |ui| {
                            if ui.selectable_label(kf.joints.is_some(), "(inline joints)").clicked()
                            {
                                kf.pose = None;
                                kf.joints.get_or_insert_with(HashMap::new);
                            }
                            for p in &pose_names {
                                if ui.selectable_label(false, p).clicked() {
                                    kf.pose = Some(p.clone());
                                    kf.joints = None;
                                }
                            }
                        });
                });
                easing_combo(ui, &mut kf.easing, i);
                let show_hold = kf.joints.is_some() || kf.expressions.is_some();
                if show_hold {
                    let mut hold = kf.hold.unwrap_or(false);
                    if ui
                        .checkbox(&mut hold, "hold (sparse overlay)")
                        .on_hover_text(
                            "Only listed joints/expressions change; body and face stay as-is.",
                        )
                        .changed()
                    {
                        kf.hold = if hold { Some(true) } else { None };
                    }
                }
                if ui
                    .small_button("✕ remove")
                    .on_hover_text("Delete this keyframe")
                    .clicked()
                {
                    remove_at = Some(i);
                }
            });
        }
        if let Some(i) = remove_at {
            session.animation.draft.keyframes.remove(i);
            session.animation.selected_keyframe = session
                .animation
                .selected_keyframe
                .min(session.animation.draft.keyframes.len().saturating_sub(1));
        }
        if let Some(i) = move_up {
            session.animation.draft.keyframes.swap(i, i - 1);
            session.animation.selected_keyframe = match session.animation.selected_keyframe {
                x if x == i => i - 1,
                x if x == i - 1 => i,
                x => x,
            };
        }
        if let Some(i) = move_down {
            session.animation.draft.keyframes.swap(i, i + 1);
            session.animation.selected_keyframe = match session.animation.selected_keyframe {
                x if x == i => i + 1,
                x if x == i + 1 => i,
                x => x,
            };
        }
    });
    if let Some(cam) = apply_kf_camera {
        apply_viewport_camera_patch(&mut viewport.orbit, &cam);
        viewport.user_orbiting = false;
    }

    if session
        .animation
        .draft
        .keyframes
        .get(session.animation.selected_keyframe)
        .is_some()
    {
        ui.separator();
        ui.label(format!(
            "Keyframe #{} detail",
            session.animation.selected_keyframe
        ));
        if let Some(expr) = &session
            .animation
            .draft
            .keyframes
            .get(session.animation.selected_keyframe)
            .and_then(|kf| kf.expressions.clone())
        {
            ui.label(format!("expressions: {} keys", expr.len()));
        }
        ui.collapsing("Camera patch", |ui| {
            ui.horizontal(|ui| {
                if ui
                    .button("Capture ← viewport")
                    .on_hover_text("Store the current viewport framing on this keyframe")
                    .clicked()
                {
                    if let Some(kf) = session
                        .animation
                        .draft
                        .keyframes
                        .get_mut(session.animation.selected_keyframe)
                    {
                        kf.camera = Some(CameraTarget {
                            yaw_deg: Some(viewport.orbit.yaw_deg),
                            pitch_deg: Some(viewport.orbit.pitch_deg),
                            distance: Some(viewport.orbit.distance),
                            look_at: Some(viewport.orbit.look_at),
                        });
                        session.info(format!(
                            "Captured viewport camera into keyframe #{}.",
                            session.animation.selected_keyframe
                        ));
                    }
                }
                if ui
                    .button("Apply → viewport")
                    .on_hover_text("Preview this keyframe's camera (double-clicking the keyframe works too)")
                    .clicked()
                {
                    if let Some(patch) = session
                        .animation
                        .draft
                        .keyframes
                        .get(session.animation.selected_keyframe)
                        .and_then(|kf| kf.camera.clone())
                    {
                        apply_viewport_camera_patch(&mut viewport.orbit, &patch);
                        viewport.user_orbiting = false;
                    } else {
                        session.info("This keyframe has no camera patch yet.");
                    }
                }
            });
            if let Some(kf_mut) = session
                .animation
                .draft
                .keyframes
                .get_mut(session.animation.selected_keyframe)
            {
                let cam = kf_mut.camera.get_or_insert_with(CameraTarget::default);
                cam_option_f32(ui, "yaw_deg", &mut cam.yaw_deg, -180.0..=180.0);
                cam_option_f32(ui, "pitch_deg", &mut cam.pitch_deg, -80.0..=80.0);
                cam_option_f32(ui, "distance", &mut cam.distance, 1.2..=12.0);
                cam_look_at(ui, &mut cam.look_at);
            }
        });
        let expr_presets: Vec<String> = expressions.snapshot().available;
        if !expr_presets.is_empty() {
            ui.collapsing("Expression overlay", |ui| {
                if let Some(kf_mut) = session
                    .animation
                    .draft
                    .keyframes
                    .get_mut(session.animation.selected_keyframe)
                {
                    let expr_map = kf_mut.expressions.get_or_insert_with(HashMap::new);
                    for preset in &expr_presets {
                        let mut weight = expr_map.get(preset).copied().unwrap_or(0.0);
                        if ui
                            .add(egui::Slider::new(&mut weight, 0.0..=1.0).text(preset))
                            .changed()
                        {
                            if weight <= 1e-4 {
                                expr_map.remove(preset);
                            } else {
                                expr_map.insert(preset.clone(), weight);
                            }
                        }
                    }
                }
            });
        }
    }
}

fn easing_name(e: Easing) -> &'static str {
    match e {
        Easing::Linear => "linear",
        Easing::EaseIn => "ease_in",
        Easing::EaseOut => "ease_out",
        Easing::EaseInOut => "ease_in_out",
        Easing::Step => "step",
    }
}

fn easing_combo(ui: &mut egui::Ui, easing: &mut Easing, keyframe_index: usize) {
    egui::ComboBox::from_id_salt(format!("kf_easing_{keyframe_index}"))
        .selected_text(easing_name(*easing))
        .show_ui(ui, |ui| {
            for e in Easing::ALL {
                if ui
                    .selectable_label(*easing == e, easing_name(e))
                    .clicked()
                {
                    *easing = e;
                }
            }
        });
}

fn resolve_editor_animation(
    draft: &AnimationFile,
    poses: &PoseLibrary,
) -> Result<paperdoll_rig::Animation, String> {
    let poses_guard = poses.0.read().unwrap();
    resolve_animation(draft.clone(), &poses_guard).map_err(|e| e.to_string())
}

fn save_pose(draft: &Pose, poses: &PoseLibrary) -> Result<std::path::PathBuf, String> {
    let skeleton = paperdoll_rig::Skeleton::humanoid_default();
    draft
        .resolve(&skeleton)
        .map_err(|e| format!("invalid pose: {e}"))?;
    let path = pose_yaml_path(Path::new(POSES_DIR), &draft.name);
    write_pose_yaml(&path, draft).map_err(|e| e.to_string())?;
    poses
        .0
        .write()
        .unwrap()
        .insert(draft.name.clone(), draft.clone());
    Ok(path)
}

/// Persist a hand gesture to `assets/hands/*.yaml` and register it in the live
/// library so it shows up immediately and stays after a restart.
fn save_hand_gesture(gesture: &HandGesture, hands: &HandGestureLibrary) -> Option<String> {
    let path = hand_gesture_yaml_path(Path::new(HANDS_DIR), &gesture.name);
    let written = write_hand_gesture_yaml(&path, gesture).is_ok();
    hands
        .0
        .write()
        .unwrap()
        .insert(gesture.name.clone(), gesture.clone());
    if written {
        Some(path.display().to_string())
    } else {
        None
    }
}

fn save_animation(
    draft: &AnimationFile,
    poses: &PoseLibrary,
    animations: &AnimationLibrary,
) -> Result<std::path::PathBuf, String> {
    let poses_guard = poses.0.read().unwrap();
    let resolved = resolve_animation(draft.clone(), &poses_guard).map_err(|e| e.to_string())?;
    let path = animation_yaml_path(Path::new(ANIMATIONS_DIR), &draft.name);
    write_animation_yaml(&path, draft).map_err(|e| e.to_string())?;
    animations
        .0
        .write()
        .unwrap()
        .insert(draft.name.clone(), resolved);
    Ok(path)
}

fn digit_from_key(code: KeyCode) -> Option<u8> {
    match code {
        KeyCode::Digit1 => Some(1),
        KeyCode::Digit2 => Some(2),
        KeyCode::Digit3 => Some(3),
        KeyCode::Digit4 => Some(4),
        KeyCode::Digit5 => Some(5),
        KeyCode::Digit6 => Some(6),
        KeyCode::Digit7 => Some(7),
        KeyCode::Digit8 => Some(8),
        KeyCode::Digit9 => Some(9),
        _ => None,
    }
}

/// Editor shortcuts: Ctrl/Cmd+S saves the active draft; Alt+1…6 applies hand
/// presets in the pose editor (see right inspector).
pub fn editor_keyboard_shortcuts(
    egui_wants: Res<EguiWantsInput>,
    keys: Res<ButtonInput<KeyCode>>,
    poses: Res<PoseLibrary>,
    animations: Res<AnimationLibrary>,
    hands: Res<HandGestureLibrary>,
    mut session: ResMut<EditorSession>,
    mut prefs: ResMut<ViewportPrefs>,
) {
    if !session.open {
        return;
    }
    if egui_wants.wants_keyboard_input() {
        return;
    }

    // H — toggle the doll's skin so only the skeleton shows.
    if keys.just_pressed(KeyCode::KeyH) {
        prefs.hide_mesh = !prefs.hide_mesh;
        if prefs.hide_mesh {
            session.info("Skin hidden — skeleton only (H to show).");
        } else {
            session.info("Skin shown (H to hide).");
        }
    }

    let ctrl = keys.pressed(KeyCode::ControlLeft)
        || keys.pressed(KeyCode::ControlRight)
        || keys.pressed(KeyCode::SuperLeft)
        || keys.pressed(KeyCode::SuperRight);
    if ctrl && keys.just_pressed(KeyCode::KeyS) {
        match session.tab {
            EditorTab::Pose => request_pose_save(&mut session, &poses),
            EditorTab::Animation => request_animation_save(&mut session, &poses, &animations),
        }
        return;
    }

    if session.tab != EditorTab::Pose {
        return;
    }
    let alt = keys.pressed(KeyCode::AltLeft) || keys.pressed(KeyCode::AltRight);
    if !alt {
        return;
    }
    let shift = keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight);
    let active = session
        .pose
        .selected_joint
        .as_deref()
        .and_then(side_from_joint)
        .unwrap_or(BodySide::Right);
    let side = if shift { active.opposite() } else { active };

    for code in [
        KeyCode::Digit1,
        KeyCode::Digit2,
        KeyCode::Digit3,
        KeyCode::Digit4,
        KeyCode::Digit5,
        KeyCode::Digit6,
        KeyCode::Digit7,
        KeyCode::Digit8,
        KeyCode::Digit9,
    ] {
        if !keys.just_pressed(code) {
            continue;
        }
        let Some(digit) = digit_from_key(code) else {
            continue;
        };
        let gesture = {
            let guard = hands.0.read().unwrap();
            sorted_gestures(&guard)
                .into_iter()
                .nth((digit - 1) as usize)
        };
        let Some(gesture) = gesture else {
            continue;
        };
        let symmetrical = session.pose.symmetrical;
        apply_hand_gesture(
            &mut session.pose.draft,
            side,
            &gesture,
            symmetrical,
        );
        session.success(format!(
            "Hand gesture '{}' on {} hand{}",
            gesture.name,
            side.prefix().trim_end_matches('_'),
            if symmetrical { " (mirrored)" } else { "" }
        ));
    }
}
