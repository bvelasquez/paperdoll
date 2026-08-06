//! On-viewport skeleton overlay + direct manipulation for the pose editor.
//!
//! Turns the 3D stage into the primary editing surface:
//!
//! * **Skeleton overlay** — every bone is drawn as a line with a joint marker,
//!   always rendered in front of the doll mesh so the rig reads through the body.
//! * **Click to select** — clicking a bone segment or joint marker selects that
//!   joint in the pose editor (replaces the old invisible ray-pick).
//! * **Rotation gizmo** — the selected joint gets an X/Y/Z ring gizmo; drag a ring
//!   to rotate that joint directly with the mouse instead of fighting sliders.
//! * **Translation arrows** — joints that support offsets (pupils, blush, pelvis)
//!   also get X/Y/Z arrows; drag an arrow to translate.
//! * **Left-drag orbit** — dragging on empty stage space orbits the camera (in
//!   addition to the existing right/middle-drag + scroll zoom).
//!
//! All edits write straight into the pose draft, so the existing preview pipeline
//! (`editor_apply_preview` → `advance_playback`) shows the result live, and the
//! sliders / save flow continue to work on exactly the same data.

use super::session::{euler_for_joint, set_joint_euler, EditorSession, EditorTab, PoseEditorState};
use crate::camera_controls::ViewportCamera;
use crate::rig_bridge::{ChoreographyCameraEntity, RigEntities, RigSkeleton};
use bevy::gizmos::prelude::*;
use bevy::input::mouse::MouseButton;
use bevy::math::Isometry3d;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use bevy_egui::egui;
use bevy_egui::input::EguiWantsInput;
use bevy_egui::EguiContexts;
use paperdoll_rig::{EulerDeg, JointId, Pose};

// ---------------------------------------------------------------------------
// Resources
// ---------------------------------------------------------------------------

/// Editor display preferences for the 3D stage. UI toggles live here; the viewport
/// system reads them each frame. `pending_focus` is a one-shot command the UI sets
/// ("frame this joint") that the viewport consumes.
#[derive(Resource)]
pub struct ViewportPrefs {
    pub show_skeleton: bool,
    pub show_gizmo: bool,
    pub left_drag_orbit: bool,
    /// Hide the doll's skin/mesh so only the skeleton overlay shows (X-ray rig work).
    pub hide_mesh: bool,
    pub pending_focus: Option<String>,
}

impl Default for ViewportPrefs {
    fn default() -> Self {
        Self {
            show_skeleton: true,
            show_gizmo: true,
            left_drag_orbit: true,
            hide_mesh: false,
            pending_focus: None,
        }
    }
}

/// Transient interaction state for the viewport manipulator (selection press
/// tracking, hover highlight, left-drag orbit, corner-widget drag).
#[derive(Resource, Default)]
pub struct GizmoState {
    hover_joint: Option<String>,
    press: Option<PressInfo>,
    nav: bool,
    last_nav_cursor: Option<Vec2>,
    /// Active drag inside the corner rotation widget (egui pass).
    widget_drag: Option<WidgetDrag>,
}

/// A drag being performed on the corner rotation widget. Rotation drags rebuild
/// `start_q * Quat::from_axis_angle(local_axis, accum)` each frame; translation
/// drags rebuild `start_translation + accum` along the axis.
struct WidgetDrag {
    joint: String,
    kind: DragKind,
    start_q: Quat,
    accum: f32,
    start_translation: [f32; 3],
    /// Screen-space unit direction the grabbed ring point moves for positive rotation.
    tangent_px: Vec2,
    /// Screen-space unit direction along the axis (translation drags).
    axis_px: Vec2,
    /// Ring radius in screen pixels (rotation) or px-per-meter (translation).
    scale_px: f32,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum DragKind {
    RotateX,
    RotateY,
    RotateZ,
    TranslateX,
    TranslateY,
    TranslateZ,
}

impl DragKind {
    fn is_translate(self) -> bool {
        matches!(
            self,
            DragKind::TranslateX | DragKind::TranslateY | DragKind::TranslateZ
        )
    }

