use crate::animation::{Animation, Easing};
use crate::camera::blend_cameras;
use crate::pose::{Pose, ResolvedPose};
use crate::skeleton::{JointId, Skeleton};
use glam::{Quat, Vec3};
use std::collections::{HashMap, HashSet};

/// Blend every joint present in `from` or `to` toward `to` by `eased_t` (already run
/// through an easing curve, not raw linear time). A joint present in only one of the
/// two poses falls back to the skeleton's rest rotation on the other side, so a pose
/// can be sparse without snapping unlisted joints.
pub fn blend_poses(
    skeleton: &Skeleton,
    from: &ResolvedPose,
    to: &ResolvedPose,
    eased_t: f32,
    vrm_local_rotations: bool,
) -> HashMap<JointId, Quat> {
    let joint_ids: HashSet<JointId> = from
        .joint_rotations
        .keys()
        .chain(to.joint_rotations.keys())
        .copied()
        .collect();

    let mut result = HashMap::with_capacity(joint_ids.len());
    for id in joint_ids {
        let rest = if vrm_local_rotations {
            Quat::IDENTITY
        } else {
            skeleton.joint(id).local_rotation
        };
        let from_rot = from.joint_rotations.get(&id).copied().unwrap_or(rest);
        let to_rot = to.joint_rotations.get(&id).copied().unwrap_or(rest);
        result.insert(id, from_rot.slerp(to_rot, eased_t));
    }
    result
}

/// Blend every joint translation present in `from` or `to` toward `to` by `eased_t`.
/// A joint present in only one of the two poses falls back to the skeleton's rest
/// translation on the other side, mirroring [`blend_poses`]. This is what lets a
/// pose move a joint off its rest position — e.g. pupils shifting for gaze, or blush
/// marks popping out of the head onto the cheeks.
pub fn blend_translations(
    skeleton: &Skeleton,
    from: &ResolvedPose,
    to: &ResolvedPose,
    eased_t: f32,
) -> HashMap<JointId, Vec3> {
    let joint_ids: HashSet<JointId> = from
        .joint_translations
        .keys()
        .chain(to.joint_translations.keys())
        .copied()
        .collect();

    let mut result = HashMap::with_capacity(joint_ids.len());
    for id in joint_ids {
        let rest = skeleton.joint(id).local_translation;
        let from_t = from.joint_translations.get(&id).copied().unwrap_or(rest);
        let to_t = to.joint_translations.get(&id).copied().unwrap_or(rest);
        result.insert(id, from_t.lerp(to_t, eased_t));
    }
    result
}

/// Turns an angular pose delta into a transition duration at a fixed rate, so a small
/// pose change (e.g. a wrist flick) transitions quickly and a large one (e.g. both
/// arms swinging from rest to a T-pose) doesn't snap despite using the same call site.
/// Every joint present in either pose is compared (missing joints fall back to the
/// skeleton's rest rotation, same as [`blend_poses`]); the single largest per-joint
/// angle drives the whole-body duration, since that's the motion a viewer's eye would
/// judge as too fast or too slow. Clamped to `[min_ms, max_ms]` so a near-zero delta
/// doesn't round to an imperceptible flash and a huge one doesn't stall the rig.
pub fn duration_ms_for_speed(
    skeleton: &Skeleton,
    from: &ResolvedPose,
    to: &ResolvedPose,
    deg_per_sec: f32,
    min_ms: u32,
    max_ms: u32,
) -> u32 {
    let joint_ids: HashSet<JointId> = from
        .joint_rotations
        .keys()
        .chain(to.joint_rotations.keys())
        .copied()
        .collect();

    let mut max_deg: f32 = 0.0;
    for id in joint_ids {
        let rest = skeleton.joint(id).local_rotation;
        let from_rot = from.joint_rotations.get(&id).copied().unwrap_or(rest);
        let to_rot = to.joint_rotations.get(&id).copied().unwrap_or(rest);
        max_deg = max_deg.max(from_rot.angle_between(to_rot).to_degrees());
    }

    if deg_per_sec <= 0.0 || max_deg <= 0.0 {
        // Camera-only transitions still need a non-zero duration — treat a large
        // orbit move as roughly comparable to a limb swing so it doesn't flash.
        let cam_deg = camera_delta_deg(from.camera, to.camera);
        if cam_deg > 0.0 && deg_per_sec > 0.0 {
            let ms = (cam_deg / deg_per_sec) * 1000.0;
            return (ms.round() as u32).clamp(min_ms, max_ms);
        }
        return min_ms;
    }
    let cam_deg = camera_delta_deg(from.camera, to.camera);
    max_deg = max_deg.max(cam_deg);
    let ms = (max_deg / deg_per_sec) * 1000.0;
    (ms.round() as u32).clamp(min_ms, max_ms)
}

