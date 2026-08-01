//! v2 visual: VRM 1.0 skinned humanoid via `bevy_vrm1`.
//!
//! Spawns a [`VrmHandle`]; once the model is [`Initialized`], maps VRM humanoid
//! bone marker entities onto [`crate::rig_bridge::RigEntities`] so the shared
//! playback engine can drive the mesh. Paperdoll face-only joints without a VRM
//! counterpart are skipped. Finger bones map when present on the VRM.

use crate::rig_bridge::{DollVisualRoot, PoseApplyMode, RigEntities, RigSkeleton};
use crate::v2_expressions::V2PendingExpressions;
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy_vrm1::prelude::*;

/// Marker on the VRM root while we wait for humanoid bone components.
#[derive(Component)]
pub struct V2PendingBind;

/// Core body / head / limb humanoid bones.
#[derive(SystemParam)]
pub struct VrmHumanoidBones<'w, 's> {
    hips: Query<'w, 's, Entity, With<Hips>>,
    spine: Query<'w, 's, Entity, With<Spine>>,
    chest: Query<'w, 's, Entity, With<Chest>>,
    upper_chest: Query<'w, 's, Entity, With<UpperChest>>,
    neck: Query<'w, 's, Entity, With<Neck>>,
    head: Query<'w, 's, Entity, With<Head>>,
    jaw: Query<'w, 's, Entity, With<Jaw>>,
    left_eye: Query<'w, 's, Entity, With<LeftEye>>,
    right_eye: Query<'w, 's, Entity, With<RightEye>>,
    left_shoulder: Query<'w, 's, Entity, With<LeftShoulder>>,
    right_shoulder: Query<'w, 's, Entity, With<RightShoulder>>,
    left_upper_arm: Query<'w, 's, Entity, With<LeftUpperArm>>,
    right_upper_arm: Query<'w, 's, Entity, With<RightUpperArm>>,
    left_lower_arm: Query<'w, 's, Entity, With<LeftLowerArm>>,
    right_lower_arm: Query<'w, 's, Entity, With<RightLowerArm>>,
    left_hand: Query<'w, 's, Entity, With<LeftHand>>,
    right_hand: Query<'w, 's, Entity, With<RightHand>>,
    left_upper_leg: Query<'w, 's, Entity, With<LeftUpperLeg>>,
    right_upper_leg: Query<'w, 's, Entity, With<RightUpperLeg>>,
    left_lower_leg: Query<'w, 's, Entity, With<LeftLowerLeg>>,
    right_lower_leg: Query<'w, 's, Entity, With<RightLowerLeg>>,
    left_foot: Query<'w, 's, Entity, With<LeftFoot>>,
    right_foot: Query<'w, 's, Entity, With<RightFoot>>,
    left_toes: Query<'w, 's, Entity, With<LeftToes>>,
    right_toes: Query<'w, 's, Entity, With<RightToes>>,
}

