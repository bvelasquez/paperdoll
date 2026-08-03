//! The HTTP pose/animation API (M5, extended): lets an external caller — a script, or
//! an AI agent — trigger the rig's poses and animations at runtime, register brand
//! new ones into the live library, and introspect the whole API via
//! `GET /capabilities` without having read this file.
//!
//! *Triggering* playback (`POST /pose`, `POST /animation`) mutates ECS-only state
//! (`RigPlayback`), which only a Bevy system can safely touch — those go over a
//! `crossbeam_channel` to `rig_bridge::apply_rig_commands`, since this server runs on
//! its own OS thread with its own Tokio runtime and Bevy's `World` isn't otherwise
//! shared with it. *Registering* a pose/animation (`POST /poses`, `POST /animations`)
//! is just inserting into a `HashMap` — it needs no ECS access at all, so those
//! handlers write directly into the `Arc<RwLock<_>>` shared with `PoseLibrary`/
//! `AnimationLibrary` instead of round-tripping through a command + system.

use crate::agent_capabilities::{
    build_animation_catalog, build_example_animations, build_example_poses, build_pose_catalog,
    camera_framing, joint_target_shape, motion_sources, timing_guide, AGENT_SYSTEM_PROMPT,
};
use crate::editor_state::SharedEditorState;
use crate::live_state::{LiveState, LiveStateSnapshot};
use crate::rig_bridge::{
    AnimationLibrary, PoseLibrary, RigCommand, RigCommandReceiver, RigCommandSender,
    ANIMATIONS_DIR, POSES_DIR,
};
use crate::vrma_import::{import_all_demo_motions, import_vrma_file, safe_assets_relative_path};
use crate::screenshot_bridge::{ScreenshotRequest, ScreenshotRequestReceiver};
use crate::v2_expressions::SharedExpressionState;
use crate::variant::{DollVariant, SharedVariantState};
use axum::{
    body::Body,
    extract::State,
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use bevy::prelude::*;
use paperdoll_rig::{resolve_animation, Animation, AnimationFile, Easing, Pose, Skeleton, VrmaImportConfig};
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, RwLock};
use std::time::Duration;

/// Only intended for a local script or agent to call, so this binds to localhost
/// rather than 0.0.0.0.
const HTTP_ADDR: &str = "127.0.0.1:7878";

/// How long `GET /screenshot` waits for the render thread to hand back a capture
/// before giving up. Generous relative to a normal frame time (a capture round-trips
/// through a couple of render frames), but bounded so a request can't hang forever if
/// the app is stalled or the window has no surface (e.g. minimized on some platforms).
const SCREENSHOT_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone)]
struct ApiState {
    poses: Arc<RwLock<HashMap<String, Pose>>>,
    animations: Arc<RwLock<HashMap<String, Animation>>>,
    live_state: LiveState,
    variant: SharedVariantState,
    expressions: SharedExpressionState,
    commands: crossbeam_channel::Sender<RigCommand>,
    screenshots: crossbeam_channel::Sender<ScreenshotRequest>,
    editor: SharedEditorState,
}

fn editor_conflict() -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::CONFLICT,
        error_body(
            "in-app pose/animation editor is open — close it (F2) before triggering playback",
        ),
    )
}

#[derive(serde::Deserialize)]
struct PoseCommandRequest {
    name: String,
    #[serde(default)]
    speed_deg_per_sec: Option<f32>,
}

#[derive(serde::Deserialize)]
struct AnimationCommandRequest {
    name: String,
}

#[derive(serde::Deserialize)]
struct VariantCommandRequest {
    variant: DollVariant,
}

async fn get_variant(State(state): State<ApiState>) -> Json<serde_json::Value> {
    Json(serde_json::to_value(state.variant.snapshot()).unwrap())
}

async fn post_variant(
    State(state): State<ApiState>,
    Json(req): Json<VariantCommandRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    if req.variant == DollVariant::V2 && !state.variant.v2_ready() {
        let snap = state.variant.snapshot();
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            error_body(format!(
                "variant v2 requires VRM at assets/{} — file not found",
                snap.v2_character
            )),
        );
    }
    let _ = state.commands.send(RigCommand::SetVariant {
        variant: req.variant,
    });
    (
        StatusCode::ACCEPTED,
        Json(serde_json::json!({ "variant": req.variant })),
    )
}

async fn get_expressions(State(state): State<ApiState>) -> Json<serde_json::Value> {
    Json(serde_json::to_value(state.expressions.snapshot()).unwrap())
}

#[derive(serde::Deserialize)]
struct ExpressionsRequest {
    /// Preset weights in \[0, 1\]. Omitted presets are left unchanged unless
    /// `reset` is true.
    #[serde(default)]
    weights: HashMap<String, f32>,
    /// If true, zero every known expression before applying `weights`.
    #[serde(default)]
    reset: bool,
}