fn camera_delta_deg(from: crate::camera::ResolvedCamera, to: crate::camera::ResolvedCamera) -> f32 {
    let mut yaw = (to.yaw_deg - from.yaw_deg) % 360.0;
    if yaw > 180.0 {
        yaw -= 360.0;
    } else if yaw < -180.0 {
        yaw += 360.0;
    }
    let pitch = (to.pitch_deg - from.pitch_deg).abs();
    // ~20 deg of "feel" per meter of zoom so distance changes aren't free.
    let zoom = (to.distance - from.distance).abs() * 20.0;
    yaw.abs().max(pitch).max(zoom)
}

fn first_pose<'a>(target: &'a PlaybackTarget) -> &'a Pose {
    match target {
        PlaybackTarget::Pose(pose) => pose,
        PlaybackTarget::Animation(anim) => anim
            .keyframes
            .first()
            .map(|kf| &kf.pose)
            .unwrap_or_else(|| panic!("animation '{}' has no keyframes", anim.name)),
    }
}

fn resolve_or_empty(pose: &Pose, skeleton: &Skeleton) -> ResolvedPose {
    pose.resolve(skeleton)
        .unwrap_or_else(|_| ResolvedPose::empty())
}

/// Resolve a pose as a blend target relative to the live `from` snapshot: applies
/// sparse camera patches, and honors `hold_joints` so camera-only keyframes don't
/// reset the body to T-pose. A `hold_joints` pose that *also* lists joints is a
/// sparse overlay: its listed joints become the new targets while every unlisted
/// joint keeps its current value (instead of resetting to rest) — this is how a
/// blink or gaze-shift keyframe can ride on top of an ongoing body pose.
///
/// Expressions follow the same hold rule: with `hold_joints`, unlisted presets keep
/// their previous weights; without hold, missing presets in the target blend toward 0.
fn resolve_target(pose: &Pose, skeleton: &Skeleton, from: &ResolvedPose) -> ResolvedPose {
    let camera = match &pose.camera {
        Some(patch) => from.camera.with_patch(patch),
        None => from.camera,
    };
    if pose.hold_joints {
        let mut resolved = resolve_or_empty(pose, skeleton);
        for (id, rot) in &from.joint_rotations {
            resolved.joint_rotations.entry(*id).or_insert(*rot);
        }
        for (id, t) in &from.joint_translations {
            resolved.joint_translations.entry(*id).or_insert(*t);
        }
        // Sparse expression overlay: start from previous weights, then apply listed.
        let mut expressions = from.expressions.clone();
        for (k, v) in &resolved.expressions {
            expressions.insert(k.clone(), *v);
        }
        // If the pose authored no expressions, keep `from` entirely.
        if pose.expressions.is_empty() {
            expressions = from.expressions.clone();
        }
        resolved.expressions = expressions;
        resolved.camera = camera;
        return resolved;
    }
    let mut resolved = resolve_or_empty(pose, skeleton);
    resolved.camera = camera;
    resolved
}

/// Lerp expression weights. A key present in only one side falls back to 0 on the
/// other, so a non-hold pose that omits a preset fades that morph out.
pub fn blend_expressions(
    from: &HashMap<String, f32>,
    to: &HashMap<String, f32>,
    eased_t: f32,
) -> HashMap<String, f32> {
    let keys: HashSet<&String> = from.keys().chain(to.keys()).collect();
    let mut result = HashMap::with_capacity(keys.len());
    for key in keys {
        let a = from.get(key).copied().unwrap_or(0.0);
        let b = to.get(key).copied().unwrap_or(0.0);
        let v = a + (b - a) * eased_t;
        if v.abs() > 1e-5 || to.contains_key(key) {
            result.insert(key.clone(), v.clamp(0.0, 1.0));
        }
    }
    result
}

fn vrm_local_for_target(target: &PlaybackTarget) -> bool {
    match target {
        PlaybackTarget::Animation(a) => a.vrm_local_rotations,
        PlaybackTarget::Pose(_) => false,
    }
}

fn blend_frame(
    skeleton: &Skeleton,
    from: &ResolvedPose,
    to: &ResolvedPose,
    eased_t: f32,
    vrm_local_rotations: bool,
) -> ResolvedPose {
    ResolvedPose {
        joint_rotations: blend_poses(skeleton, from, to, eased_t, vrm_local_rotations),
        joint_translations: blend_translations(skeleton, from, to, eased_t),
        camera: blend_cameras(from.camera, to.camera, eased_t),
        expressions: blend_expressions(&from.expressions, &to.expressions, eased_t),
    }
}