    fn local_axis(self) -> Vec3 {
        match self {
            DragKind::RotateX | DragKind::TranslateX => Vec3::X,
            DragKind::RotateY | DragKind::TranslateY => Vec3::Y,
            _ => Vec3::Z,
        }
    }

    fn axis_index(self) -> usize {
        match self {
            DragKind::RotateX | DragKind::TranslateX => 0,
            DragKind::RotateY | DragKind::TranslateY => 1,
            _ => 2,
        }
    }
}

/// A left-button press being tracked: either a candidate click-to-select, or (when
/// `joint_under` is None) an immediate orbit drag.
struct PressInfo {
    joint_under: Option<String>,
    start: Vec2,
}

// ---------------------------------------------------------------------------
// Colors
// ---------------------------------------------------------------------------

const AXIS_X_EGUI: egui::Color32 = egui::Color32::from_rgb(242, 66, 56);
const AXIS_Y_EGUI: egui::Color32 = egui::Color32::from_rgb(77, 224, 92);
const AXIS_Z_EGUI: egui::Color32 = egui::Color32::from_rgb(82, 140, 255);
const BONE_COLOR: Color = Color::srgba(0.62, 0.72, 0.95, 0.9);
const MARKER_COLOR: Color = Color::srgba(0.55, 0.62, 0.82, 0.75);
/// Ghosted skeleton used for everything outside the selected joint's chain.
const BONE_DIM: Color = Color::srgba(0.45, 0.52, 0.72, 0.16);
const MARKER_DIM: Color = Color::srgba(0.45, 0.5, 0.66, 0.2);
const SELECT_COLOR: Color = Color::srgb(1.0, 0.80, 0.20);
const HOVER_COLOR: Color = Color::srgb(1.0, 0.58, 0.20);

/// How many screen pixels of drag turns a press into an orbit instead of a click.
const NAV_DRAG_THRESHOLD_PX: f32 = 8.0;

// ---------------------------------------------------------------------------
// System
// ---------------------------------------------------------------------------

/// Hide/unhide the doll's skin (v1 procedural body / v2 skinned VRM mesh) so the
/// pose editor can work against the bare skeleton. Bone gizmos render independently
/// of mesh visibility, so the rig stays fully visible in either mode.
pub fn apply_hide_mesh(
    prefs: Res<ViewportPrefs>,
    mut roots: Query<&mut Visibility, With<crate::rig_bridge::DollVisualRoot>>,
) {
    let target = if prefs.hide_mesh {
        Visibility::Hidden
    } else {
        Visibility::Visible
    };
    for mut vis in &mut roots {
        if *vis != target {
            *vis = target;
        }
    }
}

/// Make the gizmo depth bias negative so the skeleton overlay always renders in
/// front of the doll mesh (X-ray style rig, readable through the body).
pub fn setup_gizmo_config(mut store: ResMut<GizmoConfigStore>) {
    store
        .config_mut::<DefaultGizmoConfigGroup>()
        .0
        .depth_bias = -0.95;
}

/// Main viewport manipulator: draws the skeleton + gizmo, then handles pointer
/// interaction (select / rotate / translate / orbit). Runs in `PostUpdate` before
/// `editor_apply_preview` so a gizmo edit lands in the draft and previews the same
/// frame, and after egui's input pass so `EguiWantsInput` is current.
#[allow(clippy::too_many_arguments)]
pub fn editor_viewport_manip(
    mut gizmos: Gizmos,
    mut session: ResMut<EditorSession>,
    mut viewport: ResMut<ViewportCamera>,
    mut prefs: ResMut<ViewportPrefs>,
    mut state: ResMut<GizmoState>,
    egui_wants: Res<EguiWantsInput>,
    buttons: Res<ButtonInput<MouseButton>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    camera_entity: Res<ChoreographyCameraEntity>,
    cameras: Query<(&Camera, &GlobalTransform)>,
    rig_entities: Res<RigEntities>,
    skeleton: Res<RigSkeleton>,
    transforms: Query<&GlobalTransform>,
) {
    if !session.open {
        state.widget_drag = None;
        state.press = None;
        state.nav = false;
        state.last_nav_cursor = None;
        state.hover_joint = None;
        return;
    }

    let Ok(window) = windows.single() else {
        return;
    };
    let Ok((camera, cam_tf)) = cameras.get(camera_entity.0) else {
        return;
    };

    // 1) One-shot camera focus requested from the UI (e.g. "Focus right hand").
    if let Some(name) = prefs.pending_focus.take() {
        if let Some(pos) = joint_world_pos(&rig_entities, &skeleton, &transforms, &name) {
            viewport.orbit.look_at = [pos.x, pos.y, pos.z];
            viewport.orbit.distance = focus_distance(&name);
            viewport.user_orbiting = true;
        }
    }

    // 2) Skeleton overlay (both tabs — useful while scrubbing animations too).
    //    Drawn even when the cursor is outside the window, so the rig stays visible
    //    while the user works in panels or the window lacks focus.
    if prefs.show_skeleton {
        draw_skeleton(&mut gizmos, &skeleton, &rig_entities, &transforms, &session, &state);
    }

    // 3) Interaction (pose tab only) — needs the cursor position.
    if session.tab != EditorTab::Pose {
        return;
    }
    let Some(cursor) = window.cursor_position() else {
        state.widget_drag = None;
        state.press = None;
        state.nav = false;
        state.last_nav_cursor = None;
        state.hover_joint = None;
        return;
    };
    let pointer_over_egui = egui_wants.is_pointer_over_area() || egui_wants.wants_pointer_input();

    // Left press: joint pick (click-to-select) or empty-space orbit drag. The corner
    // rotation widget owns its own pointer handling (egui pass), so it never reaches
    // here while being dragged.
    if buttons.just_pressed(MouseButton::Left) && !pointer_over_egui {
        state.press = None;
        state.nav = false;
        state.last_nav_cursor = None;

        let ray = camera.viewport_to_world(cam_tf, cursor).ok();
        let joint_under = ray
            .as_ref()
            .and_then(|r| pick_joint(r, &rig_entities, &skeleton, &transforms).map(|(n, _)| n));
        let joint_under_none = joint_under.is_none();
        state.press = Some(PressInfo {
            joint_under,
            start: cursor,
        });
        state.nav = joint_under_none && prefs.left_drag_orbit;
        if state.nav {
            state.last_nav_cursor = Some(cursor);
        }
        return;
    }

    // Held left: orbit (started on empty space, or converted once a press dragged).
    if buttons.pressed(MouseButton::Left) && !pointer_over_egui {
        if let Some(press) = &state.press {
            if press.joint_under.is_some() && (cursor - press.start).length() > NAV_DRAG_THRESHOLD_PX {
                state.press = None;
                state.nav = true;
            }
        }
        if state.nav {
            let last = state.last_nav_cursor.unwrap_or(cursor);
            let delta = cursor - last;
            if delta != Vec2::ZERO {
                viewport.orbit.yaw_deg += delta.x * 0.35;
                viewport.orbit.pitch_deg =
                    (viewport.orbit.pitch_deg - delta.y * 0.35).clamp(-80.0, 80.0);
                viewport.user_orbiting = true;
            }
            state.last_nav_cursor = Some(cursor);
        }
    } else {
        state.last_nav_cursor = None;
    }

    // Left release: a click (no meaningful drag) selects the joint under the cursor.
    if buttons.just_released(MouseButton::Left) {
        if let Some(press) = state.press.take() {
            let moved = (cursor - press.start).length() > NAV_DRAG_THRESHOLD_PX;
            if !moved {
                if let Some(joint) = press.joint_under {
                    session.pose.selected_joint = Some(joint);
                } else if let Some(joint) = state.hover_joint.clone() {
                    session.pose.selected_joint = Some(joint);
                }
            }
        }
        state.nav = false;
    }

    // Hover highlight (also used as a fallback click target). Only while the
    // pointer is over the stage — never highlight joints behind the egui panels.
    state.hover_joint = if pointer_over_egui {
        None
    } else {
        camera
            .viewport_to_world(cam_tf, cursor)
            .ok()
            .and_then(|r| pick_joint(&r, &rig_entities, &skeleton, &transforms).map(|(n, _)| n))
    };
}

// ---------------------------------------------------------------------------
// Corner rotation widget
// ---------------------------------------------------------------------------

/// A compact per-axis rotation control anchored at the bottom of the stage
/// (instead of floating rings over the character). Three clearly separated drag
/// strips — X (red), Y (green), Z (blue) — one per axis, each showing the live
/// angle. Drag a strip to rotate the selected joint about that axis; there's no
/// circle-picking to get wrong regardless of how the stage camera is oriented.
/// Every frame writes straight into the pose draft, so the preview updates live
/// and the sliders in the right panel stay in sync.
pub fn editor_corner_widget(
    mut contexts: EguiContexts,
    mut session: ResMut<EditorSession>,
    prefs: Res<ViewportPrefs>,
    mut state: ResMut<GizmoState>,
) {
    if !session.open || session.tab != EditorTab::Pose || !prefs.show_gizmo {
        state.widget_drag = None;
        return;
    }
    let Some(name) = session.pose.selected_joint.clone() else {
        state.widget_drag = None;
        return;
    };
    let Ok(ctx) = contexts.ctx_mut() else {
        return;
    };
    let symmetrical = session.pose.symmetrical;

    egui::Area::new(egui::Id::new("editor_axis_strips"))
        .anchor(egui::Align2::CENTER_BOTTOM, [0.0, -10.0])
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            egui::Frame::default()
                .fill(egui::Color32::from_rgba_unmultiplied(22, 22, 26, 230))
                .rounding(10.0)
                .inner_margin(10.0)
                .show(ui, |ui| {
                    ui.set_max_width(210.0);
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new(format!("rotate {name}"))
                                .size(11.0)
                                .color(egui::Color32::from_gray(210)),
                        );
                        if ui
                            .small_button("✕")
                            .on_hover_text("Deselect joint")
                            .clicked()
                        {
                            session.pose.selected_joint = None;
                        }
                    });
                    ui.label(
                        egui::RichText::new("drag a row to rotate that axis")
                            .size(9.5)
                            .weak(),
                    );
                    ui.add_space(2.0);
                    for (axis, color, label) in [
                        (0usize, AXIS_X_EGUI, "X"),
                        (1usize, AXIS_Y_EGUI, "Y"),
                        (2usize, AXIS_Z_EGUI, "Z"),
                    ] {
                        axis_strip(ui, &mut session.pose, &name, axis, color, label, &mut state, symmetrical);
                    }
                });
        });
}

