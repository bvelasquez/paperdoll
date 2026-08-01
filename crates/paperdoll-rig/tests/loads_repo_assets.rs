//! Integration test: the actual `assets/poses/*.yaml` and `assets/animations/*.yaml`
//! files checked into the repo must load and resolve cleanly against the default
//! humanoid skeleton. This is what would break if someone hand-edited a YAML file
//! with a typo'd joint name or a bad pose reference.

use paperdoll_rig::{load_animations_from_dir, load_poses_from_dir, Skeleton};
use std::path::Path;

fn repo_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
}

#[test]
fn repo_pose_and_animation_assets_load_and_resolve() {
    let root = repo_root();
    let poses = load_poses_from_dir(&root.join("assets/poses")).expect("poses should load");
    assert_eq!(poses.len(), 12);
    assert!(poses.contains_key("t_pose"));
    assert!(poses.contains_key("wave"));
    assert!(poses.contains_key("wave_return"));
    assert!(poses.contains_key("peace_sign_right"));
    assert!(poses.contains_key("idle"));

    let animations = load_animations_from_dir(&root.join("assets/animations"), &poses)
        .expect("animations should load and resolve pose references");
    assert_eq!(animations.len(), 6);
    let wave_animation = animations
        .get("wave_animation")
        .expect("wave_animation should be loaded");
    assert_eq!(wave_animation.keyframes.len(), 7);
    assert!(animations.contains_key("finger_emote"));
    assert!(animations.contains_key("happy_bounce"));

    let skeleton = Skeleton::humanoid_default();
    for pose in poses.values() {
        pose.resolve(&skeleton)
            .unwrap_or_else(|e| panic!("pose '{}' failed to resolve: {e}", pose.name));
    }
}