async fn post_expressions(
    State(state): State<ApiState>,
    Json(req): Json<ExpressionsRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    let snap = state.expressions.snapshot();
    if !snap.ready {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            error_body(
                "expressions require variant v2 with a bound VRM that has \
                 VRMC_vrm.expressions (try POST /variant {\"variant\":\"v2\"} first)",
            ),
        );
    }
    let mut weights = HashMap::new();
    if req.reset {
        for name in &snap.available {
            weights.insert(name.clone(), 0.0);
        }
    }
    for (k, v) in req.weights {
        if !snap.available.iter().any(|n| n == &k) {
            return (
                StatusCode::BAD_REQUEST,
                error_body(format!(
                    "unknown expression '{k}'; see GET /expressions for available presets"
                )),
            );
        }
        weights.insert(k, v.clamp(0.0, 1.0));
    }
    if weights.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            error_body("provide weights and/or reset: true"),
        );
    }
    let _ = state
        .commands
        .send(RigCommand::SetExpressions { weights: weights.clone() });
    (
        StatusCode::ACCEPTED,
        Json(serde_json::json!({ "weights": weights })),
    )
}

fn error_body(message: impl Into<String>) -> Json<serde_json::Value> {
    Json(serde_json::json!({ "error": message.into() }))
}

async fn post_pose(
    State(state): State<ApiState>,
    Json(req): Json<PoseCommandRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    if state.editor.is_active() {
        return editor_conflict();
    }
    if !state.poses.read().unwrap().contains_key(&req.name) {
        return (
            StatusCode::NOT_FOUND,
            error_body(format!("unknown pose '{}'", req.name)),
        );
    }
    let _ = state.commands.send(RigCommand::Pose {
        name: req.name.clone(),
        speed_deg_per_sec: req.speed_deg_per_sec,
    });
    (
        StatusCode::ACCEPTED,
        Json(serde_json::json!({ "pose": req.name })),
    )
}

async fn post_animation(
    State(state): State<ApiState>,
    Json(req): Json<AnimationCommandRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    if state.editor.is_active() {
        return editor_conflict();
    }
    if !state.animations.read().unwrap().contains_key(&req.name) {
        return (
            StatusCode::NOT_FOUND,
            error_body(format!("unknown animation '{}'", req.name)),
        );
    }
    let _ = state
        .commands
        .send(RigCommand::Animation { name: req.name.clone() });
    (
        StatusCode::ACCEPTED,
        Json(serde_json::json!({ "animation": req.name })),
    )
}

async fn get_poses(State(state): State<ApiState>) -> Json<Vec<String>> {
    let mut names: Vec<String> = state.poses.read().unwrap().keys().cloned().collect();
    names.sort();
    Json(names)
}

async fn get_animations(State(state): State<ApiState>) -> Json<Vec<String>> {
    let mut names: Vec<String> = state.animations.read().unwrap().keys().cloned().collect();
    names.sort();
    Json(names)
}

/// Current joint rotations (degrees) + camera orbit + playback mode. Updated every
/// Bevy frame from the live `PlaybackState` snapshot — agents use this alongside
/// `GET /screenshot` to close the verify loop without reading pixels.
async fn get_state(State(state): State<ApiState>) -> Json<LiveStateSnapshot> {
    Json(state.live_state.0.read().unwrap().clone())
}

/// Registers a new pose (or overwrites an existing one by name) directly into the
/// shared library — no channel needed, since this is a pure data write with no ECS
/// involvement. Joint names are validated against the skeleton immediately (reusing
/// `Pose::resolve`, the same validation a loaded-from-YAML pose gets), so a caller
/// finds out about a typo'd joint name at registration time, not the first time
/// something tries to play the pose.
async fn post_register_pose(
    State(state): State<ApiState>,
    Json(pose): Json<Pose>,
) -> (StatusCode, Json<serde_json::Value>) {
    let skeleton = Skeleton::humanoid_default();
    if let Err(e) = pose.resolve(&skeleton) {
        return (StatusCode::BAD_REQUEST, error_body(e.to_string()));
    }
    if let Err(e) = state.expressions.validate_names(pose.expressions.keys()) {
        return (StatusCode::BAD_REQUEST, error_body(e));
    }
    let name = pose.name.clone();
    let replaced = state
        .poses
        .write()
        .unwrap()
        .insert(name.clone(), pose)
        .is_some();
    (
        if replaced {
            StatusCode::OK
        } else {
            StatusCode::CREATED
        },
        Json(serde_json::json!({ "pose": name })),
    )
}

