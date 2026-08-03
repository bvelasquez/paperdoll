//! Curated agent-facing metadata for `GET /capabilities`: pose/animation boards,
//! catalogs, timing/camera recipes, and the system prompt agents should follow.

use paperdoll_rig::{Animation, Pose};
use serde_json::{json, Value};
use std::collections::HashMap;

/// Curated pose-board names surfaced in `example_poses`. Each entry must exist in
/// the live library; missing names are skipped.
pub const EXAMPLE_POSE_BOARD: &[(&str, &str)] = &[
    (
        "wave",
        "cartoon wave: mild right_shoulder.z (~-22, NOT -55) + elbow z (~-72) for an \
L-shape, plus positive shoulder y (~50) so the palm faces the camera. Pair with \
wave_return for wave_animation. Pitfall: large negative shoulder z with a deep elbow \
fold parks the hand on the head (see think).",
    ),
    (
        "hands_on_hips",
        "mirrored-sign convention: equal magnitude, opposite sign — \
right_shoulder.z=+45 with left_shoulder.z=-45 puts a hand on each hip.",
    ),
    (
        "idle",
        "asymmetric full-body pose (arm + knee + pelvis) — also the IdleRevert default \
after inactivity. Clears VRM expression weights so leftover morphs fade out.",
    ),
    (
        "victory",
        "both arms raised: right_shoulder.z negative, left_shoulder.z positive \
(mirrored raise from T-pose).",
    ),
    (
        "think",
        "hand near temple: positive shoulder y (forward) + raise z + deep elbow bend. \
Contrast with wave — same axes, much deeper elbow / more raise = head contact.",
    ),
    (
        "shrug",
        "clavicle + mirrored shoulder/elbow rotations for a shoulders-up shrug.",
    ),
    (
        "point",
        "arm forward toward camera: right_shoulder.y is POSITIVE (~+75; negative y \
points behind the body). Pair with point_hero for a push-in. Index extended; \
other digits curled. Verify with a side-camera screenshot (yaw ~75), not only front.",
    ),
    (
        "peace_sign_right",
        "v2 finger exemplar: right index + middle extended, ring/little + thumb curled. \
Author finger joints (*_proximal/*_intermediate/*_distal) the same way as body joints.",
    ),
    (
        "head_turn_left",
        "head turn without arm gesture: neck.y ≈ +42° (left). Arms stay in mild T-pose \
offsets. Close-up face framing in YAML camera (distance ~1.85, look_at y ~1.35). \
Pair with head_turn_right for say_no.",
    ),
    (
        "head_nod_down",
        "nod down: neck +x with soft knee dip (see joints). Rebound with head_nod_up. \
Keyframe pair for say_yes.",
    ),
    (
        "squat",
        "deep squat leg recipe: hip x ~-52, knee x ~+102, ankle x ~+32, mirrored hip z. \
Root does not translate — frame with lower look_at y (~0.55) so the pose reads on screen.",
    ),
    (
        "cross_arms",
        "folded arms: strong forward shoulder y (±80) + mild z (±25) + elbow z (±85). \
Copy camera block for upper-body close-ups (distance ~2.1).",
    ),
    (
        "contrapposto",
        "weight-shift stance: pelvis z cock + opposite knee soft bend — copy for standing \
personality poses before gestures.",
    ),
];