/// What a transition or an in-progress playback is heading toward.
#[derive(Debug, Clone)]
pub enum PlaybackTarget {
    Pose(Pose),
    Animation(Animation),
}

/// Coarse playback mode for introspection (`GET /state`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlaybackMode {
    Idle,
    Transitioning,
    Playing {
        animation: String,
        keyframe_index: usize,
    },
}

/// Playback state machine for one rig. Lives entirely in this crate (no Bevy
/// dependency) so the "smart interpolation" behavior — smooth blending, and no
/// snapping when a new command interrupts an in-progress animation — is directly
/// unit-testable.
#[derive(Debug, Clone)]
pub enum PlaybackState {
    /// Holding still at `held`. `held` must retain every joint that was in the last
    /// applied blend — not an empty map — so a later sparse target (e.g. `t_pose`
    /// with no joints listed) can still blend previously-moved joints back to rest
    /// via [`blend_poses`]. Returning empty here was the bug that made
    /// `POST /pose` → `t_pose` a no-op after any non-rest pose.
    Idle {
        held: ResolvedPose,
    },
    /// A synthesized transition segment from a snapshot of the rig's current live
    /// pose into a new target, used both for "move to this pose" commands and to
    /// smoothly interrupt whatever was previously playing.
    TransitioningTo {
        from: ResolvedPose,
        target: PlaybackTarget,
        elapsed_ms: u32,
        duration_ms: u32,
        easing: Easing,
    },
    /// Steady-state animation playback, past the initial transition-in.
    Playing {
        animation: Animation,
        keyframe_index: usize,
        elapsed_ms: u32,
        segment_from: ResolvedPose,
    },
}

impl Default for PlaybackState {
    fn default() -> Self {
        PlaybackState::Idle {
            held: ResolvedPose::empty(),
        }
    }
}

