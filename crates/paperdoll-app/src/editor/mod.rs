mod apply;
mod hand_presets;
mod joints;
mod posing_guide;
mod session;
mod symmetry;

pub use apply::{
    capture_scene_to_pose_draft, editor_apply_preview, sync_editor_http_lock,
};
pub use session::{AnimationEditorState, EditorSession, EditorTab, PoseEditorState};

use crate::editor::hand_presets::{
    apply_hand_preset, preset_from_shortcut, raised_right_hand_shot_camera, HandPreset,
    HAND_SHOT_POSE_NAME,
};
use crate::editor::session::{euler_for_joint, set_joint_euler};
use crate::editor::symmetry::{side_from_joint, BodySide};
use crate::rig_bridge::{
    ActiveVariant, AnimationLibrary, ChoreographyCameraEntity, PoseLibrary, RigEntities,
    RigPlayback, RigSkeleton, ANIMATIONS_DIR, POSES_DIR,
};
use crate::v2_expressions::SharedExpressionState;
use bevy::input::keyboard::KeyCode;
use bevy::input::mouse::MouseButton;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use crate::camera_controls::{apply_viewport_camera_patch, ViewportCamera};
use bevy_egui::input::EguiWantsInput;
use bevy_egui::{egui, EguiContexts, EguiPostUpdateSet, EguiPrimaryContextPass};
use paperdoll_rig::{
    animation_yaml_path, pose_yaml_path, resolve_animation, write_animation_yaml, write_pose_yaml,
    AnimationFile, CameraTarget, Easing, KeyframeSpec, PlaybackState, Pose,
};
use paperdoll_rig::{animation_to_file, load_animation_file};
use std::collections::HashMap;
use std::path::Path;

pub struct EditorPlugin;

impl Plugin for EditorPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<EditorSession>()
            .add_systems(Update, (toggle_editor, sync_editor_http_lock, editor_keyboard_shortcuts))
            .add_systems(EguiPrimaryContextPass, editor_ui)
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
            .add_systems(PostUpdate, editor_bone_pick);
    }
}

pub fn toggle_editor(keys: Res<ButtonInput<KeyCode>>, mut session: ResMut<EditorSession>) {
    if keys.just_pressed(KeyCode::F2) {
        session.open = !session.open;
        if session.open {
            session.status = "Editor open (F2 to close). Preview is live.".into();
        } else {
            session.status = "Play mode — F2 opens the pose/animation editor.".into();
        }
    }
}

pub fn editor_ui(
    mut contexts: EguiContexts,
    mut session: ResMut<EditorSession>,
    mut viewport: ResMut<ViewportCamera>,
    poses: Res<PoseLibrary>,
    animations: Res<AnimationLibrary>,
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
            if ui.selectable_label(session.tab == EditorTab::Pose, "Pose").clicked() {
                session.tab = EditorTab::Pose;
            }
            if ui
                .selectable_label(session.tab == EditorTab::Animation, "Animation")
                .clicked()
            {
                session.tab = EditorTab::Animation;
            }
            ui.separator();
            ui.label(format!("variant: {}", active_variant.0));
            ui.separator();
            ui.weak("Live preview in center viewport");
        });
        if !session.status.is_empty() {
            ui.label(&session.status);
        }
    });

    egui::SidePanel::left("editor_left")
        .default_width(300.0)
        .frame(panel_frame(egui::Frame::default()))
        .show(ctx, |ui| {
            match session.tab {
            EditorTab::Pose => pose_panel(ui, &mut session, &poses, &skeleton, &playback),
            EditorTab::Animation => {
                anim_panel(
                    ui,
                    &mut session,
                    &poses,
                    &animations,
                    &skeleton,
                    &expressions,
                )
            }
        }
            ui.separator();
            ui.collapsing("Stage camera", |ui| {
                stage_camera_panel(ui, &mut session, &mut viewport, &poses);
            });
        });

    if session.tab == EditorTab::Pose {
        egui::SidePanel::right("editor_right")
            .default_width(260.0)
            .frame(panel_frame(egui::Frame::default()))
            .show(ctx, |ui| {
                pose_joint_inspector(ui, &mut session.pose, &poses, &expressions);
            });
    }
}