/// Registers a new animation the same way `post_register_pose` registers a pose.
/// `resolve_animation` (shared with the YAML loader — see `paperdoll_rig::yaml`) does
/// the validation: every keyframe must set exactly one of `pose` (which must already
/// exist — including any pose registered moments earlier via `POST /poses`, since
/// both read the same live map) or inline `joints`.
async fn post_register_animation(
    State(state): State<ApiState>,
    Json(file): Json<AnimationFile>,
) -> (StatusCode, Json<serde_json::Value>) {
    let name = file.name.clone();
    // Validate expression keys on the raw keyframes before resolve (covers pose
    // overlays and expression-only beats).
    for (i, kf) in file.keyframes.iter().enumerate() {
        if let Some(expr) = &kf.expressions {
            if let Err(e) = state.expressions.validate_names(expr.keys()) {
                return (
                    StatusCode::BAD_REQUEST,
                    error_body(format!("keyframe {i}: {e}")),
                );
            }
        }
    }
    let resolved = {
        let poses = state.poses.read().unwrap();
        // Also validate expressions carried by referenced poses.
        for (i, kf) in file.keyframes.iter().enumerate() {
            if let Some(pose_name) = &kf.pose {
                if let Some(pose) = poses.get(pose_name) {
                    if let Err(e) = state.expressions.validate_names(pose.expressions.keys()) {
                        return (
                            StatusCode::BAD_REQUEST,
                            error_body(format!("keyframe {i} pose '{pose_name}': {e}")),
                        );
                    }
                }
            }
        }
        resolve_animation(file, &poses)
    };
    match resolved {
        Ok(animation) => {
            let replaced = state
                .animations
                .write()
                .unwrap()
                .insert(name.clone(), animation)
                .is_some();
            (
                if replaced {
                    StatusCode::OK
                } else {
                    StatusCode::CREATED
                },
                Json(serde_json::json!({ "animation": name })),
            )
        }
        Err(e) => (StatusCode::BAD_REQUEST, error_body(e.to_string())),
    }
}

#[derive(serde::Deserialize)]
struct ImportVrmaRequest {
    /// Path relative to `assets/`, e.g. `motions/Clapping.vrma`.
    path: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    sample_interval_ms: Option<u32>,
    #[serde(default)]
    write_yaml: Option<bool>,
    #[serde(default)]
    play: Option<bool>,
    #[serde(default)]
    r#loop: Option<bool>,
}

/// Import a `.vrma` from `assets/motions/` (or another path under `assets/motions/`),
/// register it in the live animation library, and optionally write YAML + trigger playback.
async fn post_import_vrma(
    State(state): State<ApiState>,
    Json(req): Json<ImportVrmaRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    let rel = match safe_assets_relative_path(&req.path) {
        Ok(p) => p,
        Err(e) => return (StatusCode::BAD_REQUEST, error_body(e)),
    };
    let rel_str = rel.to_string_lossy();
    if !rel_str.starts_with("motions/") {
        return (
            StatusCode::BAD_REQUEST,
            error_body(format!(
                "import path must be under `motions/` (relative to assets/), got '{rel_str}'"
            )),
        );
    }
    let vrma_path = Path::new("assets").join(&rel);
    if !vrma_path.is_file() {
        return (
            StatusCode::NOT_FOUND,
            error_body(format!("VRMA file not found at '{}'", vrma_path.display())),
        );
    }

    let mut config = VrmaImportConfig::from_path_stem(&vrma_path);
    if let Some(name) = req.name {
        config.name = paperdoll_rig::sanitize_asset_filename(&name);
    }
    if let Some(ms) = req.sample_interval_ms {
        config.sample_interval_ms = ms.max(1);
    }
    if let Some(looping) = req.r#loop {
        config.looping = looping;
    }
    let write_yaml = req.write_yaml.unwrap_or(true);

    let outcome = match import_vrma_file(
        &vrma_path,
        config,
        Path::new(ANIMATIONS_DIR),
        write_yaml,
    ) {
        Ok(o) => o,
        Err(e) => return (StatusCode::BAD_REQUEST, error_body(e.to_string())),
    };

    let anim_name = outcome.animation.name.clone();
    let replaced = state
        .animations
        .write()
        .unwrap()
        .insert(anim_name.clone(), outcome.animation)
        .is_some();

    if req.play.unwrap_or(false) {
        if state.editor.is_active() {
            return editor_conflict();
        }
        if state.commands.send(RigCommand::Animation { name: anim_name.clone() }).is_err() {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                error_body("failed to queue animation playback"),
            );
        }
    }

    (
        if replaced {
            StatusCode::OK
        } else {
            StatusCode::CREATED
        },
        Json(serde_json::json!({
            "animation": anim_name,
            "duration_ms": outcome.result.duration_ms,
            "keyframes": outcome.result.keyframe_count,
            "mapped_bones": outcome.result.mapped_bone_count,
            "yaml": outcome.yaml_path.as_ref().map(|p| p.display().to_string()),
            "played": req.play.unwrap_or(false),
        })),
    )
}

/// Fetch catalog demos (if needed), import to YAML, and register all `vrma_*` animations.
async fn post_import_demo_motions(
    State(state): State<ApiState>,
) -> (StatusCode, Json<serde_json::Value>) {
    let outcomes = match import_all_demo_motions(Path::new("."), true) {
        Ok(o) => o,
        Err(e) => return (StatusCode::BAD_REQUEST, error_body(e)),
    };
    let mut names = Vec::new();
    for outcome in outcomes {
        let name = outcome.animation.name.clone();
        state
            .animations
            .write()
            .unwrap()
            .insert(name.clone(), outcome.animation);
        names.push(name);
    }
    (
        StatusCode::CREATED,
        Json(serde_json::json!({ "animations": names })),
    )
}

