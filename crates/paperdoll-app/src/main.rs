//! Renders the paper-doll skeleton. **v2** (default) is a VRM 1.0 skinned mesh;
//! **v1** is the procedural placeholder humanoid (capsules per bone). Select with
//! `--variant` / `PAPERDOLL_VARIANT` or `POST /variant` at runtime.
//!
//! Loads `assets/poses/*.yaml` and `assets/animations/*.yaml` once here (before
//! `App::run`, so both the visual spawn path and `http_api::start_http_server`
//! share the exact same `PoseLibrary`/`AnimationLibrary` instances). The doll
//! opens in the default `idle` pose; an HTTP API lets an external caller trigger
//! poses/animations or register new ones. `GET /capabilities` documents the API.

mod doll_mesh;
mod http_api;
mod live_state;
mod rig_bridge;
mod screenshot_bridge;
mod v2_expressions;
mod v2_vrm;
mod variant;

use bevy::prelude::*;
use bevy_vrm1::prelude::VrmPlugin;
use live_state::LiveState;
use paperdoll_rig::{load_animations_from_dir, load_poses_from_dir, DEFAULT_CAMERA};
use rig_bridge::{
    ActiveVariant, AnimationLibrary, ChoreographyCameraEntity, PoseLibrary, ANIMATIONS_DIR,
    POSES_DIR,
};
use std::env;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use v2_expressions::{SharedExpressionState, V2ExpressionBindings};
use variant::{LaunchConfig, SharedVariantState};

fn main() {
    let launch = LaunchConfig::from_env_and_args().unwrap_or_else(|e| {
        eprintln!("paperdoll: {e}");
        eprintln!("Try `paperdoll --help`.");
        std::process::exit(2);
    });

    let root = resolve_asset_root();
    if let Err(e) = env::set_current_dir(&root) {
        panic!(
            "failed to cd into paperdoll asset root '{}': {e}",
            root.display()
        );
    }

    if let Err(e) = launch.ensure_assets_for_launch() {
        eprintln!("paperdoll: {e}");
        std::process::exit(2);
    }

    let poses = load_poses_from_dir(Path::new(POSES_DIR))
        .unwrap_or_else(|e| panic!("failed to load poses from '{POSES_DIR}': {e}"));
    let animations = load_animations_from_dir(Path::new(ANIMATIONS_DIR), &poses)
        .unwrap_or_else(|e| panic!("failed to load animations from '{ANIMATIONS_DIR}': {e}"));
    let live_state = LiveState::new();
    let shared_variant = SharedVariantState::new(&launch);
    let shared_expressions = SharedExpressionState::default();

    info!(
        "paperdoll starting with variant {} (v2 character asset: {})",
        launch.variant, launch.v2_character
    );

    let assets_path = root.join("assets");

    App::new()
        .insert_resource(PoseLibrary(Arc::new(RwLock::new(poses))))
        .insert_resource(AnimationLibrary(Arc::new(RwLock::new(animations))))
        .insert_resource(live_state)
        .insert_resource(ActiveVariant(launch.variant))
        .insert_resource(shared_variant)
        .insert_resource(shared_expressions)
        .insert_resource(V2ExpressionBindings::default())
        .add_plugins(
            DefaultPlugins
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        title: format!("Paper Doll ({})", launch.variant),
                        ..default()
                    }),
                    ..default()
                })
                .set(AssetPlugin {
                    // Cargo sets the default asset root to the crate dir; we keep
                    // poses/characters under the repo (or install) asset root instead.
                    file_path: assets_path.to_string_lossy().into_owned(),
                    ..default()
                }),
        )
        .add_plugins(VrmPlugin)
        .add_systems(
            Startup,
            (
                rig_bridge::setup_rig_core,
                rig_bridge::spawn_initial_visual,
                http_api::start_http_server,
                spawn_camera_and_light,
                spawn_ground,
            )
                .chain(),
        )
        .add_systems(
            Update,
            (
                // Bind VRM bones before writing poses so startup idle lands on frame 1
                // after bind (v2 loads asynchronously into T-pose otherwise).
                v2_vrm::bind_v2_rig_entities,
                v2_expressions::bind_v2_expressions,
                v2_expressions::apply_v2_expressions,
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
                return share.canonicalize().unwrap_or_else(|_| share);
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