fn stage_camera_panel(
    ui: &mut egui::Ui,
    session: &mut EditorSession,
    viewport: &mut ViewportCamera,
    poses: &PoseLibrary,
) {
    ui.heading("Stage camera");
    ui.label("Viewport: right-drag or middle-drag to orbit · scroll to zoom.");
    ui.weak("Frame the hand yourself, then Capture → pose or GET /screenshot.");

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

    ui.horizontal(|ui| {
        if ui.button("Default stage").clicked() {
            viewport.orbit = paperdoll_rig::DEFAULT_CAMERA;
            viewport.user_orbiting = false;
        }
        if ui.button("Raised right hand shot").clicked() {
            let poses_guard = poses.0.read().unwrap();
            let patch = poses_guard
                .get(HAND_SHOT_POSE_NAME)
                .and_then(|p| p.camera.as_ref())
                .cloned()
                .unwrap_or_else(raised_right_hand_shot_camera);
            drop(poses_guard);
            apply_viewport_camera_patch(&mut viewport.orbit, &patch);
            viewport.user_orbiting = false;
            session.status = format!(
                "Camera from '{HAND_SHOT_POSE_NAME}' (or default hand-shot orbit)."
            );
        }
        if ui.button("Capture → pose YAML block").clicked() {
            session.pose.show_camera = true;
            session.pose.draft.camera = Some(CameraTarget {
                yaw_deg: Some(viewport.orbit.yaw_deg),
                pitch_deg: Some(viewport.orbit.pitch_deg),
                distance: Some(viewport.orbit.distance),
                look_at: Some(viewport.orbit.look_at),
            });
            session.status = "Captured stage camera into pose draft (save pose to persist).".into();
        }
        if session.tab == EditorTab::Animation {
            if ui.button("Capture → selected keyframe").clicked() {
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
                    session.status = format!(
                        "Captured viewport camera into keyframe #{}.",
                        session.animation.selected_keyframe
                    );
                }
            }
            if ui.button("Preview keyframe camera").clicked() {
                if let Some(patch) = session
                    .animation
                    .draft
                    .keyframes
                    .get(session.animation.selected_keyframe)
                    .and_then(|kf| kf.camera.as_ref())
                {
                    apply_viewport_camera_patch(&mut viewport.orbit, patch);
                    viewport.user_orbiting = false;
                    session.status = format!(
                        "Applied keyframe #{} camera to viewport.",
                        session.animation.selected_keyframe
                    );
                }
            }
        }
        if ui.button("Load pose camera block").clicked() {
            if let Some(patch) = &session.pose.draft.camera {
                apply_viewport_camera_patch(&mut viewport.orbit, patch);
                viewport.user_orbiting = false;
                session.status = "Applied pose camera block to viewport.".into();
            }
        }
    });
}

