//! Renders the paper-doll skeleton as a procedural placeholder humanoid (capsules per
//! bone, spheres per joint). Loads `assets/poses/*.yaml` and `assets/animations/*.yaml`
//! once here (before `App::run`, so both `spawn_rig` and `http_api::start_http_server`
//! share the exact same `PoseLibrary`/`AnimationLibrary` instances rather than each
//! loading its own stale copy). The doll opens in the default `idle` pose; an HTTP API
//! (`http_api.rs`) lets an external caller — a script, or an AI agent — trigger any
//! pose/animation at runtime, or register new ones into the same live library, via a
//! channel (for playback commands) or direct writes (for registration) into the shared
//! library. `GET /capabilities` documents the whole API for a caller that's never seen
//! it before.

mod doll_mesh;
mod http_api;
mod live_state;
mod rig_bridge;
mod screenshot_bridge;

use bevy::prelude::*;
use live_state::LiveState;
use paperdoll_rig::{load_animations_from_dir, load_poses_from_dir, DEFAULT_CAMERA};
use rig_bridge::{
    AnimationLibrary, ChoreographyCameraEntity, PoseLibrary, ANIMATIONS_DIR, POSES_DIR,
};
use std::env;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

fn main() {
    let root = resolve_asset_root();
    if let Err(e) = env::set_current_dir(&root) {
        panic!(
            "failed to cd into paperdoll asset root '{}': {e}",
            root.display()
        );
    }

    let poses = load_poses_from_dir(Path::new(POSES_DIR))
        .unwrap_or_else(|e| panic!("failed to load poses from '{POSES_DIR}': {e}"));
    let animations = load_animations_from_dir(Path::new(ANIMATIONS_DIR), &poses)
        .unwrap_or_else(|e| panic!("failed to load animations from '{ANIMATIONS_DIR}': {e}"));
    let live_state = LiveState::new();

    App::new()
        .insert_resource(PoseLibrary(Arc::new(RwLock::new(poses))))
        .insert_resource(AnimationLibrary(Arc::new(RwLock::new(animations))))
        .insert_resource(live_state)
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "Paper Doll".into(),
                ..default()
            }),
            ..default()
        }))
        .add_systems(
            Startup,
            (
                rig_bridge::spawn_rig,
                http_api::start_http_server,
                spawn_camera_and_light,
                spawn_ground,
            ),
        )
        .add_systems(
            Update,
            (
                rig_bridge::apply_rig_commands,
                rig_bridge::auto_revert_to_idle_pose,
                rig_bridge::advance_playback,
            )
                .chain(),
        )
        .add_systems(Update, screenshot_bridge::handle_screenshot_requests)
        .run();
}

/// Directory that contains `assets/poses` + `assets/animations`.
///
/// Lookup order:
/// 1. `PAPERDOLL_ROOT` if set
/// 2. current working directory (dev: `cargo run` from the repo root)
/// 3. `../share/paperdoll` next to the executable (Makefile `install` layout)
fn resolve_asset_root() -> PathBuf {
    if let Ok(root) = env::var("PAPERDOLL_ROOT") {
        let root = PathBuf::from(root);
        if root.join(POSES_DIR).is_dir() {
            return root;
        }
        panic!(
            "PAPERDOLL_ROOT='{}' does not contain '{POSES_DIR}'",
            root.display()
        );
    }

    let cwd = env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    if cwd.join(POSES_DIR).is_dir() {
        return cwd;
    }

    if let Ok(exe) = env::current_exe() {
        if let Some(bin_dir) = exe.parent() {
            let share = bin_dir.join("../share/paperdoll");
            if share.join(POSES_DIR).is_dir() {
                return share
                    .canonicalize()
                    .unwrap_or_else(|_| share);
            }
        }
    }

    panic!(
        "could not find '{POSES_DIR}'. Run from the repo root, set PAPERDOLL_ROOT, \
         or install with `make install`."
    );
}

fn spawn_camera_and_light(mut commands: Commands) {
    let eye = DEFAULT_CAMERA.eye();
    let look_at = DEFAULT_CAMERA.look_at_vec();
    let camera = commands
        .spawn((
            Camera3d::default(),
            Transform::from_translation(Vec3::new(eye.x, eye.y, eye.z)).looking_at(
                Vec3::new(look_at.x, look_at.y, look_at.z),
                Vec3::Y,
            ),
        ))
        .id();
    commands.insert_resource(ChoreographyCameraEntity(camera));

    commands.spawn((
        DirectionalLight {
            illuminance: 6000.0,
            shadows_enabled: true,
            ..default()
        },
        Transform::from_xyz(4.0, 8.0, 4.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
}

fn spawn_ground(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.spawn((
        Mesh3d(meshes.add(Plane3d::default().mesh().size(5.0, 5.0))),
        MeshMaterial3d(materials.add(Color::srgb(0.2, 0.25, 0.2))),
        Transform::IDENTITY,
    ));
}