/// Curated animation exemplars (metadata + live keyframe outline). VRMA clips are
/// included with pattern `vrma_import` — agents must not hand-copy their keyframes.
pub const EXAMPLE_ANIMATION_BOARD: &[(&str, &str, &str)] = &[
    (
        "wave_animation",
        "pose_ref_choreography",
        "Simplest loop: only pose names (idle → wave ↔ wave_return). No inline joints. \
Segment timing 200–550 ms; ease_out on entries, ease_in_out on oscillation.",
    ),
    (
        "say_yes",
        "pose_ref_choreography",
        "Head-only yes: idle → head_nod_down/up twice → idle. Face close-up camera on \
bookends (yaw ~12°, distance ~2.0, look_at y ~1.28). Nod segments 220–260 ms.",
    ),
    (
        "say_no",
        "pose_ref_choreography",
        "Head shake: alternate head_turn_left / head_turn_right with short ease_in_out \
segments (~180–220 ms). No invented neck angles — reuse the poses.",
    ),
    (
        "happy_bounce",
        "pose_ref_plus_expressions",
        "Cheer template: pose refs (hands_on_hips → victory → fist_pump) + expression \
overlays; blink uses hold:true + step (~90 ms). Camera bounce on each beat.",
    ),
    (
        "finger_emote",
        "inline_joints_plus_expressions",
        "Hand-led emote: inline finger/shoulder joints on keyframes + VRM morphs. Hand \
close-up camera (yaw ~15°, distance ~2.2, look_at [0.15, 1.35, 0]). Copy for new \
finger cheers — do not guess finger angles from scratch.",
    ),
    (
        "point_hero",
        "camera_choreography",
        "Establish wide → point close-up → blink overlay → pull back. Pose refs carry \
finger shape; camera drives the hero moment (distance 5.2 → ~2.4).",
    ),
    (
        "orbit_victory",
        "camera_only_holds",
        "Body holds victory while camera-only keyframes orbit (yaw steps ~90°). Use \
empty pose + camera blocks; duration 850 ms on orbit segment with linear easing.",
    ),
    (
        "vrma_clapping",
        "vrma_import",
        "Imported mocap: thousands of quaternion keyframes. Trigger with POST /animation \
{\"name\":\"vrma_clapping\"} or POST /import/vrma — never author this by hand.",
    ),
];

/// Full system prompt returned as `agent_system_prompt` in capabilities.
pub const AGENT_SYSTEM_PROMPT: &str = r#"You are authoring poses and animations for the paperdoll HTTP API (default http://127.0.0.1:7878, variant v2 VRM).

## Before writing anything
1. GET /capabilities once. Read `posing_guide`, `camera_framing`, `timing_guide`, `motion_sources`, `example_poses`, and `example_animations`.
2. GET /poses and GET /animations for the live name lists.
3. Prefer **composing existing pose names** in animations (`pose_ref_choreography`) over inventing joint angles.
4. If you must invent joints, **copy magnitudes** from the closest `example_poses` entry, then POST /poses, POST /pose, GET /screenshot. Use a side camera (yaw ≈ 70–80) to verify forward/back arm motion.

## Posing rules (summary)
- Limbs use rest-relative **rotation_deg** (Euler degrees). Axes are NOT guessable from joint names — follow `posing_guide.arm_chain` / `leg_chain`.
- RIGHT arm: negative z raises, positive y swings **forward** toward the camera.
- Mirrored poses need **opposite-signed z** on left vs right.
- Fingers (v2): positive z curl on proximal/intermediate/distal (~80–95). See `peace_sign_right` and `finger_emote`.
- Face (v2): use `expressions` on poses/keyframes (`happy`, `blink`, …). Blink overlays: `hold: true`, `easing: step`, 80–120 ms.

## Animation rules
- Each keyframe needs `duration_ms` (blend time **from previous** keyframe) and `easing`.
- Use **one of**: `pose` (name from GET /poses), inline `joints`, `camera`, and/or `expressions` — not both `pose` and `joints` on the same keyframe.
- `hold: true` = sparse overlay (blinks, partial finger tweaks) — only listed joints/expressions move.
- Pose references are **snapshotted at registration**. After updating a pose, re-POST affected animations.
- Full-body mocap: use `vrma_*` animations or POST /import/vrma — do **not** emit `rotation_quat` keyframes unless you are a tooling pipeline.

## Camera & framing
- Orbit camera: `yaw_deg`, `pitch_deg`, `distance`, `look_at` [x,y,z]. Omitted fields keep current values.
- Start from `camera_framing` presets in capabilities; copy numbers from exemplar poses (`point`, `head_turn_left`, `squat` camera blocks).

## Timing
- Follow `timing_guide` bands. Gestures: 200–500 ms per beat; micro blinks 80–120 ms; head nods 220–260 ms; settle back to idle 320–550 ms.
- `ease_out` for anticipations, `ease_in_out` for reversals, `linear` for camera orbits.