fn pose_panel(
    ui: &mut egui::Ui,
    session: &mut EditorSession,
    poses: &PoseLibrary,
    skeleton: &RigSkeleton,
    playback: &RigPlayback,
) {
    ui.heading("Pose");
    ui.horizontal(|ui| {
        ui.label("name");
        ui.text_edit_singleline(&mut session.pose.draft.name);
    });
    let mut new_from_idle = false;
    let mut capture_scene = false;
    ui.horizontal(|ui| {
        new_from_idle = ui.button("New").clicked();
        capture_scene = ui.button("Capture scene").clicked();
    });
    if new_from_idle {
        if let Some(idle) = poses.0.read().unwrap().get("idle") {
            session.pose.draft = idle.clone();
            session.pose.draft.name = "new_pose".into();
        } else {
            session.pose = PoseEditorState::default();
        }
        session.status = "New pose from idle template.".into();
    }
    if capture_scene {
        let snap = playback.0.current_snapshot(&skeleton.0);
        capture_scene_to_pose_draft(&skeleton.0, &snap, &mut session.pose.draft);
        session.status = "Captured current rig into draft.".into();
    }
    ui.horizontal(|ui| {
        ui.label("Load");
        let mut loaded: Option<Pose> = None;
        let mut loaded_name: Option<String> = None;
        egui::ComboBox::from_id_salt("pose_load")
            .selected_text("(choose)")
            .show_ui(ui, |ui| {
                let mut names: Vec<_> = poses.0.read().unwrap().keys().cloned().collect();
                names.sort();
                for name in names {
                    if ui.selectable_label(false, &name).clicked() {
                        if let Some(p) = poses.0.read().unwrap().get(&name) {
                            loaded = Some(p.clone());
                            loaded_name = Some(name);
                        }
                    }
                }
            });
        if let Some(p) = loaded {
            session.pose.draft = p;
            if let Some(name) = loaded_name {
                session.status = format!("Loaded pose '{name}'.");
            }
        }
        if ui.button("Duplicate").clicked() {
            session.pose.draft.name = format!("{}_copy", session.pose.draft.name);
            session.status = "Duplicated — set a new name before save.".into();
        }
    });

    ui.separator();
    ui.checkbox(&mut session.pose.symmetrical, "Symmetrical (mirror left ↔ right)");
    ui.checkbox(&mut session.pose.show_camera, "Camera block");
    ui.checkbox(&mut session.pose.show_expressions, "Expressions (v2)");

    if ui.button("Save YAML → assets/poses").clicked() {
        match save_pose(&session.pose.draft, poses) {
            Ok(path) => session.status = format!("Saved {}", path.display()),
            Err(e) => session.status = e,
        }
    }

    ui.separator();
    ui.label("Joints");
    ui.text_edit_singleline(&mut session.pose.joint_filter);
    egui::ScrollArea::vertical().show(ui, |ui| {
        for (group, names) in joints::GROUPS {
            ui.collapsing(*group, |ui| {
                for name in *names {
                    if !joints::joint_matches_filter(name, &session.pose.joint_filter) {
                        continue;
                    }
                    let selected = session.pose.selected_joint.as_deref() == Some(*name);
                    if ui.selectable_label(selected, *name).clicked() {
                        session.pose.selected_joint = Some((*name).to_string());
                    }
                }
            });
        }
    });
}