/// One axis row: colored track + draggable knob + live angle readout.
fn axis_strip(
    ui: &mut egui::Ui,
    pose: &mut PoseEditorState,
    name: &str,
    axis: usize,
    color: egui::Color32,
    label: &str,
    state: &mut GizmoState,
    symmetrical: bool,
) {
    let euler = euler_for_joint(&pose.draft, name);
    let angle = match axis {
        0 => euler.x,
        1 => euler.y,
        _ => euler.z,
    };
    let kind = match axis {
        0 => DragKind::RotateX,
        1 => DragKind::RotateY,
        _ => DragKind::RotateZ,
    };

    let (rect, response) = ui.allocate_exact_size(egui::vec2(190.0, 26.0), egui::Sense::drag());
    let painter = ui.painter_at(rect);

    // Track.
    painter.rect_filled(
        rect,
        6.0,
        egui::Color32::from_rgba_unmultiplied(38, 40, 46, 255),
    );
    // Active tint while dragging this axis.
    let active = state.widget_drag.as_ref().map(|d| d.kind == kind).unwrap_or(false);
    if active {
        painter.rect_stroke(
            rect,
            6.0,
            egui::Stroke::new(1.5, color),
            egui::StrokeKind::Inside,
        );
    }

    // Axis label.
    painter.text(
        egui::pos2(rect.left() + 14.0, rect.center().y),
        egui::Align2::CENTER_CENTER,
        label,
        egui::FontId::proportional(12.0),
        color,
    );

    // Knob at a position proportional to the (wrapped) angle.
    let t = ((angle.rem_euclid(360.0)) / 360.0) as f32;
    let track_left = rect.left() + 30.0;
    let track_right = rect.right() - 46.0;
    let knob_x = track_left + t * (track_right - track_left);
    let knob_y = rect.center().y;
    painter.circle_filled(egui::pos2(knob_x, knob_y), 7.0, color);
    painter.circle_stroke(
        egui::pos2(knob_x, knob_y),
        7.0,
        egui::Stroke::new(1.0, egui::Color32::from_white_alpha(120)),
    );

    // Live angle readout.
    painter.text(
        egui::pos2(rect.right() - 8.0, rect.center().y),
        egui::Align2::RIGHT_CENTER,
        format!("{angle:+.0}°"),
        egui::FontId::proportional(11.0),
        egui::Color32::from_gray(215),
    );

    // Drag interaction: accumulate from the angle at grab time.
    if response.drag_started() {
        state.widget_drag = Some(WidgetDrag {
            joint: name.to_string(),
            kind,
            start_q: euler.to_quat(),
            accum: 0.0,
            start_translation: [0.0; 3],
            tangent_px: Vec2::X,
            axis_px: Vec2::ZERO,
            // ~0.35°/px: a full strip drag is roughly a 180° turn.
            scale_px: 165.0,
        });
    }
    if response.dragged() {
        if let Some(mut wd) = state.widget_drag.take() {
            let delta = response.drag_delta();
            wd.accum += delta.x / wd.scale_px;
            let q = wd.start_q * Quat::from_axis_angle(wd.kind.local_axis(), wd.accum);
            let euler = EulerDeg::from_quat(q);
            set_joint_euler(&mut pose.draft, &wd.joint, euler, symmetrical);
            state.widget_drag = Some(wd);
        }
    }
    if response.drag_stopped() {
        state.widget_drag = None;
    }
    if response.hovered() {
        let _ = ui.output_mut(|o| o.cursor_icon = egui::CursorIcon::ResizeHorizontal);
    }
}

