use glam::{Quat, Vec3};
use std::collections::HashMap;

/// Index into a [`Skeleton`]'s joint arena.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct JointId(pub u32);

#[derive(Debug, Clone)]
pub struct Joint {
    pub name: String,
    pub parent: Option<JointId>,
    pub children: Vec<JointId>,
    /// Transform relative to the parent joint's rest pose.
    pub local_translation: Vec3,
    pub local_rotation: Quat,
    /// Distance to this joint's primary child, used only for procedural mesh sizing
    /// (drives the marker-sphere radius at this joint).
    pub length: f32,
    /// Thickness of the bone capsule connecting this joint to its parent, used only
    /// for procedural mesh sizing. Meaningless for the root, which has no incoming
    /// bone.
    pub radius: f32,
}

/// A joint hierarchy (bone tree) in its rest pose, stored as a flat arena indexed by
/// [`JointId`] rather than `Rc<RefCell<_>>` so it stays cheap to copy/iterate.
#[derive(Debug, Clone)]
pub struct Skeleton {
    joints: Vec<Joint>,
    root: JointId,
    name_index: HashMap<String, JointId>,
}

impl Skeleton {
    pub fn root(&self) -> JointId {
        self.root
    }

    pub fn joint(&self, id: JointId) -> &Joint {
        &self.joints[id.0 as usize]
    }

    pub fn joint_by_name(&self, name: &str) -> Option<JointId> {
        self.name_index.get(name).copied()
    }

    pub fn len(&self) -> usize {
        self.joints.len()
    }

    pub fn is_empty(&self) -> bool {
        self.joints.is_empty()
    }

    pub fn joints(&self) -> impl Iterator<Item = (JointId, &Joint)> {
        self.joints
            .iter()
            .enumerate()
            .map(|(i, j)| (JointId(i as u32), j))
    }

    /// World-space (translation, rotation) for every joint, composed from the root down.
    ///
    /// Relies on the builder invariant that a joint's parent always has a lower index
    /// than the joint itself, so a single forward pass suffices.
    pub fn world_transforms(&self) -> Vec<(Vec3, Quat)> {
        let mut result: Vec<(Vec3, Quat)> = Vec::with_capacity(self.joints.len());
        for joint in &self.joints {
            let world = match joint.parent {
                Some(parent_id) => {
                    let (parent_t, parent_r) = result[parent_id.0 as usize];
                    let world_r = parent_r * joint.local_rotation;
                    let world_t = parent_t + parent_r * joint.local_translation;
                    (world_t, world_r)
                }
                None => (joint.local_translation, joint.local_rotation),
            };
            result.push(world);
        }
        result
    }