fn pose_joint_inspector(
    ui: &mut egui::Ui,
    pose: &mut PoseEditorState,
    poses: &PoseLibrary,
    expressions: &SharedExpressionState,
) {
    if let Some(joint) = pose.selected_joint.clone() {
        ui.heading(&joint);
        if let Some(hint) = posing_guide::hint_for_joint(&joint) {
            ui.label(egui::RichText::new(hint).small().weak());
        }
        let mut euler = euler_for_joint(&pose.draft, &joint);
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
                &mut pose.draft,
                &joint,
                euler,
                pose.symmetrical,
            );
        }
        if ui.button("Clear joint").clicked() {
            pose.draft.joints.remove(&joint);
            if pose.symmetrical {
                if let Some(other) = crate::editor::symmetry::counterpart_joint(&joint) {
                    pose.draft.joints.remove(&other);
                }
            }
        }
        ui.separator();
        ui.heading("Hand shapes");
        ui.label(egui::RichText::new(
            "Fist: thumb uses X/Y opposition overlay (not copied from new_pose thumb).",
        ).small().weak());
        let active_side = pose
            .selected_joint
            .as_deref()
            .and_then(side_from_joint)
            .unwrap_or(BodySide::Right);
        ui.horizontal(|ui| {
            ui.label(format!("active: {}", active_side.prefix().trim_end_matches('_')));
            let fist_ref = poses.0.read().unwrap().get(HAND_SHOT_POSE_NAME).cloned();
        for preset in HandPreset::ALL {
                if ui.button(preset.label()).clicked() {
                    apply_hand_preset(
                        &mut pose.draft,
                        active_side,
                        preset,
                        pose.symmetrical,
                        fist_ref.as_ref(),
                    );
                }
            }
        });
    } else {
        ui.label("Select a joint from the list.");
    }

    if pose.show_camera {
        ui.separator();
        ui.heading("Camera");
        let cam = pose.draft.camera.get_or_insert_with(CameraTarget::default);
        cam_option_f32(ui, "yaw_deg", &mut cam.yaw_deg, -180.0..=180.0);
        cam_option_f32(ui, "pitch_deg", &mut cam.pitch_deg, -80.0..=80.0);
        cam_option_f32(ui, "distance", &mut cam.distance, 1.2..=12.0);
        if ui.button("Reset camera block").clicked() {
            pose.draft.camera = None;
        }
    }

    if pose.show_expressions {
        ui.separator();
        ui.heading("Expressions");
        let snap = expressions.snapshot();
        if !snap.ready {
            ui.label("Load v2 VRM for expression presets.");
        } else {
            for preset in &snap.available {
                let mut weight = pose.draft.expressions.get(preset).copied().unwrap_or(0.0);
                if ui
                    .add(egui::Slider::new(&mut weight, 0.0..=1.0).text(preset))
                    .changed()
                {
                    if weight <= 1e-4 {
                        pose.draft.expressions.remove(preset);
                    } else {
                        pose.draft.expressions.insert(preset.clone(), weight);
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

fn anim_panel(
    ui: &mut egui::Ui,
    session: &mut EditorSession,
    poses: &PoseLibrary,
    animations: &AnimationLibrary,
    skeleton: &RigSkeleton,
    expressions: &SharedExpressionState,
) {
    let anim = &mut session.animation;
    ui.heading("Animation");
    ui.horizontal(|ui| {
        ui.label("name");
        ui.text_edit_singleline(&mut anim.draft.name);
    });
    ui.checkbox(&mut anim.draft.looping, "loop (YAML)");
    ui.checkbox(
        &mut anim.draft.play_automatically,
        "play automatically (bored idle)",
    );
    ui.checkbox(&mut anim.loop_playback, "Loop preview");

    ui.horizontal(|ui| {
        if ui.button("New").clicked() {
            *anim = AnimationEditorState::default();
            session.status = "New animation draft.".into();
        }
        egui::ComboBox::from_id_salt("anim_load")
            .selected_text("(load)")
            .show_ui(ui, |ui| {
                let lib = animations.0.read().unwrap();
                let mut names: Vec<_> = lib.keys().cloned().collect();
                names.sort();
                for name in names {
                    if ui.selectable_label(false, &name).clicked() {
                        let path = animation_yaml_path(
                            std::path::Path::new(crate::rig_bridge::ANIMATIONS_DIR),
                            &name,
                        );
                        anim.draft = load_animation_file(&path).unwrap_or_else(|_| {
                            animations
                                .0
                                .read()
                                .unwrap()
                                .get(&name)
                                .map(animation_to_file)
                                .unwrap_or_else(|| AnimationEditorState::default().draft)
                        });
                        anim.playhead_ms = 0;
                        anim.playing = false;
                        session.status = format!("Loaded animation '{name}'.");
                    }
                }
            });
    });

    let resolved = resolve_editor_animation(&anim.draft, poses);
    let resolve_err = resolved.as_ref().err().cloned();
    let total_ms = resolved
        .as_ref()
        .map(|a| PlaybackState::animation_playable_duration_ms(a))
        .unwrap_or(0);

    if let Some(ref e) = resolve_err {
        ui.colored_label(egui::Color32::LIGHT_RED, e);
    } else if total_ms == 0 && anim.draft.keyframes.len() < 2 {
        ui.weak("Add a second keyframe — segment timing applies on #1 and later.");
    } else if total_ms == 0 {
        ui.weak("Set segment ms on keyframe #1+ (entry #0 does not add duration).");
    }

    ui.label(format!(
        "Timeline: {total_ms} ms playable (segments on keyframes #1+; #0 is entry at t=0)"
    ));
    ui.horizontal(|ui| {
        let can_play = total_ms > 0 && resolve_err.is_none();
        if ui
            .add_enabled(can_play, egui::Button::new(if anim.playing {
                "Pause"
            } else {
                "Play"
            }))
            .clicked()
        {
            if anim.playing {
                anim.playing = false;
            } else {
                if anim.playhead_ms >= total_ms {
                    anim.playhead_ms = 0;
                }
                anim.playing = true;
            }
        }
        if ui.button("Stop").clicked() {
            anim.playing = false;
            anim.playhead_ms = 0;
        }
        ui.label(format!("{} / {} ms", anim.playhead_ms, total_ms));
    });

    if total_ms > 0 {
        anim.playhead_ms = anim.playhead_ms.min(total_ms);
        let mut playhead = anim.playhead_ms as f32;
        if ui
            .add(egui::Slider::new(&mut playhead, 0.0..=total_ms as f32).text("scrub"))
            .changed()
        {
            anim.playhead_ms = playhead.round() as u32;
            anim.playing = false;
        }
    } else {
        anim.playhead_ms = 0;
        ui.add_enabled(false, egui::Slider::new(&mut 0.0f32, 0.0..=1.0).text("scrub"));
    }

    ui.separator();
    ui.horizontal(|ui| {
        ui.label("Keyframes");
        if ui.button("+").clicked() {
            anim.draft.keyframes.push(KeyframeSpec {
                pose: Some("idle".into()),
                joints: None,
                camera: None,
                expressions: None,
                hold: None,
                duration_ms: 400,
                easing: Easing::EaseInOut,
            });
        }
    });

    let pose_names: Vec<String> = {
        let mut v: Vec<_> = poses.0.read().unwrap().keys().cloned().collect();
        v.sort();
        v
    };

    egui::ScrollArea::vertical().max_height(280.0).show(ui, |ui| {
        let mut remove_at: Option<usize> = None;
        let mut move_up: Option<usize> = None;
        let mut move_down: Option<usize> = None;
        let len = anim.draft.keyframes.len();
        for (i, kf) in anim.draft.keyframes.iter_mut().enumerate() {
            let selected = anim.selected_keyframe == i;
            ui.group(|ui| {
                ui.horizontal(|ui| {
                    if ui.selectable_label(selected, format!("#{i}")).clicked() {
                        anim.selected_keyframe = i;
                    }
                    if i > 0 && ui.small_button("↑").clicked() {
                        move_up = Some(i);
                    }
                    if i + 1 < len && ui.small_button("↓").clicked() {
                        move_down = Some(i);
                    }
                    if i == 0 {
                        ui.label(egui::RichText::new("entry").small().weak());
                    } else {
                        ui.label("segment");
                        ui.add(egui::DragValue::new(&mut kf.duration_ms).speed(10).suffix(" ms"));
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
                if ui.button("Remove").clicked() {
                    remove_at = Some(i);
                }
            });
        }
        if let Some(i) = remove_at {
            anim.draft.keyframes.remove(i);
            anim.selected_keyframe = anim.selected_keyframe.min(anim.draft.keyframes.len().saturating_sub(1));
        }
        if let Some(i) = move_up {
            anim.draft.keyframes.swap(i, i - 1);
            anim.selected_keyframe = match anim.selected_keyframe {
                x if x == i => i - 1,
                x if x == i - 1 => i,
                x => x,
            };
        }
        if let Some(i) = move_down {
            anim.draft.keyframes.swap(i, i + 1);
            anim.selected_keyframe = match anim.selected_keyframe {
                x if x == i => i + 1,
                x if x == i + 1 => i,
                x => x,
            };
        }
    });

    if let Some(kf) = anim.draft.keyframes.get(anim.selected_keyframe) {
        ui.separator();
        ui.label(format!("Keyframe #{} detail", anim.selected_keyframe));
        if let Some(expr) = &kf.expressions {
            ui.label(format!("expressions: {} keys", expr.len()));
        }
        ui.collapsing("Camera patch", |ui| {
            if let Some(kf_mut) = anim.draft.keyframes.get_mut(anim.selected_keyframe) {
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
                if let Some(kf_mut) = anim.draft.keyframes.get_mut(anim.selected_keyframe) {
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

    if ui.button("Save YAML → assets/animations").clicked() {
        match save_animation(&anim.draft, poses, animations) {
            Ok(path) => session.status = format!("Saved {}", path.display()),
            Err(e) => session.status = e,
        }
    }

    let _ = skeleton;
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

/// Click a bone marker in the viewport to select it in the pose editor (when egui is not using the pointer).
pub fn editor_bone_pick(
    mut session: ResMut<EditorSession>,
    egui_wants: Res<EguiWantsInput>,
    buttons: Res<ButtonInput<MouseButton>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    camera_entity: Res<ChoreographyCameraEntity>,
    cameras: Query<(&Camera, &GlobalTransform)>,
    rig_entities: Res<RigEntities>,
    skeleton: Res<RigSkeleton>,
    transforms: Query<&GlobalTransform>,
) {
    if !session.open || session.tab != EditorTab::Pose {
        return;
    }
    if !buttons.just_pressed(MouseButton::Left) {
        return;
    }
    if egui_wants.is_pointer_over_area() {
        return;
    }
    let Ok(window) = windows.single() else {
        return;
    };
    let Some(cursor) = window.cursor_position() else {
        return;
    };
    let Ok((camera, cam_tf)) = cameras.get(camera_entity.0) else {
        return;
    };
    let Ok(ray) = camera.viewport_to_world(cam_tf, cursor) else {
        return;
    };
    let origin = ray.origin;
    let dir = ray.direction.normalize();

    let mut best: Option<(f32, String)> = None;
    for (joint_id, entity) in &rig_entities.0 {
        let Ok(gt) = transforms.get(*entity) else {
            continue;
        };
        let p = gt.translation();
        let t = dir.dot(p - origin) / dir.dot(dir);
        if t < 0.0 {
            continue;
        }
        let closest = origin + dir * t;
        let dist = (closest - p).length();
        if dist < 0.12 {
            let name = skeleton.0.joint(*joint_id).name.clone();
            if best.as_ref().map(|(d, _)| dist < *d).unwrap_or(true) {
                best = Some((dist, name));
            }
        }
    }
    if let Some((_, name)) = best {
        session.pose.selected_joint = Some(name);
    }
}

fn digit_from_key(code: KeyCode) -> Option<u8> {
    match code {
        KeyCode::Digit1 => Some(1),
        KeyCode::Digit2 => Some(2),
        KeyCode::Digit3 => Some(3),
        KeyCode::Digit4 => Some(4),
        KeyCode::Digit5 => Some(5),
        KeyCode::Digit6 => Some(6),
        _ => None,
    }
}

/// Hand-shape shortcuts while the pose editor is focused (see right inspector).
pub fn editor_keyboard_shortcuts(
    egui_wants: Res<EguiWantsInput>,
    keys: Res<ButtonInput<KeyCode>>,
    poses: Res<PoseLibrary>,
    mut session: ResMut<EditorSession>,
) {
    if !session.open || session.tab != EditorTab::Pose {
        return;
    }
    if egui_wants.wants_keyboard_input() {
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
    ] {
        if !keys.just_pressed(code) {
            continue;
        }
        let Some(digit) = digit_from_key(code) else {
            continue;
        };
        let Some(preset) = preset_from_shortcut(digit) else {
            continue;
        };
        let symmetrical = session.pose.symmetrical;
        let fist_ref = poses.0.read().unwrap().get(HAND_SHOT_POSE_NAME).cloned();
        apply_hand_preset(
            &mut session.pose.draft,
            side,
            preset,
            symmetrical,
            fist_ref.as_ref(),
        );
        session.status = format!(
            "Hand preset '{}' on {} hand{}",
            preset.label(),
            side.prefix().trim_end_matches('_'),
            if symmetrical { " (mirrored)" } else { "" }
        );
    }
}
