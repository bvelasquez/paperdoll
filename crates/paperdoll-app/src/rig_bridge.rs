use crate::doll_mesh::{
    ahoge_mesh, blush_mesh, bone_segment_mesh, bust_mesh, eyelid_mesh, eyeliner_mesh,
    eyebrow_mesh, hair_cap_mesh, hair_tail_mesh, highlight_mesh, iris_mesh, joint_marker_mesh,
    lash_mesh, lip_mesh, mouth_interior_mesh, pupil_mesh, sclera_mesh,
};
use crate::v2_vrm;
use crate::variant::{DollVariant, SharedVariantState};
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy_vrm1::prelude::RestTransform;
use paperdoll_rig::{
    duration_ms_for_speed, Animation, CameraTarget, JointId, PlaybackState, PlaybackTarget, Pose,
    Skeleton,
};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

/// How high off the ground the root (pelvis) joint sits: hip_drop (0.04) + thigh
/// (0.44) + shin (0.42) + ankle_height (0.06) = 0.96, so the ANKLE joint sits at
/// anatomical ankle height (0.06) and the foot (toe segment + heel) touches y=0.
const ROOT_HEIGHT: f32 = 0.96;

/// Where pose YAML files live, relative to the process's working directory (the
/// workspace root when run via `cargo run -p paperdoll-app`). `pub(crate)` so
/// `http_api.rs` can load the same directory for its `GET /poses` listing.
pub(crate) const POSES_DIR: &str = "assets/poses";

/// Where animation YAML files live. See `POSES_DIR`.
pub const ANIMATIONS_DIR: &str = "assets/animations";

/// Pose applied at launch. The skeleton's rest is a T-pose (arms out); we bake this
/// default in at spawn so the window never flashes arms-out before settling.
const STARTUP_POSE: &str = "idle";

/// Joint names that are face features, not structural bones. Bone segments are
/// skipped for these (they'd read as tiny lines on the face), and cosmetic meshes
/// (eyes, eyebrows, mouth) are spawned instead of the default joint-marker sphere.
fn is_face_joint(name: &str) -> bool {
    matches!(
        name,
        "jaw"
            | "left_eye"
            | "right_eye"
            | "left_pupil"
            | "right_pupil"
            | "left_eyelid"
            | "right_eyelid"
            | "left_eyebrow"
            | "right_eyebrow"
            | "left_blush"
            | "right_blush"
    )
}

/// Finger bones exist for v2/API parity; on the procedural doll they're too small to
/// read, so we suppress bone segments and markers (same idea as face cosmetics).
fn is_finger_joint(name: &str) -> bool {
    name.contains("_thumb_")
        || name.contains("_index_")
        || name.contains("_middle_")
        || name.contains("_ring_")
        || name.contains("_little_")
}

/// Joints whose INCOMING bone segment (parent → joint) is suppressed: face
/// joints (tiny lines on the face), fingers, and hips — the short, fat pelvis→hip stubs
/// cross at center-front and poke through the hip-block cylinder as an ugly X.
/// The thigh (hip→knee) segment belongs to the knee, so legs are unaffected.
fn skip_bone(name: &str) -> bool {
    is_face_joint(name) || is_finger_joint(name) || matches!(name, "left_hip" | "right_hip")
}

/// Joints whose default marker sphere is suppressed even though they keep their
/// bone segment: the hip markers bulge out of the pelvis egg and their two
/// intersection curves cross mid-front into an ugly "X" crease — the tapered
/// thigh segment alone meets the pelvis cleanly.
fn skip_marker(name: &str) -> bool {
    is_face_joint(name) || is_finger_joint(name) || matches!(name, "left_hip" | "right_hip")
}

/// Uniform soft-scale for specific joint markers: full-size markers read as
/// balls-on-sticks at high-detail spots (knees, elbows, the waist ring), so these
/// shrink a touch and let the tapered bone segments carry the silhouette.
fn marker_soft_scale(name: &str) -> f32 {
    match name {
        "spine" => 0.8,
        "left_knee" | "right_knee" => 0.82,
        "left_elbow" | "right_elbow" => 0.85,
        "left_ankle" | "right_ankle" => 0.88,
        "left_wrist" | "right_wrist" => 0.9,
        "upper_chest" => 0.92,
        _ => 1.0,
    }
}

/// Per-bone (start_radius, end_radius) for the tapered segment ending at `child_name`,
/// derived from the two joints' own radii with a few anatomical overrides: thighs
/// taper hard to the knee, calves bump just below the knee then taper to the ankle,
/// and the neck stays near-straight instead of flaring into the skull.
fn bone_radii(child_name: &str, parent_radius: f32, joint_radius: f32) -> (f32, f32) {
    match child_name {
        // Thigh (hip→knee): wide at the hip, noticeably narrower at the knee
        "left_knee" | "right_knee" => (parent_radius, joint_radius * 0.88),
        // Calf (knee→ankle): muscle bump just under the knee, slim ankle
        "left_ankle" | "right_ankle" => (parent_radius * 1.15, joint_radius * 0.9),
        // Neck (neck→head): flare enough to tuck under the jaw so no shadow gap
        // shows between the neck segment and the head egg
        "head" => (parent_radius, parent_radius * 1.65),
        _ => (parent_radius, joint_radius),
    }
}

/// Maps rig `JointId`s to the Bevy entity carrying that joint's `Transform`. Other
/// systems look entities up here to apply interpolated rotations by joint name/id
/// without re-walking the hierarchy every frame.
#[derive(Resource, Default)]
pub struct RigEntities(pub HashMap<JointId, Entity>);

/// Root of the active visual tree (v1 pelvis hierarchy or v2 VRM). Despawned on
/// variant switch.
#[derive(Component)]
pub struct DollVisualRoot;