impl PlaybackState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn mode(&self) -> PlaybackMode {
        match self {
            PlaybackState::Idle { .. } => PlaybackMode::Idle,
            PlaybackState::TransitioningTo { .. } => PlaybackMode::Transitioning,
            PlaybackState::Playing {
                animation,
                keyframe_index,
                ..
            } => PlaybackMode::Playing {
                animation: animation.name.clone(),
                keyframe_index: *keyframe_index,
            },
        }
    }

    /// Snapshot of the pose this state is currently rendering, used as the `from`
    /// side when a new command interrupts playback mid-flight.
    pub fn current_snapshot(&self, skeleton: &Skeleton) -> ResolvedPose {
        match self {
            PlaybackState::Idle { held } => held.clone(),
            PlaybackState::TransitioningTo {
                from,
                target,
                elapsed_ms,
                duration_ms,
                easing,
            } => {
                let t = if *duration_ms == 0 {
                    1.0
                } else {
                    *elapsed_ms as f32 / *duration_ms as f32
                };
                let vrm_local = vrm_local_for_target(target);
                let to = resolve_target(first_pose(target), skeleton, from);
                blend_frame(skeleton, from, &to, easing.apply(t), vrm_local)
            }
            PlaybackState::Playing {
                animation,
                keyframe_index,
                elapsed_ms,
                segment_from,
            } => {
                let Some(kf) = animation.keyframes.get(*keyframe_index) else {
                    return segment_from.clone();
                };
                let t = if kf.duration_ms == 0 {
                    1.0
                } else {
                    (*elapsed_ms as f32 / kf.duration_ms as f32).clamp(0.0, 1.0)
                };
                let to = resolve_target(&kf.pose, skeleton, segment_from);
                blend_frame(
                    skeleton,
                    segment_from,
                    &to,
                    kf.easing.apply(t),
                    animation.vrm_local_rotations,
                )
            }
        }
    }

    pub fn uses_vrm_local_rotations(&self) -> bool {
        match self {
            PlaybackState::Idle { .. } => false,
            PlaybackState::TransitioningTo { target, .. } => vrm_local_for_target(target),
            PlaybackState::Playing { animation, .. } => animation.vrm_local_rotations,
        }
    }

    /// Begin a smooth transition into `target` over `transition_ms`, starting from
    /// whatever the rig currently looks like (captured via `current_snapshot`) rather
    /// than snapping — this is what keeps interrupts smooth.
    pub fn interrupt(&mut self, skeleton: &Skeleton, target: PlaybackTarget, transition_ms: u32) {
        let from = self.current_snapshot(skeleton);
        *self = PlaybackState::TransitioningTo {
            from,
            target,
            elapsed_ms: 0,
            duration_ms: transition_ms,
            easing: Easing::EaseInOut,
        };
    }

    /// Advance playback by `dt_ms`, returning this frame's full resolved pose
    /// (joints + camera), or `None` if idle — callers should leave the rig/camera
    /// at their last values.
    pub fn advance(&mut self, skeleton: &Skeleton, dt_ms: u32) -> Option<ResolvedPose> {
        let state = std::mem::take(self);
        let (next_state, output) = match state {
            PlaybackState::Idle { held } => (PlaybackState::Idle { held }, None),

            PlaybackState::TransitioningTo {
                from,
                target,
                elapsed_ms,
                duration_ms,
                easing,
            } => {
                let elapsed_ms = (elapsed_ms + dt_ms).min(duration_ms);
                let t = if duration_ms == 0 {
                    1.0
                } else {
                    elapsed_ms as f32 / duration_ms as f32
                };
                let vrm_local = vrm_local_for_target(&target);
                let to_resolved = resolve_target(first_pose(&target), skeleton, &from);
                let blended = blend_frame(
                    skeleton,
                    &from,
                    &to_resolved,
                    easing.apply(t),
                    vrm_local,
                );

                if elapsed_ms >= duration_ms {
                    let next = match target {
                        PlaybackTarget::Pose(_) => PlaybackState::Idle {
                            held: blended.clone(),
                        },
                        PlaybackTarget::Animation(anim) => {
                            if anim.keyframes.len() > 1 {
                                PlaybackState::Playing {
                                    animation: anim,
                                    keyframe_index: 1,
                                    elapsed_ms: 0,
                                    segment_from: to_resolved,
                                }
                            } else {
                                PlaybackState::Idle {
                                    held: blended.clone(),
                                }
                            }
                        }
                    };
                    (next, Some(blended))
                } else {
                    (
                        PlaybackState::TransitioningTo {
                            from,
                            target,
                            elapsed_ms,
                            duration_ms,
                            easing,
                        },
                        Some(blended),
                    )
                }
            }

            PlaybackState::Playing {
                animation,
                keyframe_index,
                elapsed_ms,
                segment_from,
            } => {
                if keyframe_index >= animation.keyframes.len() {
                    (PlaybackState::Idle { held: segment_from }, None)
                } else {
                    let kf = &animation.keyframes[keyframe_index];
                    let elapsed_ms = (elapsed_ms + dt_ms).min(kf.duration_ms);
                    let t = if kf.duration_ms == 0 {
                        1.0
                    } else {
                        elapsed_ms as f32 / kf.duration_ms as f32
                    };
                    let to_resolved = resolve_target(&kf.pose, skeleton, &segment_from);
                    let blended = blend_frame(
                        skeleton,
                        &segment_from,
                        &to_resolved,
                        kf.easing.apply(t),
                        animation.vrm_local_rotations,
                    );

                    if elapsed_ms >= kf.duration_ms {
                        let next_index = keyframe_index + 1;
                        let next = if next_index >= animation.keyframes.len() {
                            if animation.looping {
                                PlaybackState::Playing {
                                    animation,
                                    keyframe_index: 0,
                                    elapsed_ms: 0,
                                    segment_from: to_resolved,
                                }
                            } else {
                                PlaybackState::Idle {
                                    held: blended.clone(),
                                }
                            }
                        } else {
                            PlaybackState::Playing {
                                animation,
                                keyframe_index: next_index,
                                elapsed_ms: 0,
                                segment_from: to_resolved,
                            }
                        };
                        (next, Some(blended))
                    } else {
                        (
                            PlaybackState::Playing {
                                animation,
                                keyframe_index,
                                elapsed_ms,
                                segment_from,
                            },
                            Some(blended),
                        )
                    }
                }
            }
        };
        *self = next_state;
        output
    }

    pub fn is_idle(&self) -> bool {
        matches!(self, PlaybackState::Idle { .. })
    }

    /// Total duration of the playable segment after the first keyframe (matches
    /// [`PlaybackState::advance`] once the rig has reached keyframe index 1).
    pub fn animation_playable_duration_ms(animation: &Animation) -> u32 {
        animation
            .keyframes
            .iter()
            .skip(1)
            .map(|kf| kf.duration_ms)
            .sum()
    }

    /// Sample an animation at `time_ms` along the same timeline as steady-state
    /// [`Playing`](PlaybackState::Playing): the rig is held at the first keyframe's
    /// resolved pose at `t = 0`, then each subsequent keyframe segment runs for its
    /// authored `duration_ms`. When `animation.looping` is true, `time_ms` wraps.
    pub fn pose_at_animation_time(
        skeleton: &Skeleton,
        animation: &Animation,
        time_ms: u32,
    ) -> ResolvedPose {
        if animation.keyframes.is_empty() {
            return ResolvedPose::empty();
        }
        let from_empty = ResolvedPose::empty();
        let mut segment_from =
            resolve_target(&animation.keyframes[0].pose, skeleton, &from_empty);
        if animation.keyframes.len() == 1 {
            return segment_from;
        }

        let total = Self::animation_playable_duration_ms(animation);
        let t = if animation.looping && total > 0 {
            time_ms % total
        } else {
            time_ms.min(total)
        };
        if t == 0 {
            return segment_from;
        }

        let mut remaining = t;
        for kf in animation.keyframes.iter().skip(1) {
            if remaining <= kf.duration_ms {
                let progress = if kf.duration_ms == 0 {
                    1.0
                } else {
                    remaining as f32 / kf.duration_ms as f32
                };
                let to = resolve_target(&kf.pose, skeleton, &segment_from);
                return blend_frame(
                    skeleton,
                    &segment_from,
                    &to,
                    kf.easing.apply(progress),
                    animation.vrm_local_rotations,
                );
            }
            remaining -= kf.duration_ms;
            segment_from = resolve_target(&kf.pose, skeleton, &segment_from);
        }
        segment_from
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::animation::Keyframe;
    use crate::pose::{EulerDeg, JointTarget};

    fn pose_with_rotation(name: &str, joint: &str, z_deg: f32) -> Pose {
        let mut joints = HashMap::new();
        joints.insert(
            joint.to_string(),
            JointTarget {
                rotation_deg: Some(EulerDeg {
                    x: 0.0,
                    y: 0.0,
                    z: z_deg,
                }),
                rotation_quat: None,
                translation: None,
            },
        );
        Pose {
            name: name.to_string(),
            description: None,
            joints,
            camera: None,
            expressions: HashMap::new(),
            hold_joints: false,
        }
    }

    #[test]
    fn blend_poses_at_t0_matches_from_and_at_t1_matches_to() {
        let skeleton = Skeleton::humanoid_default();
        let joint = skeleton.joint_by_name("right_shoulder").unwrap();
        let from = pose_with_rotation("a", "right_shoulder", 0.0)
            .resolve(&skeleton)
            .unwrap();
        let to = pose_with_rotation("b", "right_shoulder", -90.0)
            .resolve(&skeleton)
            .unwrap();

        let at_start = blend_poses(&skeleton, &from, &to, 0.0, false);
        let at_end = blend_poses(&skeleton, &from, &to, 1.0, false);

        // angle_between loses precision near-parallel quaternions (acos'(1) blows up
        // near t=0/1), so use a tolerance well above float noise rather than 1e-4.
        assert!(at_start[&joint].angle_between(from.joint_rotations[&joint]) < 1e-2);
        assert!(at_end[&joint].angle_between(to.joint_rotations[&joint]) < 1e-2);
    }

    #[test]
    fn transition_to_pose_reaches_target_and_goes_idle() {
        let skeleton = Skeleton::humanoid_default();
        let joint = skeleton.joint_by_name("right_shoulder").unwrap();
        let target_pose = pose_with_rotation("wave", "right_shoulder", -80.0);

        let mut state = PlaybackState::new();
        state.interrupt(&skeleton, PlaybackTarget::Pose(target_pose.clone()), 500);

        // Halfway through, we should be between rest (identity) and the target.
        let mid = state.advance(&skeleton, 250).unwrap();
        let target_resolved = target_pose.resolve(&skeleton).unwrap();
        assert!(mid.joint_rotations[&joint].angle_between(Quat::IDENTITY) > 0.01);
        assert!(mid.joint_rotations[&joint].angle_between(target_resolved.joint_rotations[&joint]) > 0.01);

        // After the full duration, we should have arrived and gone idle.
        let end = state.advance(&skeleton, 250).unwrap();
        assert!(end.joint_rotations[&joint].angle_between(target_resolved.joint_rotations[&joint]) < 1e-2);
        assert!(state.is_idle());
    }

    #[test]
    fn interrupting_mid_transition_starts_from_current_pose_not_rest() {
        let skeleton = Skeleton::humanoid_default();
        let joint = skeleton.joint_by_name("right_shoulder").unwrap();
        let pose_a = pose_with_rotation("a", "right_shoulder", -90.0);
        let pose_b = pose_with_rotation("b", "right_shoulder", 90.0);

        let mut state = PlaybackState::new();
        state.interrupt(&skeleton, PlaybackTarget::Pose(pose_a), 1000);
        let mid_a = state.advance(&skeleton, 500).unwrap().joint_rotations[&joint];

        // Interrupt halfway through the first transition.
        state.interrupt(&skeleton, PlaybackTarget::Pose(pose_b), 1000);
        let just_after_interrupt = state.advance(&skeleton, 1).unwrap().joint_rotations[&joint];

        // The very first frame after interrupting should be extremely close to
        // wherever we were (mid_a), not a snap back toward rest/identity.
        assert!(
            just_after_interrupt.angle_between(mid_a) < 0.05,
            "expected smooth continuation, got a jump: mid_a={mid_a:?} just_after={just_after_interrupt:?}"
        );
    }

    #[test]
    fn animation_playback_advances_through_keyframes_and_stops() {
        let skeleton = Skeleton::humanoid_default();
        let joint = skeleton.joint_by_name("right_shoulder").unwrap();
        let anim = Animation {
            name: "test_anim".into(),
            description: None,
            looping: false,
            vrm_local_rotations: false,
            play_automatically: false,
            keyframes: vec![
                Keyframe {
                    pose: pose_with_rotation("start", "right_shoulder", 0.0),
                    duration_ms: 100,
                    easing: Easing::Linear,
                },
                Keyframe {
                    pose: pose_with_rotation("mid", "right_shoulder", -45.0),
                    duration_ms: 200,
                    easing: Easing::Linear,
                },
                Keyframe {
                    pose: pose_with_rotation("end", "right_shoulder", 0.0),
                    duration_ms: 200,
                    easing: Easing::Linear,
                },
            ],
        };

        let mut state = PlaybackState::new();
        state.interrupt(&skeleton, PlaybackTarget::Animation(anim), 100);

        // Drive well past the total duration (100 + 200 + 200 = 500ms of segments
        // after the initial 100ms transition-in).
        let mut last = None;
        for _ in 0..20 {
            if let Some(pose) = state.advance(&skeleton, 50) {
                last = Some(pose);
            }
        }
        assert!(state.is_idle(), "animation should finish and go idle");
        let last = last.unwrap();
        // Final keyframe returns to 0 degrees, i.e. identity.
        assert!(last.joint_rotations[&joint].angle_between(Quat::IDENTITY) < 1e-2);
    }

    #[test]
    fn duration_for_speed_scales_with_angular_delta() {
        let skeleton = Skeleton::humanoid_default();
        let from = pose_with_rotation("a", "right_shoulder", 0.0)
            .resolve(&skeleton)
            .unwrap();
        let small = pose_with_rotation("small", "right_shoulder", 10.0)
            .resolve(&skeleton)
            .unwrap();
        let large = pose_with_rotation("large", "right_shoulder", 90.0)
            .resolve(&skeleton)
            .unwrap();

        let small_ms = duration_ms_for_speed(&skeleton, &from, &small, 180.0, 50, 5000);
        let large_ms = duration_ms_for_speed(&skeleton, &from, &large, 180.0, 50, 5000);

        // 90 degrees at 180 deg/s should take ~500ms.
        assert!(
            (large_ms as i32 - 500).abs() <= 5,
            "large_ms = {large_ms}"
        );
        assert!(
            small_ms < large_ms,
            "a smaller pose delta should take less time: small={small_ms} large={large_ms}"
        );
    }

    #[test]
    fn duration_for_speed_clamps_to_min_and_max() {
        let skeleton = Skeleton::humanoid_default();
        let from = pose_with_rotation("a", "right_shoulder", 0.0)
            .resolve(&skeleton)
            .unwrap();
        let to = pose_with_rotation("b", "right_shoulder", 179.0)
            .resolve(&skeleton)
            .unwrap();

        // A tiny delta at a fast rate should clamp up to min_ms rather than round to 0.
        let identical = duration_ms_for_speed(&skeleton, &from, &from, 180.0, 150, 2000);
        assert_eq!(identical, 150);

        // A huge delta at a slow rate should clamp down to max_ms rather than stall.
        let huge = duration_ms_for_speed(&skeleton, &from, &to, 1.0, 150, 2000);
        assert_eq!(huge, 2000);
    }

    #[test]
    fn sparse_rest_pose_resets_joints_after_idle() {
        // Regression: after arriving at a sparse pose and going Idle, a later
        // transition into an empty pose (t_pose) must still blend previously-moved
        // joints back to rest. If Idle forgets the held pose, this is a no-op and
        // the doll stays visually stuck (idle screenshot == t_pose screenshot).
        let skeleton = Skeleton::humanoid_default();
        let shoulder = skeleton.joint_by_name("right_shoulder").unwrap();
        let elbow = skeleton.joint_by_name("right_elbow").unwrap();
        let wave = Pose {
            name: "wave".into(),
            description: None,
            camera: None,
            expressions: HashMap::new(),
            hold_joints: false,
            joints: {
                let mut joints = HashMap::new();
                joints.insert(
                    "right_shoulder".into(),
                    JointTarget {
                        rotation_deg: Some(EulerDeg {
                            x: 0.0,
                            y: 0.0,
                            z: 80.0,
                        }),
                        rotation_quat: None,
                        translation: None,
                    },
                );
                joints.insert(
                    "right_elbow".into(),
                    JointTarget {
                        rotation_deg: Some(EulerDeg {
                            x: 0.0,
                            y: 0.0,
                            z: 30.0,
                        }),
                        rotation_quat: None,
                        translation: None,
                    },
                );
                joints
            },
        };
        let t_pose = Pose {
            name: "t_pose".into(),
            description: None,
            joints: HashMap::new(),
            camera: None,
            expressions: HashMap::new(),
            hold_joints: false,
        };

        let mut state = PlaybackState::new();
        state.interrupt(&skeleton, PlaybackTarget::Pose(wave), 100);
        let after_wave = state.advance(&skeleton, 100).unwrap();
        assert!(state.is_idle());
        assert!(after_wave.joint_rotations[&shoulder].angle_between(Quat::IDENTITY) > 0.1);
        assert!(after_wave.joint_rotations[&elbow].angle_between(Quat::IDENTITY) > 0.1);

        state.interrupt(&skeleton, PlaybackTarget::Pose(t_pose), 100);
        let after_t = state.advance(&skeleton, 100).unwrap();
        assert!(state.is_idle());
        assert!(
            after_t.joint_rotations[&shoulder].angle_between(Quat::IDENTITY) < 1e-2,
            "shoulder should return to rest on t_pose"
        );
        assert!(
            after_t.joint_rotations[&elbow].angle_between(Quat::IDENTITY) < 1e-2,
            "elbow should return to rest on t_pose"
        );
    }

    #[test]
    fn interrupting_mid_keyframe_starts_from_live_blend_not_segment_start() {
        let skeleton = Skeleton::humanoid_default();
        let joint = skeleton.joint_by_name("right_shoulder").unwrap();
        let anim = Animation {
            name: "test_anim".into(),
            description: None,
            looping: false,
            vrm_local_rotations: false,
            play_automatically: false,
            keyframes: vec![
                Keyframe {
                    pose: pose_with_rotation("start", "right_shoulder", 0.0),
                    duration_ms: 10,
                    easing: Easing::Linear,
                },
                Keyframe {
                    pose: pose_with_rotation("end", "right_shoulder", -90.0),
                    duration_ms: 1000,
                    easing: Easing::Linear,
                },
            ],
        };

        let mut state = PlaybackState::new();
        state.interrupt(&skeleton, PlaybackTarget::Animation(anim), 0);
        // Finish the entry transition (0ms) and get into the second keyframe, then
        // advance halfway through its 1000ms blend.
        state.advance(&skeleton, 1);
        let mid = state.advance(&skeleton, 500).unwrap().joint_rotations[&joint];

        let interrupt_pose = pose_with_rotation("other", "right_shoulder", 90.0);
        state.interrupt(&skeleton, PlaybackTarget::Pose(interrupt_pose), 1000);
        let just_after = state.advance(&skeleton, 1).unwrap().joint_rotations[&joint];
        assert!(
            just_after.angle_between(mid) < 0.05,
            "mid-keyframe interrupt must continue from the live blend, not snap to segment_from"
        );
    }

    #[test]
    fn looping_animation_never_goes_idle() {
        let skeleton = Skeleton::humanoid_default();
        let anim = Animation {
            name: "loop_anim".into(),
            description: None,
            looping: true,
            vrm_local_rotations: false,
            play_automatically: false,
            keyframes: vec![
                Keyframe {
                    pose: pose_with_rotation("a", "right_shoulder", 0.0),
                    duration_ms: 100,
                    easing: Easing::Linear,
                },
                Keyframe {
                    pose: pose_with_rotation("b", "right_shoulder", -45.0),
                    duration_ms: 100,
                    easing: Easing::Linear,
                },
            ],
        };
        let mut state = PlaybackState::new();
        state.interrupt(&skeleton, PlaybackTarget::Animation(anim), 0);
        for _ in 0..50 {
            state.advance(&skeleton, 50);
        }
        assert!(!state.is_idle(), "looping animation should keep playing");
    }

    #[test]
    fn blend_expressions_missing_in_to_fades_to_zero() {
        let mut from = HashMap::new();
        from.insert("happy".into(), 1.0);
        from.insert("blink".into(), 0.5);
        let to = HashMap::new();
        let mid = blend_expressions(&from, &to, 0.5);
        assert!((mid["happy"] - 0.5).abs() < 1e-5);
        assert!((mid["blink"] - 0.25).abs() < 1e-5);
        let end = blend_expressions(&from, &to, 1.0);
        assert!(end.is_empty() || end.values().all(|v| *v < 1e-4));
    }

    #[test]
    fn expression_hold_overlay_keeps_unlisted_weights() {
        let skeleton = Skeleton::humanoid_default();
        let mut from = ResolvedPose::empty();
        from.expressions.insert("happy".into(), 0.8);

        let mut blink_pose = Pose {
            name: "blink".into(),
            description: None,
            joints: HashMap::new(),
            camera: None,
            expressions: {
                let mut e = HashMap::new();
                e.insert("blink".into(), 1.0);
                e
            },
            hold_joints: true,
        };
        let _ = &mut blink_pose; // silence
        let to = resolve_target(&blink_pose, &skeleton, &from);
        assert!((to.expressions["happy"] - 0.8).abs() < 1e-5);
        assert!((to.expressions["blink"] - 1.0).abs() < 1e-5);
    }

    #[test]
    fn non_hold_pose_clears_previous_expressions_at_end() {
        let skeleton = Skeleton::humanoid_default();
        let mut happy = Pose {
            name: "happy".into(),
            description: None,
            joints: HashMap::new(),
            camera: None,
            expressions: {
                let mut e = HashMap::new();
                e.insert("happy".into(), 1.0);
                e
            },
            hold_joints: false,
        };
        let clear = Pose {
            name: "clear".into(),
            description: None,
            joints: HashMap::new(),
            camera: None,
            expressions: HashMap::new(),
            hold_joints: false,
        };

        let mut state = PlaybackState::new();
        state.interrupt(&skeleton, PlaybackTarget::Pose(happy.clone()), 50);
        let after_happy = state.advance(&skeleton, 50).unwrap();
        assert!((after_happy.expressions["happy"] - 1.0).abs() < 1e-5);

        state.interrupt(&skeleton, PlaybackTarget::Pose(clear), 50);
        let after_clear = state.advance(&skeleton, 50).unwrap();
        assert!(
            after_clear
                .expressions
                .get("happy")
                .copied()
                .unwrap_or(0.0)
                < 1e-4,
            "non-hold empty expressions should fade happy out"
        );
        let _ = &mut happy;
    }

    #[test]
    fn pose_at_animation_time_matches_playback_mid_segment() {
        let skeleton = Skeleton::humanoid_default();
        let joint = skeleton.joint_by_name("right_shoulder").unwrap();
        let anim = Animation {
            name: "scrub".into(),
            description: None,
            looping: false,
            vrm_local_rotations: false,
            play_automatically: false,
            keyframes: vec![
                Keyframe {
                    pose: pose_with_rotation("start", "right_shoulder", 0.0),
                    duration_ms: 0,
                    easing: Easing::Linear,
                },
                Keyframe {
                    pose: pose_with_rotation("end", "right_shoulder", -90.0),
                    duration_ms: 1000,
                    easing: Easing::Linear,
                },
            ],
        };

        let at_half = PlaybackState::pose_at_animation_time(&skeleton, &anim, 500);
        let mut state = PlaybackState::new();
        state.interrupt(&skeleton, PlaybackTarget::Animation(anim.clone()), 0);
        state.advance(&skeleton, 1);
        let playing_mid = state.advance(&skeleton, 500).unwrap();

        assert!(
            at_half.joint_rotations[&joint]
                .angle_between(playing_mid.joint_rotations[&joint])
                < 0.05,
            "scrub sample should match live playback at the same time"
        );
    }

    #[test]
    fn transition_to_idle_pose_with_camera_restores_default_stage() {
        use crate::camera::{CameraTarget, DEFAULT_CAMERA, ResolvedCamera};

        let skeleton = Skeleton::humanoid_default();
        let mut from = ResolvedPose::empty();
        from.camera = ResolvedCamera {
            yaw_deg: 280.0,
            pitch_deg: 15.0,
            distance: 2.5,
            look_at: [0.2, 1.2, 0.1],
        };

        let idle = Pose {
            name: "idle".into(),
            description: None,
            joints: HashMap::new(),
            camera: Some(CameraTarget::full_default_stage()),
            expressions: HashMap::new(),
            hold_joints: false,
        };

        let mut state = PlaybackState::new();
        state.interrupt(&skeleton, PlaybackTarget::Pose(idle), 500);
        let mid = state.advance(&skeleton, 250).unwrap();
        assert!(
            (mid.camera.yaw_deg - from.camera.yaw_deg).abs() > 5.0,
            "camera should move toward default mid-transition"
        );
        assert!(
            (mid.camera.yaw_deg - DEFAULT_CAMERA.yaw_deg).abs()
                < (from.camera.yaw_deg - DEFAULT_CAMERA.yaw_deg).abs(),
            "mid yaw should be closer to default than start"
        );
        let end = state.advance(&skeleton, 250).unwrap();
        assert!((end.camera.yaw_deg - DEFAULT_CAMERA.yaw_deg).abs() < 0.01);
        assert!((end.camera.distance - DEFAULT_CAMERA.distance).abs() < 0.01);
    }
}