## Verify loop (required for new content)
1. POST /poses or POST /animations with your JSON body.
2. POST /pose or POST /animation to preview.
3. GET /screenshot (and GET /state for numeric joints/camera).
4. Fix and re-register until correct. Close the in-app editor (F2) if POST returns 409.

## Deliverable
Return the final JSON you registered and the pose/animation name to trigger it."#;

pub fn build_example_poses(poses: &HashMap<String, Pose>) -> Vec<Value> {
    EXAMPLE_POSE_BOARD
        .iter()
        .filter_map(|(name, demonstrates)| {
            let pose = poses.get(*name)?;
            Some(json!({
                "name": pose.name,
                "demonstrates": demonstrates,
                "description": pose.description,
                "joints": pose.joints,
                "expressions": pose.expressions,
                "camera": pose.camera,
            }))
        })
        .collect()
}

fn animation_pattern(animation: &Animation, poses: &HashMap<String, Pose>) -> &'static str {
    if animation.vrm_local_rotations {
        return "vrma_import";
    }
    let mut has_inline = false;
    let mut has_pose_ref = false;
    let mut has_camera_only = false;
    for kf in &animation.keyframes {
        if poses.contains_key(&kf.pose.name) {
            has_pose_ref = true;
        } else if kf.pose.name.contains('#') {
            if kf.pose.joints.is_empty() && kf.pose.hold_joints {
                has_camera_only = true;
            } else if !kf.pose.joints.is_empty() {
                has_inline = true;
            }
        }
    }
    if has_inline {
        "inline_joints"
    } else if has_camera_only && !has_pose_ref {
        "camera_only"
    } else if has_pose_ref && has_camera_only {
        "pose_ref_plus_camera"
    } else {
        "pose_ref_choreography"
    }
}

fn keyframe_outline(kf: &paperdoll_rig::Keyframe, poses: &HashMap<String, Pose>) -> Value {
    let mut outline = json!({
        "duration_ms": kf.duration_ms,
        "easing": serde_json::to_value(kf.easing).unwrap_or(Value::Null),
    });
    if poses.contains_key(&kf.pose.name) {
        outline["pose_ref"] = json!(kf.pose.name);
    } else if kf.pose.name.contains('#') {
        if !kf.pose.joints.is_empty() {
            outline["inline_joint_count"] = json!(kf.pose.joints.len());
        }
        if kf.pose.hold_joints {
            outline["hold"] = json!(true);
        }
    }
    if let Some(cam) = &kf.pose.camera {
        let mut c = serde_json::Map::new();
        if let Some(v) = cam.yaw_deg {
            c.insert("yaw_deg".into(), json!(v));
        }
        if let Some(v) = cam.pitch_deg {
            c.insert("pitch_deg".into(), json!(v));
        }
        if let Some(v) = cam.distance {
            c.insert("distance".into(), json!(v));
        }
        if let Some(v) = cam.look_at {
            c.insert("look_at".into(), json!(v));
        }
        if !c.is_empty() {
            outline["camera"] = Value::Object(c);
        }
    }
    if !kf.pose.expressions.is_empty() {
        outline["expressions"] = json!(kf.pose.expressions);
    }
    outline
}

pub fn build_example_animations(
    animations: &HashMap<String, Animation>,
    poses: &HashMap<String, Pose>,
) -> Vec<Value> {
    EXAMPLE_ANIMATION_BOARD
        .iter()
        .filter_map(|(name, pattern_hint, demonstrates)| {
            let animation = animations.get(*name)?;
            let pattern = if *pattern_hint != "vrma_import" {
                *pattern_hint
            } else {
                animation_pattern(animation, poses)
            };
            let total_ms: u32 = animation.keyframes.iter().map(|k| k.duration_ms).sum();
            let mut entry = json!({
                "name": animation.name,
                "pattern": pattern,
                "demonstrates": demonstrates,
                "description": animation.description,
                "keyframe_count": animation.keyframes.len(),
                "total_duration_ms": total_ms,
                "vrm_local_rotations": animation.vrm_local_rotations,
            });
            if animation.vrm_local_rotations {
                entry["keyframes_outline"] = json!([]);
                entry["note"] = json!(
                    "VRMA import — use POST /animation or POST /import/vrma; do not copy quaternions into new clips"
                );
            } else {
                let outline: Vec<Value> = animation
                    .keyframes
                    .iter()
                    .map(|kf| keyframe_outline(kf, poses))
                    .collect();
                entry["keyframes_outline"] = json!(outline);
            }
            Some(entry)
        })
        .collect()
}