/// How [`advance_playback`] writes paperdoll quaternions onto bound entities.
#[derive(Resource, Clone, Copy, PartialEq, Eq, Default)]
pub enum PoseApplyMode {
    /// v1: paperdoll rest is identity — write absolute local rotations.
    #[default]
    Absolute,
    /// v2: VRM bones keep authored rest — write `rest * paperdoll`.
    RestRelative,
}

/// Currently displayed visual variant.
#[derive(Resource, Clone, Copy, PartialEq, Eq)]
pub struct ActiveVariant(pub DollVariant);

/// The rig's rest-pose skeleton, kept as a resource so systems can resolve pose YAML
/// (by joint name) without rebuilding it every frame.
#[derive(Resource)]
pub struct RigSkeleton(pub Skeleton);

/// Poses keyed by name: everything loaded from `assets/poses/*.yaml` at startup, plus
/// anything registered afterward via `POST /poses`. `Arc<RwLock<_>>` rather than a
/// plain map because the *same* map is also held directly by the HTTP thread
/// (`http_api.rs`) — registration writes it there, with no round trip through the ECS,
/// since inserting into a `HashMap` needs no access to Bevy's `World`. Only *starting
/// playback* of a pose (`apply_rig_commands`) is ECS-only state and goes through the
/// command channel instead.
#[derive(Resource, Clone)]
pub struct PoseLibrary(pub Arc<RwLock<HashMap<String, Pose>>>);

/// Animations keyed by name, with pose references already resolved against
/// [`PoseLibrary`] at the time each was loaded or registered. Shared with the HTTP
/// thread the same way `PoseLibrary` is — see its doc comment.
#[derive(Resource, Clone)]
pub struct AnimationLibrary(pub Arc<RwLock<HashMap<String, Animation>>>);

/// A command sent from the HTTP API (`http_api.rs`, running on its own OS thread)
/// into the Bevy ECS, applied once per frame by [`apply_rig_commands`]. Crossing the
/// thread boundary via a channel rather than shared state, since Bevy's `World` isn't
/// safely accessible from an async server running outside its own schedule.
#[derive(Debug, Clone)]
pub enum RigCommand {
    Pose {
        name: String,
        /// Overrides `TransitionSpeed::deg_per_sec` for just this command, so an API
        /// caller can ask for a faster/slower transition without touching the
        /// resource every other command relies on.
        speed_deg_per_sec: Option<f32>,
    },
    Animation {
        name: String,
    },
    /// Tear down the current visual and spawn the other (v1 procedural / v2 VRM).
    SetVariant {
        variant: DollVariant,
    },
    /// Set VRM expression blend weights (v2 only). Keys are preset names.
    SetExpressions {
        weights: HashMap<String, f32>,
    },
}

/// Receiving end of the HTTP API's command channel, drained by [`apply_rig_commands`].
/// `crossbeam_channel::Receiver` is `Send + Sync` (unlike `std::sync::mpsc::Receiver`),
/// which a Bevy `Resource` requires.
#[derive(Resource)]
pub struct RigCommandReceiver(pub crossbeam_channel::Receiver<RigCommand>);

/// Sending end of the command channel (HTTP API, bored autoplay, startup idle).
#[derive(Resource, Clone)]
pub struct RigCommandSender(pub crossbeam_channel::Sender<RigCommand>);

/// The rig's live interpolation state (`paperdoll_rig::PlaybackState`), advanced once
/// per frame by [`advance_playback`]. Seeded to the default idle pose in `spawn_rig`
/// and driven by the HTTP API's `POST /pose`/`POST /animation` thereafter.
#[derive(Resource, Default)]
pub struct RigPlayback(pub PlaybackState);

/// Degrees/second used to size a transition's duration from its angular pose delta
/// (`paperdoll_rig::duration_ms_for_speed`), so a small pose change transitions
/// quickly and a large one doesn't snap despite both going through the same call
/// site — plus the floor/ceiling so a near-zero delta doesn't flash and a huge one
/// doesn't stall. A resource rather than a bare constant so a future API command
/// (M5) can change how "fast" the doll moves without touching `advance_playback`.
#[derive(Resource, Clone, Copy)]
pub struct TransitionSpeed {
    pub deg_per_sec: f32,
    pub min_ms: u32,
    pub max_ms: u32,
}

impl Default for TransitionSpeed {
    fn default() -> Self {
        Self {
            deg_per_sec: 180.0,
            min_ms: 150,
            max_ms: 2000,
        }
    }
}

/// Tracks how long the rig has sat idle since the last command, so
/// [`auto_revert_to_idle_pose`] can send it back to `default_pose_name` after
/// `timeout_secs` of inactivity — the doll shouldn't stay frozen in whatever pose a
/// caller last requested indefinitely; it should settle back into a relaxed default.
#[derive(Resource)]
pub struct IdleRevert {
    pub timeout_secs: f32,
    pub default_pose_name: String,
    /// `Time::elapsed_secs()` as of the last command `apply_rig_commands` applied.
    last_activity_secs: f32,
    /// Set once the auto-revert has fired, so it doesn't re-trigger every frame
    /// while the rig sits idle at the default pose it just reverted to — cleared the
    /// next time a real command arrives.
    reverted: bool,
    /// Set when an HTTP animation starts; cleared when the animation finishes and
    /// queues [`Self::pending_after_animation`], or when a pose command supersedes it.
    expect_return_after_animation: bool,
    /// Set by [`advance_playback`] when a one-shot animation finishes, so the next
    /// [`auto_revert_to_idle_pose`] returns to the default pose immediately instead of
    /// waiting out `timeout_secs`.
    pending_after_animation: bool,
}

impl IdleRevert {
    /// Keep the rig from auto-reverting while the in-app editor is authoring.
    pub fn hold_for_editor(&mut self, now_secs: f32) {
        self.last_activity_secs = now_secs;
        self.reverted = false;
        self.expect_return_after_animation = false;
        self.pending_after_animation = false;
    }