/// Captures the primary window and returns it as a PNG — lets a caller (human or
/// agent) see the rig's current pose without needing eyes on the actual window,
/// closing the loop with `POST /pose`/`POST /animation` entirely over HTTP. The
/// capture itself must happen on Bevy's side (`screenshot_bridge::handle_screenshot_requests`);
/// this just sends the request and blocks (off the async runtime, via
/// `spawn_blocking`) on a oneshot reply channel until the bytes come back or
/// `SCREENSHOT_TIMEOUT` elapses.
async fn get_screenshot(State(state): State<ApiState>) -> Response {
    let (reply_tx, reply_rx) = crossbeam_channel::bounded(1);
    if state
        .screenshots
        .send(ScreenshotRequest { reply: reply_tx })
        .is_err()
    {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            error_body("screenshot request channel closed"),
        )
            .into_response();
    }

    let recv_result =
        tokio::task::spawn_blocking(move || reply_rx.recv_timeout(SCREENSHOT_TIMEOUT)).await;

    match recv_result {
        Ok(Ok(bytes)) if !bytes.is_empty() => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "image/png")],
            Body::from(bytes),
        )
            .into_response(),
        Ok(Ok(_)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            error_body("screenshot capture failed (see server logs)"),
        )
            .into_response(),
        Ok(Err(_)) => (
            StatusCode::GATEWAY_TIMEOUT,
            error_body("timed out waiting for screenshot capture"),
        )
            .into_response(),
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            error_body("screenshot task panicked"),
        )
            .into_response(),
    }
}