// ---------------------------------------------------------------------------
// Drawing
// ---------------------------------------------------------------------------

fn draw_skeleton(
    gizmos: &mut Gizmos,
    skeleton: &RigSkeleton,
    rig: &RigEntities,
    transforms: &Query<&GlobalTransform>,
    session: &EditorSession,
    state: &GizmoState,
) {
    let selected = session.pose.selected_joint.as_deref();
    let hover = state.hover_joint.as_deref();

    // With a joint selected, everything outside its immediate chain (selected +
    // parent + children) dims to a ghost, so the active part and its rotation
    // gizmo are easy to see instead of being buried in the full skeleton.
    let dim_unrelated = selected.is_some();
    let mut chain: std::collections::HashSet<JointId> = Default::default();
    if let Some(sel) = selected.and_then(|n| skeleton.0.joint_by_name(n)) {
        chain.insert(sel);
        if let Some(p) = skeleton.0.joint(sel).parent {
            chain.insert(p);
        }
        for c in &skeleton.0.joint(sel).children {
            chain.insert(*c);
        }
    }
    let in_chain = |id: JointId| chain.contains(&id);

    for (id, joint) in skeleton.0.joints() {
        let Some(&entity) = rig.0.get(&id) else { continue };
        let Ok(gt) = transforms.get(entity) else { continue };
        let p = gt.translation();
        let dimmed = dim_unrelated && !in_chain(id);

        // Bone segment from parent to this joint.
        if let Some(pid) = joint.parent {
            if let Some(&pe) = rig.0.get(&pid) {
                if let Ok(pgt) = transforms.get(pe) {
                    let color = if selected == Some(joint.name.as_str()) {
                        SELECT_COLOR
                    } else if hover == Some(joint.name.as_str()) {
                        HOVER_COLOR
                    } else if dimmed {
                        BONE_DIM
                    } else {
                        BONE_COLOR
                    };
                    gizmos.line(pgt.translation(), p, color);
                }
            }
        }

        // Joint marker.
        let r = (joint.radius * 1.15).clamp(0.014, 0.06);
        if selected == Some(joint.name.as_str()) {
            gizmos.sphere(
                Isometry3d::from_translation(p),
                r * 1.9,
                Color::srgba(1.0, 0.8, 0.2, 0.3),
            );
            gizmos.sphere(Isometry3d::from_translation(p), r, SELECT_COLOR);
        } else if hover == Some(joint.name.as_str()) {
            gizmos.sphere(Isometry3d::from_translation(p), r * 1.4, HOVER_COLOR);
        } else if dimmed {
            gizmos.sphere(Isometry3d::from_translation(p), r * 0.55, MARKER_DIM);
        } else {
            gizmos.sphere(Isometry3d::from_translation(p), r, MARKER_COLOR);
        }
    }
}