    /// True when the rig is settled on [`Self::default_pose_name`] (safe to bored-autoplay).
    pub fn is_holding_default_pose(&self) -> bool {
        self.reverted
    }
}

impl Default for IdleRevert {
    fn default() -> Self {
        Self {
            timeout_secs: 10.0,
            default_pose_name: "idle".to_string(),
            last_activity_secs: 0.0,
            reverted: false,
            expect_return_after_animation: false,
            pending_after_animation: false,
        }
    }
}

/// Inserts shared rig resources (skeleton, playback, idle revert). Call once at
/// startup before spawning either visual variant.
pub fn setup_rig_core(mut commands: Commands, poses: Res<PoseLibrary>) {
    let skeleton = Skeleton::humanoid_default();
    let startup_pose = {
        let poses_guard = poses.0.read().unwrap();
        poses_guard
            .get(STARTUP_POSE)
            .cloned()
            .unwrap_or_else(|| panic!("startup pose '{STARTUP_POSE}' not found in '{POSES_DIR}'"))
    };
    let startup_resolved = startup_pose
        .resolve(&skeleton)
        .unwrap_or_else(|e| panic!("startup pose '{STARTUP_POSE}' failed to resolve: {e}"));
    let playback = PlaybackState::Idle {
        held: startup_resolved,
    };
    let mut idle_revert = IdleRevert::default();
    idle_revert.reverted = true;

    commands.insert_resource(RigEntities::default());
    commands.insert_resource(RigPlayback(playback));
    commands.insert_resource(TransitionSpeed::default());
    commands.insert_resource(RigSkeleton(skeleton));
    commands.insert_resource(idle_revert);
    commands.insert_resource(PoseApplyMode::Absolute);
}

/// Startup: spawn whichever visual [`ActiveVariant`] selected at launch.
pub fn spawn_initial_visual(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut rig_entities: ResMut<RigEntities>,
    mut apply_mode: ResMut<PoseApplyMode>,
    skeleton: Res<RigSkeleton>,
    poses: Res<PoseLibrary>,
    active: Res<ActiveVariant>,
    shared_variant: Res<SharedVariantState>,
    asset_server: Res<AssetServer>,
) {
    spawn_visual_for_variant(
        active.0,
        &mut commands,
        &mut meshes,
        &mut materials,
        &mut rig_entities,
        &mut apply_mode,
        &skeleton,
        &poses,
        &asset_server,
        &shared_variant.v2_character(),
    );
}

pub(crate) fn spawn_visual_for_variant(
    variant: DollVariant,
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    rig_entities: &mut RigEntities,
    apply_mode: &mut PoseApplyMode,
    skeleton: &RigSkeleton,
    poses: &PoseLibrary,
    asset_server: &AssetServer,
    v2_character: &str,
) {
    match variant {
        DollVariant::V1 => {
            spawn_v1_visual(commands, meshes, materials, rig_entities, apply_mode, skeleton, poses)
        }
        DollVariant::V2 => {
            v2_vrm::spawn_v2_visual(commands, asset_server, v2_character, apply_mode, rig_entities)
        }
    }
}