/// Self-describing capabilities document: every endpoint, its request/response shape,
/// the full list of valid joint names, and the full list of valid easing values — the
/// idea being an LLM/agent that has never seen this codebase can `GET /capabilities`
/// once and then use the whole API correctly, including registering new content.
///
/// `example_poses` are pulled live from the pose library (not hardcoded joint maps) so
/// this document cannot drift from the poses `POST /pose` actually plays.
async fn get_capabilities(State(state): State<ApiState>) -> Json<serde_json::Value> {
    let skeleton = Skeleton::humanoid_default();
    let joint_names: Vec<String> = skeleton.joints().map(|(_, j)| j.name.clone()).collect();
    // Serialize each variant rather than hand-writing the string list, so this can't
    // silently drift from `Easing`'s actual `#[serde(rename_all = "snake_case")]` wire
    // representation if a variant is ever renamed or added.
    let easing_options: Vec<serde_json::Value> = Easing::ALL
        .iter()
        .map(|e| serde_json::to_value(e).unwrap())
        .collect();

    let (example_poses, example_animations, pose_catalog, animation_catalog) = {
        let poses = state.poses.read().unwrap();
        let animations = state.animations.read().unwrap();
        (
            build_example_poses(&poses),
            build_example_animations(&animations, &poses),
            build_pose_catalog(&poses),
            build_animation_catalog(&animations, &poses),
        )
    };

    Json(serde_json::json!({
        "name": "paperdoll pose API",
        "description": "HTTP API for posing and animating a 3D paper-doll humanoid \
            (v1 procedural or v2 VRM skinned) running in paperdoll-app. Trigger a \
            loaded pose or animation to make the rig move immediately; register new \
            poses/animations at runtime to extend what's available, no server restart \
            required. Switch visuals with GET/POST /variant.",
        "variant": state.variant.snapshot(),
        "expressions": state.expressions.snapshot(),
        "in_app_editor": {
            "toggle": "F2 in the paperdoll window",
            "pose_authoring": "Live preview, joint tree + Euler sliders, capture scene, save to assets/poses/*.yaml",
            "animation_authoring": "Keyframe list, scrub/play/loop preview, save to assets/animations/*.yaml",
            "http_while_open": "POST /pose and POST /animation return 409 Conflict; registration endpoints unchanged"
        },
        "v2": {
            "description": "Default visual. VRM 1.0 skinned mesh with 65 shared \
                joints (body + face cosmetics + fingers). Drive fingers via pose \
                joints (*_thumb_*, *_index_*, *_middle_*, *_ring_*, *_little_*). \
                Drive face via pose/keyframe `expressions` weights (synced with \
                motion) or                 GET/POST /expressions. Authoring models: wave_animation, say_yes, \
                say_no, happy_bounce, finger_emote, point_hero, orbit_victory. \
                Full-body mocap: play vrma_clapping / vrma_jump / vrma_goodbye or \
                POST /import/vrma — do not hand-write quaternion clips.",
            "finger_joints": joint_names.iter().filter(|n| {
                n.contains("_thumb_") || n.contains("_index_") || n.contains("_middle_")
                    || n.contains("_ring_") || n.contains("_little_")
            }).cloned().collect::<Vec<_>>(),
            "expression_endpoints": ["GET /expressions", "POST /expressions"],
            "preferred": true,
        },
        "launch": {
            "cli": "paperdoll [--variant v1|v2]",
            "env": "PAPERDOLL_VARIANT=v1|v2, PAPERDOLL_V2_CHARACTER=characters/default.vrm",
            "default": "v2",
        },
        "skeleton": {
            "joint_count": joint_names.len(),
            "joint_names": joint_names,
            "notes": "Same 65-joint API for v1 and v2. Finger joints animate the \
                mesh on v2; on v1 they are accepted but not visualized. Eight face \
                cosmetics (pupils, eyelids, eyebrows, blush) are v1-only.",
        },
        "easing_options": easing_options,
        "posing_guide": {
            "summary": "Which axis moves a joint isn't guessable from the joint name \
                alone — it depends on which direction that joint's rest offset points. \
                This section says the rule once so you don't have to rediscover it by \
                trial and error (POST a pose, GET /screenshot, repeat).",
            "rest_pose": "Every joint's rest rotation is identity. The skeleton stands \
                in a T-pose: arms straight out to the sides, legs straight down, toes \
                pointing forward. `left_*` joints extend toward local +X, `right_*` \
                joints toward local -X — the two sides are mirror images.",
            "the_rule": "Every limb joint (shoulder/elbow/wrist/hand, hip/knee/ankle) \
                has a rest offset direction toward its child. Rotating that joint \
                around the axis PARALLEL to its own offset is a no-op — it's a roll \
                around the limb's own length, invisible on a rigid capsule. Rotating \
                around either PERPENDICULAR axis actually swings the limb.",
            "arm_chain": {
                "offset_direction": "local X (shoulder/elbow/wrist/hand all extend \
                    along X in rest pose)",
                "x_rotation_deg": "no visible effect (roll around the arm's own axis)",
                "z_rotation_deg": "swings the limb up/down from T-pose — the useful \
                    'raise / lower / bend elbow' axis from the default camera",
                "z_sign_from_t_pose": "RIGHT arm: negative z raises overhead, positive z \
                    lowers toward the hip/side. LEFT arm: positive z raises, negative z \
                    lowers. So a two-armed overhead raise is right.z=-80, left.z=+80 \
                    (see `victory`). A right-arm cartoon wave uses mild raise \
                    right_shoulder.z around -22 (not -55) plus elbow bend — see `wave`.",
                "y_rotation_deg": "swings the limb forward/backward, toward or away \
                    from the camera. Compose with z when you need a hand in front of \
                    the torso/face (see `think`, `point`).",
                "y_sign_from_t_pose": "RIGHT arm: POSITIVE y swings forward toward the \
                    camera / in front of the torso; NEGATIVE y swings behind the body. \
                    `point` uses right_shoulder.y ≈ +75 (forward); y ≈ -75 points \
                    backwards. `think` also uses positive y to bring the hand forward \
                    to the face. Always verify forward/back with a side camera \
                    (yaw ≈ 70–80) — front views foreshorten and lie.",
                "wave_vs_think_pitfall": "Do not combine a large negative right_shoulder.z \
                    (high raise) with a deep negative elbow z — that folds the hand onto \
                    the head (think). A readable wave keeps shoulder z mild (~-20), \
                    elbow ~-70, and positive shoulder y so the palm faces out.",
            },
            "leg_chain": {
                "offset_direction": "local Y (hip/knee/ankle all extend downward \
                    along -Y in rest pose)",
                "y_rotation_deg": "no visible effect (roll around the leg's own axis)",
                "x_rotation_deg": "swings the limb forward/backward — the 'bend the \
                    knee' / 'raise the leg' axis, clearly visible from the front. \
                    Positive x on the knee bends it naturally for idle weight-shift.",
                "z_rotation_deg": "swings the limb sideways (abduction) — treat as \
                    unverified; prefer small values and verify with GET /screenshot.",
            },
            "torso_chain": "pelvis/spine/chest/upper_chest/neck are stacked along Y; \
                rotating one tilts everything above it. Prefer spine/chest x for a bow \
                — pelvis x also tips the legs (hips are pelvis children) and reads as \
                the whole figure tipping over. pelvis.z is a side-to-side hip cock \
                (see `idle`).",
            "head_chain": "neck is the primary head joint. rotation_deg.y turns the \
                face left/right (see head_turn_left/right ≈ ±42°). rotation_deg.x nods \
                chin down (+) or up (−) with head_nod_down/up. Compose head animations \
                from those pose names (say_yes, say_no) instead of guessing neck angles. \
                Head poses ship with face close-up camera — copy their camera blocks.",
            "face_chain": "On v2 (default): put VRM morph weights in pose/keyframe \
                `expressions` (happy, blink, blinkLeft, blinkRight, aa/ih/ou/ee/oh, \
                angry, sad, relaxed — see GET /expressions). Weights blend with the \
                same easing as joints; unlisted presets fade to 0 unless hold:true \
                (sparse overlay for blinks). Do NOT rely on v1 pupil/eyelid/eyebrow/\
                blush joints for v2 face — they are cosmetics on the procedural doll. \
                On v1 only: Face joints live on the +Z (front) surface of the head. \
                jaw: rotation X opens the mouth; *_eyelid: rotation X closes the lid \
                (~85=closed); *_eyebrow: rotation Z mirrored; *_pupil / *_blush: use \
                translation.",
            "finger_chain": "v2 finger joints: left|right_{thumb,index,middle,ring,\
                little}_{metacarpal|proximal|intermediate|distal} (thumb has \
                metacarpal/proximal/distal). Curl toward a fist with positive z on \
                proximal/intermediate/distal (~80–95). See peace_sign_right and \
                finger_emote for worked finger recipes.",
            "mirrored_sign_convention": "left_* and right_* joints are mirror images \
                (opposite sign on the rest offset's X component), so the SAME visual \
                pose on both sides needs OPPOSITE-signed rotation_deg.z — e.g. \
                `hands_on_hips` uses right_shoulder.z=45 and left_shoulder.z=-45 to \
                put a hand on each hip symmetrically. When adapting a one-sided pose \
                to the other side, negate z (and whichever other axis you're using).",
            "sparse_poses": "A pose only lists joints it changes; omitted joints return \
                to the skeleton rest transform when blending into that pose. An empty \
                joints map (`t_pose`) is a full reset to rest. Expression weights work \
                the same way: omitted presets fade to 0 (unless hold). Always verify \
                with GET /screenshot after POST /pose.",
            "how_to_verify_a_new_pose": "POST it to /poses, POST its name to /pose, \
                then GET /screenshot — don't assume an angle looks right without \
                checking, since the axis conventions above are about direction, not \
                magnitude, and every body is a different scale.",
            "worked_examples": "See example_poses below — each is a live pose from \
                the library (same joints POST /pose will play), chosen to demonstrate \
                one rule above concretely. Agents authoring new poses should COPY \
                magnitudes from these maps rather than inventing angles from memory.",
            "agent_workflow": "1) GET /capabilities (agent_system_prompt + posing_guide + \
                example_poses + example_animations + timing_guide + camera_framing). \
                2) Prefer referencing existing pose names in animations. 3) If inventing \
                joints, start from a close example_pose and tweak. 4) POST /poses, \
                POST /pose, GET /screenshot — use side camera (yaw≈75) for forward/back. \
                5) If an animation references an updated pose, re-POST /animations. \
                6) Full-body motion: POST /animation vrma_* or POST /import/vrma.",
        },
        "timing_guide": timing_guide(),
        "camera_framing": camera_framing(),
        "motion_sources": motion_sources(),
        "agent_system_prompt": AGENT_SYSTEM_PROMPT,
        "example_poses": example_poses,
        "example_animations": example_animations,
        "pose_catalog": pose_catalog,
        "animation_catalog": animation_catalog,
        "camera_shape": {
            "yaw_deg": "number, optional — horizontal orbit around look_at (degrees). \
                0 looks along +Z; positive toward +X. Omitted = keep current yaw.",
            "pitch_deg": "number, optional — vertical orbit angle (degrees). Positive \
                looks down from above. Omitted = keep current pitch.",
            "distance": "number, optional — orbit radius / zoom (world units). Smaller \
                = tighter close-up. Omitted = keep current distance.",
            "look_at": "[x, y, z] number array, optional — world point the camera \
                looks at (a pan). Default framing looks at [0, 0.9, 0].",
            "note": "Camera is sparse like joints. Put `camera` on a pose, and/or on \
                an animation keyframe (keyframe fields overlay the pose's camera). A \
                keyframe with only `camera` (no pose/joints) holds the body still \
                while the camera moves — for orbit/push-in choreography.",
            "default": {
                "yaw_deg": 35.5,
                "pitch_deg": 9.2,
                "distance": 4.36,
                "look_at": [0.0, 0.9, 0.0],
            },
        },
        "pose_shape": {
            "name": "string, required — unique identifier used to reference this pose",
            "description": "string, optional",
            "joints": {
                "<joint_name>": joint_target_shape(),
            },
            "camera": "object, optional — see `camera_shape` above",
            "expressions": "object, optional — VRM morph preset → weight in [0,1] \
                (v2). Keys from GET /expressions `available`. Blended with joints; \
                omitted presets fade to 0. Example: {\"happy\": 1.0, \"blink\": 0.0}",
            "note": "Every field under `joints` is sparse: a joint not listed keeps \
                the skeleton's rest transform, and within a listed joint an omitted \
                `rotation_deg`/`translation` keeps that specific part of the rest \
                transform. Joint names must be one of `skeleton.joint_names` above.",
        },
        "animation_shape": {
            "name": "string, required",
            "description": "string, optional",
            "vrm_local_rotations": "boolean, optional, default false — when true, joint \
                targets use rotation_quat (glTF local) from VRMA import; Euler \
                rotation_deg in hand-authored clips must leave this false.",
            "loop": "boolean, optional, default false — authoring hint for sequences \
                that could repeat; POST /animation always plays one cycle then returns \
                to the default idle pose (YAML loop is ignored at trigger time)",
            "play_automatically": "boolean, optional, default false — when true, the \
                doll window may pick this animation at random while idle (bored autoplay; \
                interval via PAPERDOLL_BORED_INTERVAL_SECS)",
            "keyframes": [
                {
                    "pose": "string, optional — a name from GET /poses, resolved at \
                        registration time (joint values are copied into the animation). \
                        If you later POST /poses to update a referenced pose, re-POST \
                        this animation too or it keeps the old joint values.",
                    "joints": "object, optional — inline joint targets; same shape as \
                        a pose's `joints` field (see joint_target_shape in pose_shape)",
                    "camera": "object, optional — see `camera_shape`; overlays the \
                        referenced pose's camera when both are set",
                    "expressions": "object, optional — VRM morph weights; overlays the \
                        referenced pose's expressions when both are set. Expression-only \
                        keyframes (no pose/joints/camera) hold the body like camera-only.",
                    "duration_ms": "integer, required — time to blend into this \
                        keyframe from wherever the previous one (or the entry \
                        transition, for keyframe 0) left off",
                    "easing": "one of `easing_options` above, default \"linear\"",
                    "hold": "boolean, optional, default false — true makes this keyframe \
                        a sparse OVERLAY: only its listed joints/expressions move; every \
                        unlisted joint/expression keeps its current value. Use for \
                        blink overlays on top of an ongoing body pose.",
                    "note": "Each keyframe needs `pose`, `joints`, `camera`, and/or \
                        `expressions` (and must not set both `pose` and `joints`). \
                        Camera-only / expression-only keyframes hold the body.",
                },
            ],
        },
        "endpoints": [
            {
                "method": "GET",
                "path": "/capabilities",
                "description": "This document — includes `agent_system_prompt`, \
                    `posing_guide`, `timing_guide`, `camera_framing`, `motion_sources`, \
                    `example_poses`, `example_animations`, `pose_catalog`, and \
                    `animation_catalog`.",
            },
            {
                "method": "GET",
                "path": "/state",
                "description": "Live snapshot: playback mode, current joint \
                    rotations in degrees (sparse — only joints present in the held \
                    blend), and the current orbit camera. Use with GET /screenshot \
                    to verify poses numerically and visually.",
                "response_body": "{ playback: { mode, animation?, keyframe_index? }, \
                    joints: { <name>: { rotation_deg: {x,y,z} } }, camera: camera_shape, \
                    expressions?: { <preset>: number } }",
            },
            {
                "method": "GET",
                "path": "/poses",
                "description": "List the name of every pose currently in the \
                    library (loaded from assets/poses/*.yaml at startup, plus \
                    anything registered since via POST /poses).",
                "response_body": "array of pose name strings",
            },
            {
                "method": "GET",
                "path": "/animations",
                "description": "Like GET /poses, for animations.",
                "response_body": "array of animation name strings",
            },
            {
                "method": "GET",
                "path": "/screenshot",
                "description": "Captures the primary window and returns it as a PNG. \
                    Use this to see the result of a POST /pose or POST /animation \
                    without needing eyes on the actual window — e.g. an agent can \
                    POST /pose then GET /screenshot to verify the pose looks right.",
                "response_body": "200 with Content-Type: image/png and the raw PNG \
                    bytes; 504 {\"error\": \"...\"} if capture times out",
            },
            {
                "method": "GET",
                "path": "/variant",
                "description": "Which visual is active: v1 (procedural capsule doll) \
                    or v2 (VRM skinned humanoid). Pose/animation APIs are shared.",
                "response_body": "{ variant, available, v2_character, v2_asset_present, \
                    description }",
            },
            {
                "method": "POST",
                "path": "/variant",
                "description": "Switch the live visual (despawn current, spawn the \
                    other). Does not restart the process. Requires the v2 VRM file \
                    on disk when switching to v2.",
                "request_body": {
                    "variant": "\"v1\" | \"v2\"",
                },
                "response_body": "202 Accepted {\"variant\": \"v1\"|\"v2\"}; \
                    503 if v2 asset is missing",
            },
            {
                "method": "GET",
                "path": "/expressions",
                "description": "VRM expression presets (blend shapes) for the active \
                    v2 character: available names and current weights. Empty/not ready \
                    on v1 or before the VRM finishes binding.",
                "response_body": "{ ready, available, weights }",
            },
            {
                "method": "POST",
                "path": "/expressions",
                "description": "Set VRM expression weights (v2 only). Values are \
                    clamped to [0,1]. Use reset:true to zero all presets first.",
                "request_body": {
                    "weights": "object of preset_name → number, optional",
                    "reset": "bool, optional — zero all known expressions first",
                },
                "response_body": "202 Accepted {\"weights\": {...}}; 503 if v2 \
                    expressions not ready; 400 if body empty",
            },
            {
                "method": "POST",
                "path": "/pose",
                "description": "Start a live transition of the rig into a named \
                    pose. Transition duration is derived from the pose's angular \
                    distance from wherever the rig currently is, at a configurable \
                    default speed (~180 deg/sec) unless overridden per call.",
                "request_body": {
                    "name": "string, required — must be a name from GET /poses",
                    "speed_deg_per_sec": "number, optional — overrides the default \
                        transition speed for just this command",
                },
                "response_body": "202 Accepted {\"pose\": \"<name>\"}; \
                    404 {\"error\": \"...\"} if the name is unknown",
            },
            {
                "method": "POST",
                "path": "/animation",
                "description": "Start playing a named animation sequence from its \
                    first keyframe. After the entry transition, each keyframe plays \
                    at its own authored duration_ms/easing. Keyframe `camera` fields \
                    drive orbit choreography.",
                "request_body": {
                    "name": "string, required — must be a name from GET /animations",
                },
                "response_body": "202 Accepted {\"animation\": \"<name>\"}; \
                    404 {\"error\": \"...\"} if the name is unknown",
            },
            {
                "method": "POST",
                "path": "/poses",
                "description": "Register a new pose (or overwrite an existing one by \
                    name) in the live library. Immediately available to POST /pose \
                    and to POST /animations keyframes referencing it by name. See \
                    pose_shape above for the request body.",
                "request_body": "a pose, see `pose_shape` above",
                "response_body": "201 Created (200 OK if a pose with that name \
                    already existed and was replaced) {\"pose\": \"<name>\"}; \
                    400 {\"error\": \"...\"} if a joint name is unknown",
            },
            {
                "method": "POST",
                "path": "/animations",
                "description": "Register a new animation (or overwrite an existing \
                    one by name). See animation_shape above for the request body.",
                "request_body": "an animation, see `animation_shape` above",
                "response_body": "201 Created (200 OK if replaced) \
                    {\"animation\": \"<name>\"}; 400 {\"error\": \"...\"} if a \
                    keyframe's pose reference or inline joint name is invalid",
            },
            {
                "method": "POST",
                "path": "/import/vrma",
                "description": "Import a VRM Animation (.vrma) from assets/motions/ \
                    into sampled YAML keyframes, register in the live library, and \
                    optionally play. Body + VRM expression curves are sampled; camera \
                    is not imported (add in the editor or YAML).",
                "request_body": "{ \"path\": \"motions/Clapping.vrma\", \"name\": \
                    \"optional_id\", \"sample_interval_ms\": 100, \"write_yaml\": true, \
                    \"play\": false, \"loop\": false }",
                "response_body": "201 Created {\"animation\",\"duration_ms\",\"keyframes\",\
                    \"mapped_bones\",\"yaml\",\"played\"}; 404 if file missing; 409 if \
                    play:true while editor open",
            },
            {
                "method": "POST",
                "path": "/import/demo-motions",
                "description": "Fetch built-in VRMA demos, write YAML, register \
                    vrma_clapping / vrma_jump / vrma_goodbye (no playback).",
                "response_body": "201 {\"animations\": [\"vrma_clapping\", ...]}",
            },
        ],
    }))
}