// ---------------------------------------------------------------------------
// World lookups
// ---------------------------------------------------------------------------

fn joint_world_pos(
    rig: &RigEntities,
    skeleton: &RigSkeleton,
    transforms: &Query<&GlobalTransform>,
    name: &str,
) -> Option<Vec3> {
    let id = skeleton.0.joint_by_name(name)?;
    let entity = *rig.0.get(&id)?;
    let gt = transforms.get(entity).ok()?;
    Some(gt.translation())
}

fn focus_distance(name: &str) -> f32 {
    if name.contains("_hand") {
        0.85
    } else if name.contains("_proximal")
        || name.contains("_intermediate")
        || name.contains("_distal")
        || name.contains("_metacarpal")
    {
        0.45
    } else if name == "head" || name.contains("_eye") || name.contains("eyebrow") {
        0.75
    } else {
        1.6
    }
}

fn joint_can_translate(draft: &Pose, name: &str) -> bool {
    draft
        .joints
        .get(name)
        .map(|t| t.translation.is_some())
        .unwrap_or(false)
        || name.ends_with("_pupil")
        || name.ends_with("_blush")
        || name == "pelvis"
}

/// Distance from a ray to a segment (returns distance + ray parameter t).
fn ray_segment_dist(origin: Vec3, dir: Vec3, a: Vec3, b: Vec3) -> (f32, f32) {
    let u = b - a;
    let v = dir;
    let w = a - origin;
    let au = u.dot(u);
    let buv = u.dot(v);
    let cv = v.dot(v);
    let duw = u.dot(w);
    let evw = v.dot(w);
    let denom = au * cv - buv * buv;
    let s = if denom.abs() > 1e-8 {
        ((buv * evw - cv * duw) / denom).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let t = (buv * s + evw) / cv;
    let closest_seg = a + u * s;
    let closest_ray = origin + v * t;
    (closest_seg.distance(closest_ray), t.max(0.0))
}

fn ray_point_dist(origin: Vec3, dir: Vec3, p: Vec3) -> (f32, f32) {
    let t = (p - origin).dot(dir) / dir.length_squared();
    ((origin + dir * t).distance(p), t.max(0.0))
}

/// Nearest joint/bone under the cursor ray. Returns (joint name, 3D distance).
fn pick_joint(
    ray: &Ray3d,
    rig: &RigEntities,
    skeleton: &RigSkeleton,
    transforms: &Query<&GlobalTransform>,
) -> Option<(String, f32)> {
    let origin = ray.origin;
    let dir: Vec3 = *ray.direction;
    let mut best: Option<(String, f32, f32)> = None;
    for (_id, joint) in skeleton.0.joints() {
        let Some(&entity) = rig.0.get(&_id) else { continue };
        let Ok(gt) = transforms.get(entity) else { continue };
        let p = gt.translation();
        let parent_p = joint
            .parent
            .and_then(|pid| rig.0.get(&pid))
            .and_then(|&pe| transforms.get(pe).ok())
            .map(|pg| pg.translation());
        let (dist, t) = match parent_p {
            Some(pp) => ray_segment_dist(origin, dir, pp, p),
            None => ray_point_dist(origin, dir, p),
        };
        let threshold = if parent_p.is_some() { 0.05 } else { 0.06 };
        if dist < threshold {
            let better = best
                .as_ref()
                .map(|(_, d, t0)| dist < *d - 1e-4 || ((dist - *d).abs() <= 1e-4 && t < *t0))
                .unwrap_or(true);
            if better {
                best = Some((joint.name.clone(), dist, t));
            }
        }
    }
    best.map(|(n, d, _)| (n, d))
}


// ---------------------------------------------------------------------------
// Draft writes
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::math::Ray3d;
    use std::collections::HashMap;

    fn ray(o: [f32; 3], d: [f32; 3]) -> Ray3d {
        Ray3d::new(Vec3::from(o), Dir3::new(Vec3::from(d)).unwrap())
    }

    #[test]
    fn ray_segment_dist_hits_midpoint() {
        // Ray along +Z through the origin; segment along X at z=1.
        let (dist, t) = ray_segment_dist(
            Vec3::ZERO,
            Vec3::Z,
            Vec3::new(-1.0, 0.0, 1.0),
            Vec3::new(1.0, 0.0, 1.0),
        );
        assert!((dist - 0.0).abs() < 1e-5, "dist={dist}");
        assert!((t - 1.0).abs() < 1e-5, "t={t}");
    }

    #[test]
    fn ray_segment_dist_misses_off_axis() {
        // Ray along +Z at x=2; segment along X at z=1, x from -1..1 → closest x=1.
        let (dist, _) = ray_segment_dist(
            Vec3::new(2.0, 0.0, 0.0),
            Vec3::Z,
            Vec3::new(-1.0, 0.0, 1.0),
            Vec3::new(1.0, 0.0, 1.0),
        );
        assert!((dist - 1.0).abs() < 1e-5, "dist={dist}");
    }

    #[test]
    fn ray_point_dist_projects_perpendicular() {
        let (dist, t) = ray_point_dist(
            Vec3::ZERO,
            Vec3::Z,
            Vec3::new(0.3, -0.2, 2.0),
        );
        assert!((dist - (0.3f32 * 0.3 + 0.2 * 0.2).sqrt()).abs() < 1e-5);
        assert!((t - 2.0).abs() < 1e-5);
    }

    #[test]
    fn local_axis_rotation_keeps_ring_axis_fixed() {
        // The gizmo rotates about a LOCAL axis: q' = q * Rx(a). The world direction of
        // that local axis (q * X) must be invariant under the rotation, which is what
        // makes the grabbed ring stay put on screen while the bone spins.
        let q = Quat::from_euler(bevy::math::EulerRot::XYZ, 0.4, -0.7, 1.1);
        let axis_world_before = q * Vec3::X;
        for step in [0.1_f32, -0.5, 2.0] {
            let q2 = q * Quat::from_axis_angle(Vec3::X, step);
            let axis_world_after = q2 * Vec3::X;
            assert!(
                axis_world_before.angle_between(axis_world_after) < 1e-5,
                "local X axis moved under right-multiply rotation"
            );
        }
    }

    #[test]
    fn euler_roundtrip_preserves_drag_accumulation() {
        // Drag math: q = start_q * Rx(accum), stored as euler. Reading the euler back
        // must reproduce the same quaternion (the sliders show the same value too).
        let start_q = Quat::from_euler(bevy::math::EulerRot::XYZ, 0.2, 0.3, 0.4);
        for accum in [0.0_f32, 0.5, -1.2] {
            let q = start_q * Quat::from_axis_angle(Vec3::Y, accum);
            let euler = EulerDeg::from_quat(q);
            let q_back = euler.to_quat();
            assert!(
                q.angle_between(q_back) < 1e-3,
                "euler roundtrip drift at accum={accum}"
            );
        }
    }

}