/// Spawns the v1 procedural doll. Pelvis is tagged [`DollVisualRoot`] for variant swaps.
pub fn spawn_v1_visual(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    rig_entities: &mut RigEntities,
    apply_mode: &mut PoseApplyMode,
    skeleton_res: &RigSkeleton,
    poses: &PoseLibrary,
) {
    *apply_mode = PoseApplyMode::Absolute;
    rig_entities.0.clear();

    let skeleton = &skeleton_res.0;
    let startup_pose = {
        let poses_guard = poses.0.read().unwrap();
        poses_guard
            .get(STARTUP_POSE)
            .cloned()
            .unwrap_or_else(|| panic!("startup pose '{STARTUP_POSE}' not found in '{POSES_DIR}'"))
    };
    let startup_resolved = startup_pose.resolve(skeleton).unwrap_or_else(|e| {
        panic!("startup pose '{STARTUP_POSE}' failed to resolve: {e}")
    });

    let bone_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.85, 0.72, 0.55),
        ..default()
    });
    // Joint markers use a slightly deeper warm tan than the bones — articulation
    // stays readable, but the figure reads as one continuous wooden-doll surface
    // rather than dark "armor pieces" over a tan body.
    let joint_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.78, 0.64, 0.47),
        ..default()
    });

    // `Skeleton::joints()` iterates in build order, which the builder guarantees puts
    // every parent before its children — so each parent entity already exists in
    // `rig_entities` by the time we reach a joint that references it. Entities spawn
    // already in `STARTUP_POSE` (not skeleton rest) so the first painted frame is idle.
    for (joint_id, joint) in skeleton.joints() {
        let translation = if joint.parent.is_none() {
            joint.local_translation + Vec3::new(0.0, ROOT_HEIGHT, 0.0)
        } else {
            joint.local_translation
        };
        let rotation = startup_resolved
            .joint_rotations
            .get(&joint_id)
            .copied()
            .unwrap_or(joint.local_rotation);

        let mut entity_cmds = commands.spawn((
            Transform::from_translation(translation).with_rotation(rotation),
            Visibility::default(),
            Name::new(joint.name.clone()),
        ));
        if joint.parent.is_none() {
            entity_cmds.insert(DollVisualRoot);
        }
        let entity = entity_cmds.id();

        if let Some(parent_id) = joint.parent {
            let parent_entity = rig_entities.0[&parent_id];
            commands.entity(parent_entity).add_child(entity);

            if !skip_bone(&joint.name) {
                // The bone connecting parent -> this joint is drawn under the *parent*
                // entity (not under `entity`), since it spans the segment ending at this
                // joint's local offset rather than starting from it. Tapered: wide at
                // the parent end, narrow at this joint (see `bone_radii`).
                let bone_vector = joint.local_translation;
                let bone_length = bone_vector.length();
                if bone_length > 1e-5 {
                    let parent_radius = skeleton.joint(parent_id).radius;
                    let (radius_start, radius_end) =
                        bone_radii(&joint.name, parent_radius, joint.radius);
                    let midpoint = bone_vector * 0.5;
                    let rotation = Quat::from_rotation_arc(Vec3::Y, bone_vector.normalize());
                    let bone_entity = commands
                        .spawn((
                            Mesh3d(meshes.add(bone_segment_mesh(
                                bone_length,
                                radius_start,
                                radius_end,
                            ))),
                            MeshMaterial3d(bone_material.clone()),
                            Transform::from_translation(midpoint).with_rotation(rotation),
                        ))
                        .id();
                    commands.entity(parent_entity).add_child(bone_entity);
                }
            }
        }

        if joint.name == "head" {
            // The head marker is the FACE's canvas: skin-toned (not the gray joint
            // material) and subtly egg-shaped — a touch narrower and taller than a
            // pure sphere reads as a feminine head rather than a ball.
            let head_entity = commands
                .spawn((
                    Mesh3d(meshes.add(joint_marker_mesh(joint.radius))),
                    MeshMaterial3d(bone_material.clone()),
                    Transform::from_scale(Vec3::new(0.94, 1.08, 0.98)),
                ))
                .id();
            commands.entity(entity).add_child(head_entity);
        } else if joint.name == "pelvis" {
            // The pelvis marker IS the pelvis block: one smooth egg, wide and
            // flat front-to-back. Its bottom point hangs between the thigh tops,
            // which touch at the top — so the dip hides inside the merged thighs
            // instead of creasing against them. (The old center-front X came from
            // the pelvis→hip bone stubs, now hidden via `skip_bone`.)
            let pelvis_entity = commands
                .spawn((
                    Mesh3d(meshes.add(joint_marker_mesh(joint.radius))),
                    MeshMaterial3d(bone_material.clone()),
                    Transform::from_translation(Vec3::new(0.0, -0.01, 0.005))
                        .with_scale(Vec3::new(1.02, 0.65, 0.8)),
                ))
                .id();
            commands.entity(entity).add_child(pelvis_entity);
        } else if joint.name == "left_hand" || joint.name == "right_hand" {
            // Hands: small flattened-palm ovals rather than round balls — daintier,
            // and they read as hands instead of joints when fingers aren't modeled.
            let marker_entity = commands
                .spawn((
                    Mesh3d(meshes.add(joint_marker_mesh(joint.radius))),
                    MeshMaterial3d(joint_material.clone()),
                    Transform::from_scale(Vec3::new(0.85, 1.15, 0.55)),
                ))
                .id();
            commands.entity(entity).add_child(marker_entity);
        } else if !skip_marker(&joint.name) {
            let marker_entity = commands
                .spawn((
                    Mesh3d(meshes.add(joint_marker_mesh(joint.radius))),
                    MeshMaterial3d(joint_material.clone()),
                    Transform::from_scale(Vec3::splat(marker_soft_scale(&joint.name))),
                ))
                .id();
            commands.entity(entity).add_child(marker_entity);
        }

        rig_entities.0.insert(joint_id, entity);
    }

    // ── Feminine body cosmetics ──────────────────────────────────────────────
    // Bust spheres on the chest joint + hip-flare spheres on the pelvis. Cosmetic
    // only, not part of the joint hierarchy — they move with their parent joint but
    // aren't independently posable the way a real joint would be.
    let chest_entity = rig_entities.0[&skeleton
        .joint_by_name("chest")
        .expect("skeleton has a chest joint")];
    for sign in [1.0_f32, -1.0] {
        let bust_entity = commands
            .spawn((
                Mesh3d(meshes.add(bust_mesh(0.072))),
                MeshMaterial3d(bone_material.clone()),
                // Slightly below and forward of the chest joint (which sits at the
                // top of the rib-cage segment), gently oval rather than round.
                Transform::from_translation(Vec3::new(sign * 0.055, 0.005, 0.14))
                    .with_scale(Vec3::new(1.0, 0.9, 0.85)),
            ))
            .id();
        commands.entity(chest_entity).add_child(bust_entity);
    }

    let pelvis_entity = rig_entities.0[&skeleton
        .joint_by_name("pelvis")
        .expect("skeleton has a pelvis joint")];

    // Buttocks on the BACK of the pelvis — generous round cheeks, curves where they
    // belong while the front stays flat.
    for sign in [1.0_f32, -1.0] {
        let butt_entity = commands
            .spawn((
                Mesh3d(meshes.add(bust_mesh(0.09))),
                MeshMaterial3d(bone_material.clone()),
                Transform::from_translation(Vec3::new(sign * 0.06, -0.05, -0.095))
                    .with_scale(Vec3::new(1.0, 0.9, 0.8)),
            ))
            .id();
        commands.entity(pelvis_entity).add_child(butt_entity);
    }

    // Small heel spheres at the ankles so the back of the foot plants on the floor
    // (the toe segment slopes down to meet the ground at the front).
    for (side, _sign) in [("left", 1.0_f32), ("right", -1.0_f32)] {
        let ankle_entity = rig_entities.0[&skeleton
            .joint_by_name(&format!("{side}_ankle"))
            .expect("ankle joint exists")];
        let heel = commands
            .spawn((
                Mesh3d(meshes.add(bust_mesh(0.032))),
                MeshMaterial3d(bone_material.clone()),
                Transform::from_translation(Vec3::new(0.0, -0.025, -0.025)),
            ))
            .id();
        commands.entity(ankle_entity).add_child(heel);
    }

    // ── Face features ────────────────────────────────────────────────────────
    // Anime-style feminine face: big glossy eyes (sclera + iris + pupil + sparkle
    // highlight), resting upper eyelids that can blink, eyeliner + lash flicks,
    // thin soft brows, pop-out blush ovals, and a small lip that rides the jaw.
    // Slight emissive on the eye parts keeps them readable in any lighting.

    let sclera_material = materials.add(StandardMaterial {
        base_color: Color::srgb(1.0, 1.0, 1.0),
        emissive: LinearRgba::new(0.30, 0.30, 0.30, 1.0),
        ..default()
    });
    let iris_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.48, 0.30, 0.14),
        emissive: LinearRgba::new(0.12, 0.06, 0.02, 1.0),
        ..default()
    });
    let pupil_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.06, 0.04, 0.06),
        ..default()
    });
    let highlight_material = materials.add(StandardMaterial {
        base_color: Color::srgb(1.0, 1.0, 1.0),
        emissive: LinearRgba::new(0.9, 0.9, 0.9, 1.0),
        ..default()
    });
    let eyelid_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.87, 0.74, 0.58),
        ..default()
    });
    let eyeliner_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.14, 0.09, 0.09),
        ..default()
    });
    let eyebrow_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.30, 0.18, 0.10),
        ..default()
    });
    let blush_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.95, 0.55, 0.58),
        emissive: LinearRgba::new(0.18, 0.05, 0.05, 1.0),
        ..default()
    });
    let lip_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.87, 0.38, 0.40),
        emissive: LinearRgba::new(0.10, 0.02, 0.02, 1.0),
        ..default()
    });
    let mouth_interior_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.28, 0.09, 0.10),
        ..default()
    });
    let hair_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.23, 0.13, 0.09),
        ..default()
    });

    // Eye assembly, mirrored per side. The `*_pupil` joint carries iris + pupil +
    // highlights as one unit so translating it shifts the whole gaze; the
    // `*_eyelid` joint carries the lid cap so rotating X swings it over the eye.
    for (side, sign) in [("left", 1.0_f32), ("right", -1.0_f32)] {
        let eye_entity = rig_entities.0[&skeleton
            .joint_by_name(&format!("{side}_eye"))
            .expect("eye joint exists")];

        // Sclera — modest-sized, gently flattened white ellipse
        let sclera = commands
            .spawn((
                Mesh3d(meshes.add(sclera_mesh())),
                MeshMaterial3d(sclera_material.clone()),
                Transform::from_translation(Vec3::new(0.0, 0.0, 0.026))
                    .with_scale(Vec3::new(1.0, 1.15, 0.35)),
            ))
            .id();
        commands.entity(eye_entity).add_child(sclera);

        // Upper eyeliner — thin dark capsule lying horizontally along the top of the eye
        let liner = commands
            .spawn((
                Mesh3d(meshes.add(eyeliner_mesh())),
                MeshMaterial3d(eyeliner_material.clone()),
                Transform::from_translation(Vec3::new(0.0, 0.034, 0.033))
                    .with_rotation(Quat::from_rotation_z(std::f32::consts::FRAC_PI_2)),
            ))
            .id();
        commands.entity(eye_entity).add_child(liner);

        // Lash flicks at the outer corner — two short angled capsules
        for (y, tilt_deg) in [(0.022_f32, 55.0_f32), (0.014, 80.0)] {
            let lash = commands
                .spawn((
                    Mesh3d(meshes.add(lash_mesh())),
                    MeshMaterial3d(eyeliner_material.clone()),
                    Transform::from_translation(Vec3::new(sign * 0.032, y, 0.030))
                        .with_rotation(Quat::from_rotation_z(sign * tilt_deg.to_radians())),
                ))
                .id();
            commands.entity(eye_entity).add_child(lash);
        }

        // Gaze unit (iris + pupil + highlights) rides the pupil joint
        let pupil_entity = rig_entities.0[&skeleton
            .joint_by_name(&format!("{side}_pupil"))
            .expect("pupil joint exists")];
        let iris = commands
            .spawn((
                Mesh3d(meshes.add(iris_mesh())),
                MeshMaterial3d(iris_material.clone()),
                Transform::from_translation(Vec3::new(0.0, 0.0, 0.020))
                    .with_scale(Vec3::new(1.0, 1.12, 0.4)),
            ))
            .id();
        commands.entity(pupil_entity).add_child(iris);
        let pupil = commands
            .spawn((
                Mesh3d(meshes.add(pupil_mesh())),
                MeshMaterial3d(pupil_material.clone()),
                Transform::from_translation(Vec3::new(0.0, 0.0, 0.026))
                    .with_scale(Vec3::new(1.0, 1.1, 0.5)),
            ))
            .id();
        commands.entity(pupil_entity).add_child(pupil);
        // Sparkle highlights — one bright upper-inner dot, one tiny lower-outer dot
        for (offset, scale) in [
            (Vec3::new(sign * 0.009, 0.010, 0.030), 1.0_f32),
            (Vec3::new(-sign * 0.006, -0.007, 0.031), 0.6),
        ] {
            let sparkle = commands
                .spawn((
                    Mesh3d(meshes.add(highlight_mesh())),
                    MeshMaterial3d(highlight_material.clone()),
                    Transform::from_translation(offset).with_scale(Vec3::splat(scale)),
                ))
                .id();
            commands.entity(pupil_entity).add_child(sparkle);
        }

        // Eyelid — soft skin-toned cap, tucked above the eye at rest; the eyelid
        // joint rotates around X to swing it down over the eye for blinks/winks.
        let eyelid_entity = rig_entities.0[&skeleton
            .joint_by_name(&format!("{side}_eyelid"))
            .expect("eyelid joint exists")];
        let eyelid = commands
            .spawn((
                Mesh3d(meshes.add(eyelid_mesh())),
                MeshMaterial3d(eyelid_material.clone()),
                Transform::from_translation(Vec3::new(0.0, 0.034, 0.014))
                    .with_scale(Vec3::new(1.05, 0.85, 0.4)),
            ))
            .id();
        commands.entity(eyelid_entity).add_child(eyelid);

        // Eyebrow — thin soft capsule, gentle natural slant (outer end slightly down)
        let eyebrow_entity = rig_entities.0[&skeleton
            .joint_by_name(&format!("{side}_eyebrow"))
            .expect("eyebrow joint exists")];
        let eyebrow = commands
            .spawn((
                Mesh3d(meshes.add(eyebrow_mesh())),
                MeshMaterial3d(eyebrow_material.clone()),
                Transform::from_rotation(
                    Quat::from_rotation_z(std::f32::consts::FRAC_PI_2)
                        * Quat::from_rotation_z(-sign * 0.1),
                ),
            ))
            .id();
        commands.entity(eyebrow_entity).add_child(eyebrow);

        // Blush oval — tucked inside the head at rest (blush joint z=0.04 is below
        // the cheek surface ≈0.067); an expression pose translates the joint out
        // to z≈0.07 to pop it onto the cheek.
        let blush_entity = rig_entities.0[&skeleton
            .joint_by_name(&format!("{side}_blush"))
            .expect("blush joint exists")];
        let blush = commands
            .spawn((
                Mesh3d(meshes.add(blush_mesh())),
                MeshMaterial3d(blush_material.clone()),
                Transform::from_scale(Vec3::new(1.2, 0.65, 0.25)),
            ))
            .id();
        commands.entity(blush_entity).add_child(blush);
    }

    // Mouth: a small coral lip riding the jaw joint, plus a dark interior patch
    // fixed to the head just behind the lip — the jaw swings the lip down to reveal
    // it, reading as an open mouth.
    let jaw_entity = rig_entities.0[&skeleton.joint_by_name("jaw").expect("jaw joint exists")];
    let lip = commands
        .spawn((
            Mesh3d(meshes.add(lip_mesh())),
            MeshMaterial3d(lip_material.clone()),
            Transform::from_translation(Vec3::new(0.0, 0.0, 0.015))
                .with_scale(Vec3::new(1.5, 0.55, 0.5)),
        ))
        .id();
    commands.entity(jaw_entity).add_child(lip);

    let head_entity = rig_entities.0[&skeleton.joint_by_name("head").expect("head joint exists")];
    let interior = commands
        .spawn((
            Mesh3d(meshes.add(mouth_interior_mesh())),
            MeshMaterial3d(mouth_interior_material.clone()),
            Transform::from_translation(Vec3::new(0.0, -0.055, 0.101))
                .with_scale(Vec3::new(1.3, 0.7, 0.3)),
        ))
        .id();
    commands.entity(head_entity).add_child(interior);

    // Hair — a dark cap framing the face (covers the back/top of the head, open at
    // the front), hanging twin-tails, and a small ahoge cowlick on top.
    let hair_cap = commands
        .spawn((
            Mesh3d(meshes.add(hair_cap_mesh())),
            MeshMaterial3d(hair_material.clone()),
            Transform::from_translation(Vec3::new(0.0, 0.028, -0.028))
                .with_scale(Vec3::new(1.0, 1.0, 1.05)),
        ))
        .id();
    commands.entity(head_entity).add_child(hair_cap);
    for sign in [1.0_f32, -1.0] {
        // Twin-tails hang beside the jaw/neck, not at eye level (where they read as ears)
        let tail = commands
            .spawn((
                Mesh3d(meshes.add(hair_tail_mesh())),
                MeshMaterial3d(hair_material.clone()),
                Transform::from_translation(Vec3::new(sign * 0.125, -0.07, -0.05))
                    .with_scale(Vec3::new(0.85, 1.4, 0.85)),
            ))
            .id();
        commands.entity(head_entity).add_child(tail);
    }
    let ahoge = commands
        .spawn((
            Mesh3d(meshes.add(ahoge_mesh())),
            MeshMaterial3d(hair_material.clone()),
            Transform::from_translation(Vec3::new(0.0, 0.115, -0.01))
                .with_rotation(Quat::from_rotation_x(-0.35)),
        ))
        .id();
    commands.entity(head_entity).add_child(ahoge);

    let _ = startup_resolved; // baked into joint transforms above
}