/// Finger humanoid bones (VRM 1.0).
#[derive(SystemParam)]
pub struct VrmFingerBones<'w, 's> {
    left_thumb_metacarpal: Query<'w, 's, Entity, With<LeftThumbMetacarpal>>,
    left_thumb_proximal: Query<'w, 's, Entity, With<LeftThumbProximal>>,
    left_thumb_distal: Query<'w, 's, Entity, With<LeftThumbDistal>>,
    left_index_proximal: Query<'w, 's, Entity, With<LeftIndexProximal>>,
    left_index_intermediate: Query<'w, 's, Entity, With<LeftIndexIntermediate>>,
    left_index_distal: Query<'w, 's, Entity, With<LeftIndexDistal>>,
    left_middle_proximal: Query<'w, 's, Entity, With<LeftMiddleProximal>>,
    left_middle_intermediate: Query<'w, 's, Entity, With<LeftMiddleIntermediate>>,
    left_middle_distal: Query<'w, 's, Entity, With<LeftMiddleDistal>>,
    left_ring_proximal: Query<'w, 's, Entity, With<LeftRingProximal>>,
    left_ring_intermediate: Query<'w, 's, Entity, With<LeftRingIntermediate>>,
    left_ring_distal: Query<'w, 's, Entity, With<LeftRingDistal>>,
    left_little_proximal: Query<'w, 's, Entity, With<LeftLittleProximal>>,
    left_little_intermediate: Query<'w, 's, Entity, With<LeftLittleIntermediate>>,
    left_little_distal: Query<'w, 's, Entity, With<LeftLittleDistal>>,
    right_thumb_metacarpal: Query<'w, 's, Entity, With<RightThumbMetacarpal>>,
    right_thumb_proximal: Query<'w, 's, Entity, With<RightThumbProximal>>,
    right_thumb_distal: Query<'w, 's, Entity, With<RightThumbDistal>>,
    right_index_proximal: Query<'w, 's, Entity, With<RightIndexProximal>>,
    right_index_intermediate: Query<'w, 's, Entity, With<RightIndexIntermediate>>,
    right_index_distal: Query<'w, 's, Entity, With<RightIndexDistal>>,
    right_middle_proximal: Query<'w, 's, Entity, With<RightMiddleProximal>>,
    right_middle_intermediate: Query<'w, 's, Entity, With<RightMiddleIntermediate>>,
    right_middle_distal: Query<'w, 's, Entity, With<RightMiddleDistal>>,
    right_ring_proximal: Query<'w, 's, Entity, With<RightRingProximal>>,
    right_ring_intermediate: Query<'w, 's, Entity, With<RightRingIntermediate>>,
    right_ring_distal: Query<'w, 's, Entity, With<RightRingDistal>>,
    right_little_proximal: Query<'w, 's, Entity, With<RightLittleProximal>>,
    right_little_intermediate: Query<'w, 's, Entity, With<RightLittleIntermediate>>,
    right_little_distal: Query<'w, 's, Entity, With<RightLittleDistal>>,
}

impl VrmHumanoidBones<'_, '_> {
    fn pairs(&self) -> Vec<(&str, Option<Entity>)> {
        vec![
            ("pelvis", self.hips.iter().next()),
            ("spine", self.spine.iter().next()),
            ("chest", self.chest.iter().next()),
            ("upper_chest", self.upper_chest.iter().next()),
            ("neck", self.neck.iter().next()),
            ("head", self.head.iter().next()),
            ("jaw", self.jaw.iter().next()),
            ("left_eye", self.left_eye.iter().next()),
            ("right_eye", self.right_eye.iter().next()),
            ("left_clavicle", self.left_shoulder.iter().next()),
            ("right_clavicle", self.right_shoulder.iter().next()),
            ("left_shoulder", self.left_upper_arm.iter().next()),
            ("right_shoulder", self.right_upper_arm.iter().next()),
            ("left_elbow", self.left_lower_arm.iter().next()),
            ("right_elbow", self.right_lower_arm.iter().next()),
            ("left_wrist", self.left_hand.iter().next()),
            ("right_wrist", self.right_hand.iter().next()),
            ("left_hand", self.left_hand.iter().next()),
            ("right_hand", self.right_hand.iter().next()),
            ("left_hip", self.left_upper_leg.iter().next()),
            ("right_hip", self.right_upper_leg.iter().next()),
            ("left_knee", self.left_lower_leg.iter().next()),
            ("right_knee", self.right_lower_leg.iter().next()),
            ("left_ankle", self.left_foot.iter().next()),
            ("right_ankle", self.right_foot.iter().next()),
            ("left_toe", self.left_toes.iter().next()),
            ("right_toe", self.right_toes.iter().next()),
        ]
    }
}