    /// A ~24-joint humanoid rig proportioned like an adult human (roughly 1.55m tall
    /// in rest pose, feet at y=0 once `paperdoll-app` applies its ground offset):
    /// pelvis/spine/chest/upper_chest/neck/head down the spine, a clavicle-shoulder-
    /// elbow-wrist-hand chain per arm (the clavicle lets the shoulder girdle move
    /// independently of the upper arm, e.g. for a shrug), and a hip-knee-ankle-toe
    /// chain per leg (the toe lets the foot flex/point independently of the ankle).
    pub fn humanoid_default() -> Skeleton {
        let mut b = SkeletonBuilder::new();

        // Spine: lower ab (spine) -> rib cage (chest) -> shoulder-girdle base
        // (upper_chest) -> neck -> head. Three torso segments (rather than one) let a
        // pose bend/twist the torso in stages instead of as a single rigid block.
        // Radii widen at the pelvis and chest but pinch in at the spine, for a
        // feminine hourglass silhouette rather than a straight-sided torso.
        let pelvis = b.add_joint("pelvis", None, Vec3::ZERO, 0.16, 0.15);
        let spine = b.add_joint("spine", Some(pelvis), Vec3::new(0.0, 0.17, 0.0), 0.17, 0.088);
        let chest = b.add_joint("chest", Some(spine), Vec3::new(0.0, 0.18, 0.0), 0.18, 0.115);
        let upper_chest = b.add_joint(
            "upper_chest",
            Some(chest),
            Vec3::new(0.0, 0.13, 0.0),
            0.13,
            0.092,
        );
        let neck = b.add_joint("neck", Some(upper_chest), Vec3::new(0.0, 0.085, 0.0), 0.085, 0.032);
        let head = b.add_joint("head", Some(neck), Vec3::new(0.0, 0.13, 0.0), 0.40, 0.09);

        // Face joints: all children of `head` (or of the eye joints), placed on the
        // front (+Z) surface of the head sphere. Radii are kept tiny (0.01) so their
        // default joint-marker spheres are invisible — cosmetic meshes in
        // `rig_bridge::spawn_rig` replace them.
        // - jaw: rotates around X to open/close the mouth (positive = open)
        // - eyes: static anchors for the eye assembly (sclera + lashes)
        // - pupils: children of the eyes — TRANSLATE for gaze direction
        // - eyelids: children of the eyes — rotate X to swing down over the eye
        //   (0 = natural resting lid, +85 = closed, negative = wide-eyed)
        // - eyebrows: rotate around Z to raise/furrow (mirrored sign convention)
        // - blush: tucked inside the head at rest; TRANSLATE +Z to pop onto the cheek
        b.add_joint("jaw", Some(head), Vec3::new(0.0, -0.055, 0.095), 0.06, 0.01);
        let left_eye = b.add_joint("left_eye", Some(head), Vec3::new(0.045, 0.01, 0.09), 0.01, 0.01);
        let right_eye = b.add_joint("right_eye", Some(head), Vec3::new(-0.045, 0.01, 0.09), 0.01, 0.01);
        b.add_joint("left_pupil", Some(left_eye), Vec3::new(0.0, 0.0, 0.012), 0.01, 0.01);
        b.add_joint("right_pupil", Some(right_eye), Vec3::new(0.0, 0.0, 0.012), 0.01, 0.01);
        b.add_joint("left_eyelid", Some(left_eye), Vec3::new(0.0, 0.022, 0.012), 0.01, 0.01);
        b.add_joint("right_eyelid", Some(right_eye), Vec3::new(0.0, 0.022, 0.012), 0.01, 0.01);
        b.add_joint("left_eyebrow", Some(head), Vec3::new(0.045, 0.055, 0.095), 0.01, 0.01);
        b.add_joint("right_eyebrow", Some(head), Vec3::new(-0.045, 0.055, 0.095), 0.01, 0.01);
        b.add_joint("left_blush", Some(head), Vec3::new(0.075, -0.025, 0.04), 0.01, 0.01);
        b.add_joint("right_blush", Some(head), Vec3::new(-0.075, -0.025, 0.04), 0.01, 0.01);

        // Arms: clavicle (shoulder girdle) -> shoulder (upper arm) -> elbow (forearm)
        // -> wrist -> hand. Mirrored across x for left/right.
        for (side, sign) in [("left", 1.0_f32), ("right", -1.0_f32)] {
            let clavicle = b.add_joint(
                &format!("{side}_clavicle"),
                Some(upper_chest),
                Vec3::new(sign * 0.068, 0.02, 0.0),
                0.10,
                0.032,
            );
            let shoulder = b.add_joint(
                &format!("{side}_shoulder"),
                Some(clavicle),
                Vec3::new(sign * 0.10, 0.0, 0.0),
                0.28,
                0.048,
            );
            let elbow = b.add_joint(
                &format!("{side}_elbow"),
                Some(shoulder),
                Vec3::new(sign * 0.28, 0.0, 0.0),
                0.24,
                0.038,
            );
            let wrist = b.add_joint(
                &format!("{side}_wrist"),
                Some(elbow),
                Vec3::new(sign * 0.24, 0.0, 0.0),
                0.09,
                0.026,
            );
            b.add_joint(
                &format!("{side}_hand"),
                Some(wrist),
                Vec3::new(sign * 0.10, 0.0, 0.0),
                0.11,
                0.023,
            );

            // Legs: hip (thigh) -> knee (shin) -> ankle -> toe. `paperdoll-app` adds a
            // ground-height offset to the pelvis (hip_drop + thigh + shin + 0.06), so
            // the ANKLE lands at anatomical ankle height (y=0.06) and the toe segment
            // slopes down to touch the floor at y=0. Hip stance keeps the thighs just
            // touching at the top, tapering apart below.
            let hip = b.add_joint(
                &format!("{side}_hip"),
                Some(pelvis),
                Vec3::new(sign * 0.09, -0.04, 0.0),
                0.44,
                0.095,
            );
            let knee = b.add_joint(
                &format!("{side}_knee"),
                Some(hip),
                Vec3::new(0.0, -0.44, 0.0),
                0.42,
                0.05,
            );
            let ankle = b.add_joint(
                &format!("{side}_ankle"),
                Some(knee),
                Vec3::new(0.0, -0.42, 0.0),
                0.10,
                0.035,
            );
            b.add_joint(
                &format!("{side}_toe"),
                Some(ankle),
                Vec3::new(0.0, -0.03, 0.15),
                0.06,
                0.026,
            );
        }

        b.build()
    }
}

/// Incrementally constructs a [`Skeleton`], guaranteeing every joint's parent is added
/// before the joint itself (required by [`Skeleton::world_transforms`]).
pub struct SkeletonBuilder {
    joints: Vec<Joint>,
    name_index: HashMap<String, JointId>,
}