/// Params needed to tear down / respawn visuals on `SetVariant`.
#[derive(SystemParam)]
pub struct VariantSpawnParams<'w, 's> {
    commands: Commands<'w, 's>,
    meshes: ResMut<'w, Assets<Mesh>>,
    materials: ResMut<'w, Assets<StandardMaterial>>,
    rig_entities: ResMut<'w, RigEntities>,
    apply_mode: ResMut<'w, PoseApplyMode>,
    active: ResMut<'w, ActiveVariant>,
    shared_variant: Res<'w, SharedVariantState>,
    shared_expressions: Res<'w, crate::v2_expressions::SharedExpressionState>,
    expression_bindings: ResMut<'w, crate::v2_expressions::V2ExpressionBindings>,
    asset_server: Res<'w, AssetServer>,
    skeleton: Res<'w, RigSkeleton>,
    poses: Res<'w, PoseLibrary>,
    visual_roots: Query<'w, 's, Entity, With<DollVisualRoot>>,
}

/// Drains commands sent by the HTTP API (`POST /pose`, `POST /animation`,
/// `POST /variant`, see `http_api.rs`) and starts the corresponding transition or
/// visual swap. Runs before `advance_playback` each frame (see `main.rs`) so a
/// command takes effect the same frame it arrives instead of lagging a frame behind.
/// Also resets [`IdleRevert`]'s activity clock on every pose/animation command
/// actually applied, so `auto_revert_to_idle_pose` doesn't fire while the rig is
/// still being actively directed.
pub fn apply_rig_commands(
    time: Res<Time>,
    receiver: Res<RigCommandReceiver>,
    animations: Res<AnimationLibrary>,
    speed: Res<TransitionSpeed>,
    mut playback: ResMut<RigPlayback>,
    mut idle: ResMut<IdleRevert>,
    mut bored: ResMut<crate::bored_play::BoredPlay>,
    mut variant_spawn: VariantSpawnParams,
) {
    while let Ok(command) = receiver.0.try_recv() {
        match command {
            RigCommand::SetVariant { variant } => {
                if variant_spawn.active.0 == variant {
                    continue;
                }
                for entity in variant_spawn.visual_roots.iter() {
                    variant_spawn.commands.entity(entity).despawn();
                }
                variant_spawn.rig_entities.0.clear();
                crate::v2_expressions::clear_expression_state(
                    &variant_spawn.shared_expressions,
                    &mut variant_spawn.expression_bindings,
                );
                variant_spawn.active.0 = variant;
                variant_spawn.shared_variant.set_active(variant);
                let v2_character = variant_spawn.shared_variant.v2_character();
                spawn_visual_for_variant(
                    variant,
                    &mut variant_spawn.commands,
                    &mut variant_spawn.meshes,
                    &mut variant_spawn.materials,
                    &mut variant_spawn.rig_entities,
                    &mut variant_spawn.apply_mode,
                    &variant_spawn.skeleton,
                    &variant_spawn.poses,
                    &variant_spawn.asset_server,
                    &v2_character,
                );
                info!("switched visual variant to {variant}");
                continue;
            }
            RigCommand::SetExpressions { weights } => {
                if let Err(e) = variant_spawn.shared_expressions.apply_weights(&weights) {
                    warn!("ignoring /expressions: {e}");
                }
                continue;
            }
            _ => {}
        }

        // Snapshotting per-command (rather than once before the loop) means a burst
        // of commands in one frame each blend smoothly from wherever the previous
        // one in the burst left off, not all from the same stale starting point.
        let from = playback.0.current_snapshot(&variant_spawn.skeleton.0);
        match command {
            RigCommand::SetVariant { .. } | RigCommand::SetExpressions { .. } => unreachable!(),
            RigCommand::Pose {
                name,
                speed_deg_per_sec,
            } => {
                let pose = {
                    let poses_guard = variant_spawn.poses.0.read().unwrap();
                    let Some(pose) = poses_guard.get(&name) else {
                        warn!("ignoring /pose command for unknown pose '{name}'");
                        continue;
                    };
                    pose.clone()
                };
                let Ok(target_resolved) = pose.resolve(&variant_spawn.skeleton.0) else {
                    warn!("ignoring /pose command: pose '{name}' failed to resolve");
                    continue;
                };
                let duration_ms = duration_ms_for_speed(
                    &variant_spawn.skeleton.0,
                    &from,
                    &target_resolved,
                    speed_deg_per_sec.unwrap_or(speed.deg_per_sec),
                    speed.min_ms,
                    speed.max_ms,
                );
                playback.0.interrupt(
                    &variant_spawn.skeleton.0,
                    PlaybackTarget::Pose(pose),
                    duration_ms,
                );
                idle.last_activity_secs = time.elapsed_secs();
                idle.reverted = false;
                idle.expect_return_after_animation = false;
                idle.pending_after_animation = false;
                bored.note_directed_playback(time.elapsed_secs());
            }
            RigCommand::Animation { name } => {
                let mut anim = {
                    let animations_guard = animations.0.read().unwrap();
                    let Some(anim) = animations_guard.get(&name) else {
                        warn!("ignoring /animation command for unknown animation '{name}'");
                        continue;
                    };
                    anim.clone()
                };
                // HTTP-triggered playback is always one-shot: play through once, then
                // settle back to idle. YAML `loop: true` is ignored here so celebrate
                // / agent triggers never leave the doll stuck bouncing forever.
                anim.looping = false;
                let Some(first_keyframe) = anim.keyframes.first() else {
                    warn!("ignoring /animation command: '{name}' has no keyframes");
                    continue;
                };
                let Ok(target_resolved) = first_keyframe
                    .pose
                    .resolve(&variant_spawn.skeleton.0)
                else {
                    warn!("ignoring /animation command: '{name}' first keyframe failed to resolve");
                    continue;
                };
                let duration_ms = duration_ms_for_speed(
                    &variant_spawn.skeleton.0,
                    &from,
                    &target_resolved,
                    speed.deg_per_sec,
                    speed.min_ms,
                    speed.max_ms,
                );
                playback.0.interrupt(
                    &variant_spawn.skeleton.0,
                    PlaybackTarget::Animation(anim),
                    duration_ms,
                );
                idle.last_activity_secs = time.elapsed_secs();
                idle.reverted = false;
                idle.expect_return_after_animation = true;
                idle.pending_after_animation = false;
                bored.note_directed_playback(time.elapsed_secs());
            }
        }
    }
}

