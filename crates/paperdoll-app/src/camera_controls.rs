//! Viewport orbit camera: user drag/zoom plus sync from playback choreography.

use crate::editor::{EditorSession, EditorTab};
use crate::rig_bridge::ChoreographyCameraEntity;
use bevy::input::mouse::{MouseButton, MouseWheel};
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use bevy_egui::input::EguiWantsInput;
use paperdoll_rig::ResolvedCamera;

/// Live orbit framing for the primary window camera. Updated from pose/animation
/// playback unless the user is actively orbiting.
#[derive(Resource, Clone, Copy)]
pub struct ViewportCamera {
    pub orbit: ResolvedCamera,
    /// True while the user holds the orbit mouse button (keeps playback from snapping back).
    pub user_orbiting: bool,
}

impl Default for ViewportCamera {
    fn default() -> Self {
        Self {
            orbit: paperdoll_rig::DEFAULT_CAMERA,
            user_orbiting: false,
        }
    }
}

#[derive(Resource, Default)]
pub struct OrbitPointerState {
    pub last_cursor: Option<Vec2>,
}

/// Right-drag to orbit, scroll to zoom in the 3D viewport. Pointer input is skipped over
/// egui panels, but [`ViewportCamera::orbit`] is always applied (stage-camera sliders live in UI).
pub fn viewport_camera_controls(
    buttons: Res<ButtonInput<MouseButton>>,
    mut scroll: MessageReader<MouseWheel>,
    windows: Query<&Window, With<PrimaryWindow>>,
    egui_wants: Res<EguiWantsInput>,
    mut viewport: ResMut<ViewportCamera>,
    mut pointer: ResMut<OrbitPointerState>,
    camera_entity: Res<ChoreographyCameraEntity>,
    mut transforms: Query<&mut Transform>,
) {
    let pointer_over_ui =
        egui_wants.is_pointer_over_area() || egui_wants.wants_pointer_input();

    if !pointer_over_ui {
        let Ok(window) = windows.single() else {
            write_viewport_camera_transform(&viewport, camera_entity, &mut transforms);
            return;
        };

        let orbit_button =
            buttons.pressed(MouseButton::Right) || buttons.pressed(MouseButton::Middle);

        if orbit_button {
            if let Some(pos) = window.cursor_position() {
                if let Some(last) = pointer.last_cursor {
                    let delta = pos - last;
                    if delta != Vec2::ZERO {
                        viewport.user_orbiting = true;
                        let sensitivity = 0.35;
                        viewport.orbit.yaw_deg += delta.x * sensitivity;
                        viewport.orbit.pitch_deg =
                            (viewport.orbit.pitch_deg - delta.y * sensitivity).clamp(-80.0, 80.0);
                    }
                }
                pointer.last_cursor = Some(pos);
            }
        } else {
            pointer.last_cursor = None;
            if !buttons.pressed(MouseButton::Left) {
                viewport.user_orbiting = false;
            }
        }

        for ev in scroll.read() {
            let zoom = 1.0 - ev.y * 0.08;
            viewport.orbit.distance = (viewport.orbit.distance * zoom).clamp(1.2, 12.0);
            viewport.user_orbiting = true;
        }

        if buttons.just_released(MouseButton::Right) || buttons.just_released(MouseButton::Middle)
        {
            viewport.user_orbiting = false;
            pointer.last_cursor = None;
        }
    } else if !buttons.pressed(MouseButton::Right) && !buttons.pressed(MouseButton::Middle) {
        pointer.last_cursor = None;
    }

    write_viewport_camera_transform(&viewport, camera_entity, &mut transforms);
}

fn write_viewport_camera_transform(
    viewport: &ViewportCamera,
    camera_entity: Res<ChoreographyCameraEntity>,
    transforms: &mut Query<&mut Transform>,
) {
    if let Ok(mut tf) = transforms.get_mut(camera_entity.0) {
        let eye = viewport.orbit.eye();
        let look_at = viewport.orbit.look_at_vec();
        *tf = Transform::from_translation(eye).looking_at(look_at, Vec3::Y);
    }
}

/// After playback writes choreography, follow the sampled camera unless the user is orbiting.
pub fn sync_viewport_from_choreography(
    session: Res<EditorSession>,
    mut viewport: ResMut<ViewportCamera>,
    playback: Res<crate::rig_bridge::RigPlayback>,
    skeleton: Res<crate::rig_bridge::RigSkeleton>,
) {
    if session.open && session.tab == EditorTab::Pose {
        return;
    }
    if viewport.user_orbiting {
        return;
    }
    let snap = playback.0.current_snapshot(&skeleton.0);
    viewport.orbit = snap.camera;
}

/// Apply a sparse YAML `camera:` patch onto the live viewport orbit.
pub fn apply_viewport_camera_patch(cam: &mut ResolvedCamera, patch: &paperdoll_rig::CameraTarget) {
    *cam = cam.with_patch(patch);
}