impl VrmFingerBones<'_, '_> {
    fn pairs(&self) -> Vec<(&str, Option<Entity>)> {
        vec![
            ("left_thumb_metacarpal", self.left_thumb_metacarpal.iter().next()),
            ("left_thumb_proximal", self.left_thumb_proximal.iter().next()),
            ("left_thumb_distal", self.left_thumb_distal.iter().next()),
            ("left_index_proximal", self.left_index_proximal.iter().next()),
            (
                "left_index_intermediate",
                self.left_index_intermediate.iter().next(),
            ),
            ("left_index_distal", self.left_index_distal.iter().next()),
            ("left_middle_proximal", self.left_middle_proximal.iter().next()),
            (
                "left_middle_intermediate",
                self.left_middle_intermediate.iter().next(),
            ),
            ("left_middle_distal", self.left_middle_distal.iter().next()),
            ("left_ring_proximal", self.left_ring_proximal.iter().next()),
            (
                "left_ring_intermediate",
                self.left_ring_intermediate.iter().next(),
            ),
            ("left_ring_distal", self.left_ring_distal.iter().next()),
            ("left_little_proximal", self.left_little_proximal.iter().next()),
            (
                "left_little_intermediate",
                self.left_little_intermediate.iter().next(),
            ),
            ("left_little_distal", self.left_little_distal.iter().next()),
            (
                "right_thumb_metacarpal",
                self.right_thumb_metacarpal.iter().next(),
            ),
            ("right_thumb_proximal", self.right_thumb_proximal.iter().next()),
            ("right_thumb_distal", self.right_thumb_distal.iter().next()),
            ("right_index_proximal", self.right_index_proximal.iter().next()),
            (
                "right_index_intermediate",
                self.right_index_intermediate.iter().next(),
            ),
            ("right_index_distal", self.right_index_distal.iter().next()),
            (
                "right_middle_proximal",
                self.right_middle_proximal.iter().next(),
            ),
            (
                "right_middle_intermediate",
                self.right_middle_intermediate.iter().next(),
            ),
            ("right_middle_distal", self.right_middle_distal.iter().next()),
            ("right_ring_proximal", self.right_ring_proximal.iter().next()),
            (
                "right_ring_intermediate",
                self.right_ring_intermediate.iter().next(),
            ),
            ("right_ring_distal", self.right_ring_distal.iter().next()),
            (
                "right_little_proximal",
                self.right_little_proximal.iter().next(),
            ),
            (
                "right_little_intermediate",
                self.right_little_intermediate.iter().next(),
            ),
            ("right_little_distal", self.right_little_distal.iter().next()),
        ]
    }
}

/// Spawn the v2 VRM character (async load). [`bind_v2_rig_entities`] fills
/// [`RigEntities`] after [`Initialized`].
pub fn spawn_v2_visual(
    commands: &mut Commands,
    asset_server: &AssetServer,
    character_asset: &str,
    apply_mode: &mut PoseApplyMode,
    rig_entities: &mut RigEntities,
) {
    rig_entities.0.clear();
    *apply_mode = PoseApplyMode::RestRelative;

    let handle: Handle<VrmAsset> = asset_server.load(character_asset.to_string());
    commands.spawn((
        VrmHandle(handle),
        DollVisualRoot,
        V2PendingBind,
        Name::new("paperdoll_v2_vrm"),
        Transform::IDENTITY,
        Visibility::default(),
    ));
}

fn insert_pairs(
    rig_entities: &mut RigEntities,
    skeleton: &RigSkeleton,
    pairs: Vec<(&str, Option<Entity>)>,
) -> usize {
    let mut bound = 0usize;
    for (name, entity) in pairs {
        let Some(entity) = entity else {
            continue;
        };
        let Some(joint_id) = skeleton.0.joint_by_name(name) else {
            continue;
        };
        if rig_entities.0.contains_key(&joint_id) {
            continue;
        }
        rig_entities.0.insert(joint_id, entity);
        bound += 1;
    }
    bound
}

/// After VRM humanoid bones are ready, wire them into [`RigEntities`] and queue
/// expression binding.
pub fn bind_v2_rig_entities(
    mut commands: Commands,
    mut rig_entities: ResMut<RigEntities>,
    skeleton: Res<RigSkeleton>,
    pending: Query<Entity, (With<Vrm>, With<Initialized>, With<V2PendingBind>)>,
    body: VrmHumanoidBones,
    fingers: VrmFingerBones,
) {
    let Ok(root) = pending.single() else {
        return;
    };

    let body_pairs = body.pairs();
    if body_pairs[0].1.is_none() {
        return;
    }

    rig_entities.0.clear();
    let body_bound = insert_pairs(&mut rig_entities, &skeleton, body_pairs);
    let finger_bound = insert_pairs(&mut rig_entities, &skeleton, fingers.pairs());

    commands
        .entity(root)
        .remove::<V2PendingBind>()
        .insert(V2PendingExpressions);
    info!(
        "v2 VRM bound: {body_bound} body + {finger_bound} finger joints \
         (entity {root})"
    );
}