/// Once the rig has been idle (holding still, no in-progress transition) for
/// [`IdleRevert::timeout_secs`] since the last command — or immediately after a
/// one-shot animation finishes (`pending_after_animation`) — starts a transition
/// back to `IdleRevert::default_pose_name` (body + default stage camera when the pose
/// includes a `camera:` block or full default patch).
pub fn auto_revert_to_idle_pose(
    time: Res<Time>,
    skeleton: Res<RigSkeleton>,
    poses: Res<PoseLibrary>,
    speed: Res<TransitionSpeed>,
    editor: Res<crate::editor_state::SharedEditorState>,
    mut playback: ResMut<RigPlayback>,
    mut idle: ResMut<IdleRevert>,
    mut viewport: ResMut<crate::camera_controls::ViewportCamera>,
) {
    if editor.is_active() {
        return;
    }
    if !playback.0.is_idle() || idle.reverted {
        return;
    }
    let after_animation = idle.pending_after_animation;
    if !after_animation {
        let now = time.elapsed_secs();
        if now - idle.last_activity_secs < idle.timeout_secs {
            return;
        }
    }
    idle.pending_after_animation = false;

    let mut default_pose = {
        let poses_guard = poses.0.read().unwrap();
        let Some(pose) = poses_guard.get(&idle.default_pose_name) else {
            warn!(
                "idle revert: default pose '{}' not found — is assets/poses/idle.yaml missing?",
                idle.default_pose_name
            );
            return;
        };
        pose.clone()
    };
    if default_pose.camera.is_none() {
        default_pose.camera = Some(CameraTarget::full_default_stage());
    }
    let from = playback.0.current_snapshot(&skeleton.0);
    let Ok(target_resolved) = default_pose.resolve(&skeleton.0) else {
        warn!(
            "idle revert: default pose '{}' failed to resolve",
            idle.default_pose_name
        );
        return;
    };
    let duration_ms = duration_ms_for_speed(
        &skeleton.0,
        &from,
        &target_resolved,
        speed.deg_per_sec,
        speed.min_ms,
        speed.max_ms,
    );
    playback
        .0
        .interrupt(&skeleton.0, PlaybackTarget::Pose(default_pose), duration_ms);
    viewport.user_orbiting = false;
    // Camera follows blended snapshot via sync_viewport_from_choreography during the transition.
    // Mark reverted (rather than bumping last_activity_secs) so this doesn't count as
    // fresh "activity" — if a caller checked, the rig should still read as having been
    // idle since their last real command, just now visually at the default pose.
    idle.reverted = true;
}