pub fn build_pose_catalog(poses: &HashMap<String, Pose>) -> Vec<Value> {
    let mut names: Vec<_> = poses.keys().collect();
    names.sort();
    names
        .into_iter()
        .map(|name| {
            let pose = &poses[name];
            json!({
                "name": pose.name,
                "description": pose.description,
                "joint_count": pose.joints.len(),
                "has_camera": pose.camera.is_some(),
                "has_expressions": !pose.expressions.is_empty(),
                "on_pose_board": EXAMPLE_POSE_BOARD.iter().any(|(n, _)| *n == pose.name),
            })
        })
        .collect()
}

pub fn build_animation_catalog(
    animations: &HashMap<String, Animation>,
    poses: &HashMap<String, Pose>,
) -> Vec<Value> {
    let mut names: Vec<_> = animations.keys().collect();
    names.sort();
    names
        .into_iter()
        .map(|name| {
            let animation = &animations[name];
            let total_ms: u32 = animation.keyframes.iter().map(|k| k.duration_ms).sum();
            json!({
                "name": animation.name,
                "description": animation.description,
                "pattern": animation_pattern(animation, poses),
                "keyframe_count": animation.keyframes.len(),
                "total_duration_ms": total_ms,
                "vrm_local_rotations": animation.vrm_local_rotations,
                "play_automatically": animation.play_automatically,
                "on_animation_board": EXAMPLE_ANIMATION_BOARD.iter().any(|(n, _, _)| *n == animation.name),
                "authoring_hint": if animation.vrm_local_rotations {
                    "play_only — import via VRMA"
                } else if name == "new_animation" {
                    "placeholder — prefer wave_animation or say_yes"
                } else {
                    "compose pose refs when possible"
                },
            })
        })
        .collect()
}

pub fn timing_guide() -> Value {
    json!({
        "unit": "duration_ms is the blend time FROM the previous keyframe (or entry pose for keyframe 0)",
        "easing_when": {
            "ease_out": "anticipation / entering a pose (first beat off idle)",
            "ease_in_out": "reversals and oscillation (wave_return, head turns)",
            "ease_in": "rare — exits into a held shape",
            "linear": "camera orbit segments (orbit_victory)",
            "step": "blinks and snaps with hold:true overlays (happy_bounce blink)"
        },
        "bands_ms": {
            "micro": "80–140 (blinks, expression snaps)",
            "short": "180–280 (head turns, quick gestures)",
            "medium": "300–450 (arm raises, single nod down/up)",
            "long": "500–850 (camera moves, victory holds)"
        },
        "exemplar_totals": {
            "wave_animation": "~2280 ms over 7 keyframes",
            "say_yes": "~1400 ms over 7 keyframes",
            "say_no": "~900 ms over 5 keyframes",
            "happy_bounce": "see example_animations outline",
            "vrma_clapping": "long — use POST /animation to preview, do not re-author"
        },
        "rule": "Shorter segments feel snappier; chain medium segments for cheers. Always end with idle or a neutral pose 320–550 ms ease_in_out."
    })
}