impl SkeletonBuilder {
    pub fn new() -> Self {
        Self {
            joints: Vec::new(),
            name_index: HashMap::new(),
        }
    }

    pub fn add_joint(
        &mut self,
        name: &str,
        parent: Option<JointId>,
        local_translation: Vec3,
        length: f32,
        radius: f32,
    ) -> JointId {
        let id = JointId(self.joints.len() as u32);
        self.joints.push(Joint {
            name: name.to_string(),
            parent,
            children: Vec::new(),
            local_translation,
            local_rotation: Quat::IDENTITY,
            length,
            radius,
        });
        self.name_index.insert(name.to_string(), id);
        if let Some(parent_id) = parent {
            self.joints[parent_id.0 as usize].children.push(id);
        }
        id
    }

    pub fn build(self) -> Skeleton {
        let root = self
            .joints
            .iter()
            .position(|j| j.parent.is_none())
            .map(|i| JointId(i as u32))
            .expect("skeleton must have at least one root joint");
        Skeleton {
            joints: self.joints,
            root,
            name_index: self.name_index,
        }
    }
}

impl Default for SkeletonBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn humanoid_default_has_expected_joint_count_and_names() {
        let skeleton = Skeleton::humanoid_default();
        assert_eq!(skeleton.len(), 35);
        for name in [
            "pelvis",
            "spine",
            "chest",
            "upper_chest",
            "neck",
            "head",
            "left_clavicle",
            "left_shoulder",
            "left_elbow",
            "left_wrist",
            "left_hand",
            "right_clavicle",
            "right_shoulder",
            "right_elbow",
            "right_wrist",
            "right_hand",
            "left_hip",
            "left_knee",
            "left_ankle",
            "left_toe",
            "right_hip",
            "right_knee",
            "right_ankle",
            "right_toe",
            "jaw",
            "left_eye",
            "right_eye",
            "left_pupil",
            "right_pupil",
            "left_eyelid",
            "right_eyelid",
            "left_eyebrow",
            "right_eyebrow",
            "left_blush",
            "right_blush",
        ] {
            assert!(
                skeleton.joint_by_name(name).is_some(),
                "missing joint '{name}'"
            );
        }
    }

    #[test]
    fn rest_pose_world_transforms_stack_translations_up_the_spine() {
        let skeleton = Skeleton::humanoid_default();
        let world = skeleton.world_transforms();
        let head_id = skeleton.joint_by_name("head").unwrap();
        let (head_pos, _) = world[head_id.0 as usize];
        // pelvis(0) + spine(0.17) + chest(0.18) + upper_chest(0.13) + neck(0.085) + head(0.13) = 0.695
        assert!(
            (head_pos.y - 0.695).abs() < 1e-5,
            "head_pos.y = {}",
            head_pos.y
        );
    }

    #[test]
    fn rotating_shoulder_moves_child_joints_world_position() {
        let mut skeleton = Skeleton::humanoid_default();
        let shoulder_id = skeleton.joint_by_name("left_shoulder").unwrap();
        let elbow_id = skeleton.joint_by_name("left_elbow").unwrap();

        let before = skeleton.world_transforms();
        let elbow_before = before[elbow_id.0 as usize].0;

        // Rotate the shoulder 90 degrees around Z; the elbow, which sits along +X
        // relative to the shoulder, should swing accordingly.
        let joints_mut = skeleton_joints_mut_for_test(&mut skeleton);
        joints_mut[shoulder_id.0 as usize].local_rotation =
            Quat::from_rotation_z(std::f32::consts::FRAC_PI_2);

        let after = skeleton.world_transforms();
        let elbow_after = after[elbow_id.0 as usize].0;

        assert!(
            elbow_before.distance(elbow_after) > 0.1,
            "expected elbow to move when shoulder rotates: before={elbow_before:?} after={elbow_after:?}"
        );
    }

    #[test]
    fn left_and_right_ankles_match_height_at_rest() {
        let skeleton = Skeleton::humanoid_default();
        let world = skeleton.world_transforms();
        let left = world[skeleton.joint_by_name("left_ankle").unwrap().0 as usize].0;
        let right = world[skeleton.joint_by_name("right_ankle").unwrap().0 as usize].0;
        assert!(
            (left.y - right.y).abs() < 1e-5,
            "ankle height mismatch: left.y={} right.y={}",
            left.y,
            right.y
        );
    }

    // Test-only accessor: production code mutates joints via the builder, but this
    // test needs to perturb a rest-pose rotation directly to exercise world_transforms.
    fn skeleton_joints_mut_for_test(skeleton: &mut Skeleton) -> &mut Vec<Joint> {
        &mut skeleton.joints
    }
}
