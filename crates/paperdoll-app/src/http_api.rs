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

use crate::live_state::{LiveState, LiveStateSnapshot};
use crate::rig_bridge::{
    AnimationLibrary, PoseLibrary, RigCommand, RigCommandReceiver, ANIMATIONS_DIR, POSES_DIR,
};
use crate::screenshot_bridge::{ScreenshotRequest, ScreenshotRequestReceiver};
use axum::{
    body::Body,
    extract::State,
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use bevy::prelude::*;
use paperdoll_rig::{resolve_animation, Animation, AnimationFile, Easing, Pose, Skeleton};
use std::collections::HashMap;
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
    commands: crossbeam_channel::Sender<RigCommand>,
    screenshots: crossbeam_channel::Sender<ScreenshotRequest>,
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

fn error_body(message: impl Into<String>) -> Json<serde_json::Value> {
    Json(serde_json::json!({ "error": message.into() }))
}

async fn post_pose(
    State(state): State<ApiState>,
    Json(req): Json<PoseCommandRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
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
    let resolved = {
        let poses = state.poses.read().unwrap();
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

/// Curated pose-board names surfaced in `GET /capabilities` `example_poses`. Each
/// entry must exist in the live library (shipped YAML or runtime-registered); missing
/// names are skipped so a caller never sees stale joint values that don't match
/// `GET /poses` / `POST /pose`.
const EXAMPLE_POSE_BOARD: &[(&str, &str)] = &[
    (
        "wave",
        "right-arm raise: from T-pose, right_shoulder.z is NEGATIVE to lift overhead \
(positive z lowers the right arm toward the hip). Elbow z bends the forearm up \
for a cartoon wave; left arm lowered with left_shoulder.z negative.",
    ),
    (
        "hands_on_hips",
        "mirrored-sign convention: equal magnitude, opposite sign — \
right_shoulder.z=+45 with left_shoulder.z=-45 puts a hand on each hip.",
    ),
    (
        "idle",
        "asymmetric full-body pose (arm + knee + pelvis) — also the IdleRevert default \
after inactivity.",
    ),
    (
        "victory",
        "both arms raised: right_shoulder.z negative, left_shoulder.z positive \
(mirrored raise from T-pose).",
    ),
    (
        "think",
        "shoulder y (forward/back) composed with z (raise) plus a deep elbow bend \
to bring the hand near the head.",
    ),
    (
        "shrug",
        "clavicle + mirrored shoulder/elbow rotations for a shoulders-up shrug.",
    ),
    (
        "point",
        "arm forward toward camera (shoulder y) with a frontal camera frame — \
pair with point_hero animation for a push-in.",
    ),
];

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

    let example_poses: Vec<serde_json::Value> = {
        let poses = state.poses.read().unwrap();
        EXAMPLE_POSE_BOARD
            .iter()
            .filter_map(|(name, demonstrates)| {
                let pose = poses.get(*name)?;
                Some(serde_json::json!({
                    "name": pose.name,
                    "demonstrates": demonstrates,
                    "description": pose.description,
                    "joints": pose.joints,
                }))
            })
            .collect()
    };

    Json(serde_json::json!({
        "name": "paperdoll pose API",
        "description": "HTTP API for posing and animating a procedural 3D paper-doll \
            humanoid rig running in paperdoll-app. Trigger a loaded pose or animation \
            to make the rig move immediately; register new poses/animations at \
            runtime to extend what's available, no server restart required.",
        "skeleton": {
            "joint_count": joint_names.len(),
            "joint_names": joint_names,
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
                    (see `victory`). A right-arm wave uses right_shoulder.z around -55.",
                "y_rotation_deg": "swings the limb forward/backward, toward or away \
                    from the camera — real, but subtle from a front-ish camera angle. \
                    Compose with z when you need a hand in front of the torso/face \
                    (see `think`).",
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
            "face_chain": "Face joints live on the +Z (front) surface of the head. \
                jaw: rotation X opens the mouth (0=closed, ~45=fully open) and reveals \
                the dark mouth interior behind the lip. *_eyelid: rotation X swings the \
                lid down over the eye (0=natural resting lid, ~85=fully closed, negative \
                = pulled back wide-eyed) — use ~85 for blinks. *_eyebrow: rotation Z, \
                MIRRORED like arms — left_eyebrow +Z furrows the inner brow down (angry), \
                -Z raises it (happy/surprised); right_eyebrow is the opposite sign. \
                *_pupil: use `translation` (not rotation) to shift the gaze — rest is \
                [0, 0, 0.012], move both pupils the same small delta (±0.008) to look \
                around. *_blush: hidden inside the head at rest; set `translation` \
                [±0.075, -0.025, 0.07] to pop the blush oval out onto the cheek. \
                Expression poses in the library: happy, angry, sad, surprised_face, \
                excited, sleepy, wink, dreamy, tsundere (face-only — in animations, \
                reference them with `hold: true` to overlay the face without resetting \
                the body).",
            "mirrored_sign_convention": "left_* and right_* joints are mirror images \
                (opposite sign on the rest offset's X component), so the SAME visual \
                pose on both sides needs OPPOSITE-signed rotation_deg.z — e.g. \
                `hands_on_hips` uses right_shoulder.z=45 and left_shoulder.z=-45 to \
                put a hand on each hip symmetrically. When adapting a one-sided pose \
                to the other side, negate z (and whichever other axis you're using).",
            "sparse_poses": "A pose only lists joints it changes; omitted joints return \
                to the skeleton rest transform when blending into that pose. An empty \
                joints map (`t_pose`) is a full reset to rest. Always verify with \
                GET /screenshot after POST /pose.",
            "how_to_verify_a_new_pose": "POST it to /poses, POST its name to /pose, \
                then GET /screenshot — don't assume an angle looks right without \
                checking, since the axis conventions above are about direction, not \
                magnitude, and every body is a different scale.",
            "worked_examples": "See example_poses below — each is a live pose from \
                the library (same joints POST /pose will play), chosen to demonstrate \
                one rule above concretely.",
        },
        "example_poses": example_poses,
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
                "<joint_name>": {
                    "rotation_deg": {
                        "x": "number, optional, default 0",
                        "y": "number, optional, default 0",
                        "z": "number, optional, default 0",
                    },
                    "translation": "[x, y, z] number array, optional — replaces the \
                        joint's rest translation entirely rather than offsetting it",
                },
            },
            "camera": "object, optional — see `camera_shape` above",
            "note": "Every field under `joints` is sparse: a joint not listed keeps \
                the skeleton's rest transform, and within a listed joint an omitted \
                `rotation_deg`/`translation` keeps that specific part of the rest \
                transform. Joint names must be one of `skeleton.joint_names` above.",
        },
        "animation_shape": {
            "name": "string, required",
            "description": "string, optional",
            "loop": "boolean, optional, default false — authoring hint for sequences \
                that could repeat; POST /animation always plays one cycle then returns \
                to the default idle pose (YAML loop is ignored at trigger time)",
            "keyframes": [
                {
                    "pose": "string, optional — a name from GET /poses, resolved at \
                        registration time",
                    "joints": "object, optional — inline joint targets, same shape as \
                        a pose's `joints` field",
                    "camera": "object, optional — see `camera_shape`; overlays the \
                        referenced pose's camera when both are set",
                    "duration_ms": "integer, required — time to blend into this \
                        keyframe from wherever the previous one (or the entry \
                        transition, for keyframe 0) left off",
                    "easing": "one of `easing_options` above, default \"linear\"",
                    "hold": "boolean, optional, default false — true makes this keyframe \
                        a sparse OVERLAY: only its listed joints move; every unlisted \
                        joint keeps its current value instead of resetting to rest. \
                        Use for blinks, gaze shifts, blush pop-outs, or layering a \
                        face-only expression pose onto whatever the body is doing.",
                    "note": "Each keyframe needs `pose`, `joints`, and/or `camera` \
                        (and must not set both `pose` and `joints`). Camera-only \
                        keyframes hold the body.",
                },
            ],
        },
        "endpoints": [
            {
                "method": "GET",
                "path": "/capabilities",
                "description": "This document — includes `posing_guide` (how \
                    rotation axes actually behave per joint chain, since it isn't \
                    guessable from joint names alone), `camera_shape` (orbit \
                    pan/tilt/yaw/zoom), and `example_poses` (live from the pose \
                    library).",
            },
            {
                "method": "GET",
                "path": "/state",
                "description": "Live snapshot: playback mode, current joint \
                    rotations in degrees (sparse — only joints present in the held \
                    blend), and the current orbit camera. Use with GET /screenshot \
                    to verify poses numerically and visually.",
                "response_body": "{ playback: { mode, animation?, keyframe_index? }, \
                    joints: { <name>: { rotation_deg: {x,y,z} } }, camera: camera_shape }",
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
        ],
    }))
}

/// Startup system: clones the shared `PoseLibrary`/`AnimationLibrary` (the exact
/// instances `main()` inserted and `spawn_rig` also reads — see its doc comment),
/// wires up the command and screenshot-request channels, and spawns the server thread.
pub fn start_http_server(
    mut commands: Commands,
    poses: Res<PoseLibrary>,
    animations: Res<AnimationLibrary>,
    live_state: Res<LiveState>,
) {
    let (tx, rx) = crossbeam_channel::unbounded::<RigCommand>();
    commands.insert_resource(RigCommandReceiver(rx));

    let (screenshot_tx, screenshot_rx) = crossbeam_channel::unbounded::<ScreenshotRequest>();
    commands.insert_resource(ScreenshotRequestReceiver(screenshot_rx));

    let state = ApiState {
        poses: poses.0.clone(),
        animations: animations.0.clone(),
        live_state: live_state.clone(),
        commands: tx,
        screenshots: screenshot_tx,
    };

    std::thread::Builder::new()
        .name("paperdoll-http".into())
        .spawn(move || {
            let runtime = tokio::runtime::Runtime::new().expect("failed to start HTTP runtime");
            runtime.block_on(async move {
                let app = Router::new()
                    .route("/capabilities", get(get_capabilities))
                    .route("/state", get(get_state))
                    .route("/pose", post(post_pose))
                    .route("/animation", post(post_animation))
                    .route("/poses", get(get_poses).post(post_register_pose))
                    .route("/animations", get(get_animations).post(post_register_animation))
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