pub fn camera_framing() -> Value {
    json!({
        "default_full_body": {
            "yaw_deg": 35.5,
            "pitch_deg": 9.2,
            "distance": 4.36,
            "look_at": [0.0, 0.9, 0.0],
            "use_for": "establishing shots, wave, victory wide"
        },
        "face_dialogue": {
            "yaw_deg": 12.0,
            "pitch_deg": 3.0,
            "distance": 2.0,
            "look_at": [0.0, 1.28, 0.0],
            "use_for": "say_yes / say_no bookends — copy from shipped head poses"
        },
        "hand_close_up": {
            "yaw_deg": 15.0,
            "pitch_deg": 5.0,
            "distance": 2.2,
            "look_at": [0.15, 1.35, 0.0],
            "use_for": "finger_emote, peace_sign_right verification"
        },
        "upper_body": {
            "yaw_deg": 18.0,
            "pitch_deg": 4.0,
            "distance": 2.1,
            "look_at": [0.0, 1.15, 0.0],
            "use_for": "cross_arms, think, shrug"
        },
        "hero_wide": {
            "yaw_deg": 40.0,
            "pitch_deg": 12.0,
            "distance": 5.2,
            "look_at": [0.0, 0.9, 0.0],
            "use_for": "point_hero opening"
        },
        "squat_framing": {
            "yaw_deg": 22.0,
            "pitch_deg": 6.0,
            "distance": 3.6,
            "look_at": [0.0, 0.55, 0.0],
            "use_for": "low center of mass — lower look_at y because hips do not translate"
        },
        "side_verify_arm": {
            "yaw_deg": 75.0,
            "pitch_deg": 8.0,
            "distance": 4.0,
            "look_at": [0.0, 0.95, 0.0],
            "use_for": "screenshot check for forward/back shoulder y (point, think)"
        },
        "note": "Sparse camera fields are allowed — omitted yaw/pitch/distance/look_at keep the live camera. Put camera on pose YAML for reusable framing."
    })
}

pub fn motion_sources() -> Value {
    json!({
        "hand_authored_pose": {
            "format": "rotation_deg Euler per joint in POST /poses",
            "when": "new static shapes, fingers, stances",
            "learn_from": "example_poses + posing_guide"
        },
        "hand_authored_animation": {
            "format": "keyframes with pose refs and/or inline joints, optional camera and expressions",
            "when": "cheers, emotes, camera moves, head nods",
            "learn_from": "example_animations — especially wave_animation, say_yes, happy_bounce"
        },
        "vrma_import": {
            "format": "vrm_local_rotations: true + rotation_quat per joint (tool-generated)",
            "when": "clapping, jumping, dance, any full-body mocap",
            "how": "POST /import/vrma with path under assets/motions/, or play vrma_clapping / vrma_jump / vrma_goodbye",
            "do_not": "Ask an LLM to write quaternion keyframes — use import or pose-ref choreography instead"
        },
        "runtime_registration": "POST /poses and POST /animations validate like YAML on disk; no restart required",
        "library_refresh": "Shipped assets load at startup; HTTP import/register updates live maps immediately"
    })
}

pub fn joint_target_shape() -> Value {
    json!({
        "rotation_deg": {
            "x": "number, optional, default 0",
            "y": "number, optional, default 0",
            "z": "number, optional, default 0",
        },
        "rotation_quat": "[x, y, z, w] glTF local quaternion — VRMA imports only; \
            preferred over rotation_deg when set. Do not hand-author for new agent poses.",
        "translation": "[x, y, z] optional — hip root motion in VRMA samples; rare in hand poses",
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use paperdoll_rig::{load_animations_from_dir, load_poses_from_dir};
    use std::path::Path;

    fn repo_assets() -> (HashMap<String, Pose>, HashMap<String, Animation>) {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap();
        let poses = load_poses_from_dir(&root.join("assets/poses")).unwrap();
        let animations =
            load_animations_from_dir(&root.join("assets/animations"), &poses).unwrap();
        (poses, animations)
    }

    #[test]
    fn example_boards_cover_shipped_exemplars() {
        let (poses, animations) = repo_assets();
        let ex_poses = build_example_poses(&poses);
        assert!(
            ex_poses.len() >= EXAMPLE_POSE_BOARD.len() - 1,
            "pose board entries should resolve (got {})",
            ex_poses.len()
        );
        let ex_anims = build_example_animations(&animations, &poses);
        assert_eq!(
            ex_anims.len(),
            EXAMPLE_ANIMATION_BOARD.len(),
            "every animation board entry must exist in assets"
        );
    }

    #[test]
    fn catalogs_include_vrma_and_say_clips() {
        let (poses, animations) = repo_assets();
        let catalog = build_animation_catalog(&animations, &poses);
        let names: Vec<_> = catalog
            .iter()
            .filter_map(|v| v.get("name").and_then(|n| n.as_str()))
            .collect();
        for need in [
            "vrma_clapping",
            "say_yes",
            "say_no",
            "wave_animation",
        ] {
            assert!(names.contains(&need), "missing {need} in catalog");
        }
    }
}