/// Advances the rig's live interpolation state by this frame's delta time and writes
/// the resulting per-joint rotations into their entities' `Transform`s, applies the
/// blended camera orbit to the primary choreography camera, and publishes a snapshot
/// for `GET /state`. When idle, still refreshes `LiveState` so `/state` stays accurate.
///
/// When a playing animation reaches its last keyframe and becomes idle, sets
/// [`IdleRevert::pending_after_animation`] so the next frame's
/// [`auto_revert_to_idle_pose`] returns the doll to the default idle pose.
pub fn advance_playback(
    time: Res<Time>,
    skeleton: Res<RigSkeleton>,
    rig_entities: Res<RigEntities>,
    live_state: Res<crate::live_state::LiveState>,
    apply_mode: Res<PoseApplyMode>,
    shared_expressions: Res<crate::v2_expressions::SharedExpressionState>,
    mut playback: ResMut<RigPlayback>,
    mut idle: ResMut<IdleRevert>,
    mut transforms: Query<&mut Transform>,
    rest_transforms: Query<&RestTransform>,
) {
    let dt_ms = (time.delta_secs() * 1000.0).round() as u32;
    // Always take a snapshot — including Idle. v1 spawns already in STARTUP_POSE, but
    // v2 binds asynchronously onto VRM rest; without writing the held idle pose after
    // bind, the mesh stays in T-pose until the first POST /pose|/animation.
    let snapshot = if dt_ms == 0 {
        playback.0.current_snapshot(&skeleton.0)
    } else if let Some(resolved) = playback.0.advance(&skeleton.0, dt_ms) {
        resolved
    } else {
        playback.0.current_snapshot(&skeleton.0)
    };
    let vrm_local = playback.0.uses_vrm_local_rotations();

    for (joint_id, rotation) in &snapshot.joint_rotations {
        if let Some(&entity) = rig_entities.0.get(joint_id) {
            if let Ok(mut transform) = transforms.get_mut(entity) {
                transform.rotation = match *apply_mode {
                    PoseApplyMode::Absolute => *rotation,
                    PoseApplyMode::RestRelative if vrm_local => *rotation,
                    PoseApplyMode::RestRelative => {
                        if let Ok(rest) = rest_transforms.get(entity) {
                            rest.0.rotation * *rotation
                        } else {
                            *rotation
                        }
                    }
                };
            }
        }
    }
    // Pelvis root motion from VRMA (offset from hips pose at t=0). Applied on v2 for
    // the hips bone only; other joint translations remain v1-only cosmetic offsets.
    if let Some(pelvis_id) = skeleton.0.joint_by_name("pelvis") {
        if let Some(offset) = snapshot.joint_translations.get(&pelvis_id) {
            if let Some(&entity) = rig_entities.0.get(&pelvis_id) {
                if let Ok(mut transform) = transforms.get_mut(entity) {
                    if let Ok(rest) = rest_transforms.get(entity) {
                        transform.translation = rest.0.translation + *offset;
                    }
                }
            }
        }
    }
    // Joint translations (gaze pupils, pop-out blush, etc.) — v1 only. VRM bind
    // pose owns bone translations; applying paperdoll offsets would yank the mesh.
    if *apply_mode == PoseApplyMode::Absolute {
        for (joint_id, translation) in &snapshot.joint_translations {
            if let Some(&entity) = rig_entities.0.get(joint_id) {
                if let Ok(mut transform) = transforms.get_mut(entity) {
                    let is_root = skeleton.0.joint(*joint_id).parent.is_none();
                    transform.translation = if is_root {
                        *translation + Vec3::new(0.0, ROOT_HEIGHT, 0.0)
                    } else {
                        *translation
                    };
                }
            }
        }
    }
    // Primary camera transform is driven by [`crate::camera_controls::ViewportCamera`]
    // (orbit + choreography sync), not written here.
    // Drive VRM face morphs from the same blend as joints/camera (v2).
    shared_expressions.apply_playback_weights(&snapshot.expressions);
    if idle.expect_return_after_animation && playback.0.is_idle() {
        idle.pending_after_animation = true;
        idle.expect_return_after_animation = false;
    }
    let mode = playback.0.mode();
    live_state.publish(&skeleton.0, &snapshot, &mode);
}

/// Entity id of the primary orbit camera driven by pose/animation `camera` fields.
#[derive(Resource)]
pub struct ChoreographyCameraEntity(pub Entity);
