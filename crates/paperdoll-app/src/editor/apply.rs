use crate::editor::session::{EditorSession, EditorTab};
use crate::editor_state::SharedEditorState;
use crate::rig_bridge::{IdleRevert, RigPlayback, RigSkeleton};
use bevy::prelude::*;
use paperdoll_rig::{PlaybackState, Pose, PoseError, ResolvedPose, Skeleton};

/// Resolved pose for live preview — same sparse semantics as `Pose::resolve` / idle
/// playback (only listed joints are written onto the mesh; v2 uses rest-relative).
pub fn expand_pose_for_preview(pose: &Pose, skeleton: &Skeleton) -> Result<ResolvedPose, PoseError> {
    pose.resolve(skeleton)
}

/// Mirrors [`EditorSession::open`] for the HTTP thread before idle-revert runs in `Update`.
pub fn sync_editor_http_lock(session: Res<EditorSession>, shared: Res<SharedEditorState>) {
    shared.set_active(session.open);
}

/// While the editor is open, push the authored preview onto the rig and pause idle
/// revert / agent timeouts.
pub fn editor_apply_preview(
    time: Res<Time>,
    skeleton: Res<RigSkeleton>,
    mut session: ResMut<EditorSession>,
    mut playback: ResMut<RigPlayback>,
    mut idle: ResMut<IdleRevert>,
    poses: Res<crate::rig_bridge::PoseLibrary>,
) {
    if !session.open {
        return;
    }

    idle.hold_for_editor(time.elapsed_secs());

    let poses_guard = poses.0.read().unwrap();
    if session.tab == EditorTab::Pose
        && session.pose.draft.joints.is_empty()
        && !session.pose.auto_fill_done
    {
        if let Some(idle_pose) = poses_guard.get("idle") {
            let existing: std::collections::HashSet<String> =
                poses_guard.keys().cloned().collect();
            session.pose.draft = idle_pose.clone();
            session.pose.draft.name =
                crate::editor::session::unique_name("new_pose", &existing);
            session.pose.auto_fill_done = true;
            session.pose.checkpoint();
        }
    }

    let preview = match session.tab {
        EditorTab::Pose => {
            session.animation.playing = false;
            match expand_pose_for_preview(&session.pose.draft, &skeleton.0) {
                Ok(resolved) => resolved,
                Err(e) => {
                    session.error(format!("pose: {e}"));
                    return;
                }
            }
        }
        EditorTab::Animation => {
            let resolved_anim = match paperdoll_rig::resolve_animation(
                session.animation.draft.clone(),
                &poses_guard,
            ) {
                Ok(a) => a,
                Err(e) => {
                    session.error(format!("animation: {e}"));
                    session.animation.playing = false;
                    session.animation.playhead_ms = 0;
                    return;
                }
            };
            let total = PlaybackState::animation_playable_duration_ms(&resolved_anim);

            if total == 0 {
                session.animation.playhead_ms = 0;
                session.animation.playing = false;
            } else {
                let dt_ms = (time.delta_secs() * 1000.0).round() as u32;
                if session.animation.playing && dt_ms > 0 {
                    session.animation.playhead_ms = session
                        .animation
                        .playhead_ms
                        .saturating_add(dt_ms);
                }
                if session.animation.playing {
                    if session.animation.loop_playback {
                        session.animation.playhead_ms %= total;
                    } else if session.animation.playhead_ms >= total {
                        session.animation.playhead_ms = total;
                        session.animation.playing = false;
                    }
                }
            }

            let mut anim = resolved_anim;
            anim.looping = session.animation.loop_playback;
            PlaybackState::pose_at_animation_time(
                &skeleton.0,
                &anim,
                session.animation.playhead_ms,
            )
        }
    };
    drop(poses_guard);

    match &mut playback.0 {
        PlaybackState::Idle { held } => *held = preview,
        _ => playback.0 = PlaybackState::Idle { held: preview },
    }
}

/// Capture the current rig snapshot into the pose draft (sparse: only joints that
/// differ from rest rotation).
pub fn capture_scene_to_pose_draft(
    skeleton: &paperdoll_rig::Skeleton,
    snapshot: &ResolvedPose,
    draft: &mut paperdoll_rig::Pose,
) {
    use paperdoll_rig::EulerDeg;
    use std::collections::HashMap;

    let mut joints = HashMap::new();
    for (id, rot) in &snapshot.joint_rotations {
        let rest = skeleton.joint(*id).local_rotation;
        if rot.angle_between(rest) > 0.01 {
            let name = skeleton.joint(*id).name.clone();
            joints.insert(
                name,
                paperdoll_rig::JointTarget {
                    rotation_deg: Some(EulerDeg::from_quat(*rot)),
                    rotation_quat: None,
                    translation: None,
                },
            );
        }
    }
    draft.joints = joints;
    draft.expressions = snapshot.expressions.clone();
    if snapshot.camera != paperdoll_rig::DEFAULT_CAMERA {
        draft.camera = Some(paperdoll_rig::CameraTarget {
            yaw_deg: Some(snapshot.camera.yaw_deg),
            pitch_deg: Some(snapshot.camera.pitch_deg),
            distance: Some(snapshot.camera.distance),
            look_at: Some(snapshot.camera.look_at),
        });
    }
}