/// Startup system: clones the shared `PoseLibrary`/`AnimationLibrary` (the exact
/// instances `main()` inserted and the visual spawn path also reads), wires up the
/// command and screenshot-request channels, and spawns the server thread.
pub fn start_http_server(
    mut commands: Commands,
    poses: Res<PoseLibrary>,
    animations: Res<AnimationLibrary>,
    live_state: Res<LiveState>,
    variant: Res<SharedVariantState>,
    expressions: Res<SharedExpressionState>,
    editor: Res<SharedEditorState>,
) {
    let (tx, rx) = crossbeam_channel::unbounded::<RigCommand>();
    commands.insert_resource(RigCommandReceiver(rx));
    commands.insert_resource(RigCommandSender(tx.clone()));

    let (screenshot_tx, screenshot_rx) = crossbeam_channel::unbounded::<ScreenshotRequest>();
    commands.insert_resource(ScreenshotRequestReceiver(screenshot_rx));

    let state = ApiState {
        poses: poses.0.clone(),
        animations: animations.0.clone(),
        live_state: live_state.clone(),
        variant: variant.clone(),
        expressions: expressions.clone(),
        commands: tx,
        screenshots: screenshot_tx,
        editor: editor.clone(),
    };

    std::thread::Builder::new()
        .name("paperdoll-http".into())
        .spawn(move || {
            let runtime = tokio::runtime::Runtime::new().expect("failed to start HTTP runtime");
            runtime.block_on(async move {
                let app = Router::new()
                    .route("/capabilities", get(get_capabilities))
                    .route("/state", get(get_state))
                    .route("/variant", get(get_variant).post(post_variant))
                    .route("/expressions", get(get_expressions).post(post_expressions))
                    .route("/pose", post(post_pose))
                    .route("/animation", post(post_animation))
                    .route("/poses", get(get_poses).post(post_register_pose))
                    .route("/animations", get(get_animations).post(post_register_animation))
                    .route("/import/vrma", post(post_import_vrma))
                    .route("/import/demo-motions", post(post_import_demo_motions))
                    .route("/screenshot", get(get_screenshot))
                    .with_state(state);
                let listener = tokio::net::TcpListener::bind(HTTP_ADDR)
                    .await
                    .unwrap_or_else(|e| panic!("failed to bind HTTP API on {HTTP_ADDR}: {e}"));
                info!(
                    "pose HTTP API listening on http://{HTTP_ADDR} \
                     (poses from '{POSES_DIR}', animations from '{ANIMATIONS_DIR}'; \
                     see GET /capabilities)"
                );
                axum::serve(listener, app)
                    .await
                    .expect("HTTP server crashed");
            });
        })
        .expect("failed to spawn HTTP server thread");
}
