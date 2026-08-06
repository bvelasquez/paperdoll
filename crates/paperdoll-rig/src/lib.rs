//! Renderer-agnostic paper-doll rig: skeleton, pose/animation data model, and the
//! interpolation engine that drives smooth transitions between poses. Deliberately
//! has no dependency on Bevy (or any renderer) so it can be unit-tested in isolation
//! and reused by a future headless/agent-driven build.

pub mod animation;
pub mod camera;
pub mod demo_motions;
pub mod hand;
pub mod interpolation;
pub mod pose;
pub mod skeleton;
pub mod vrma;
pub mod yaml;

pub use animation::{Animation, AnimationFile, Easing, Keyframe, KeyframeSpec};
pub use camera::{
    blend_cameras, merge_camera_targets, CameraTarget, ResolvedCamera, DEFAULT_CAMERA,
};
pub use interpolation::{
    blend_expressions, blend_poses, blend_translations, duration_ms_for_speed, PlaybackMode,
    PlaybackState, PlaybackTarget,
};
pub use pose::{EulerDeg, JointTarget, Pose, PoseError, ResolvedPose};
pub use skeleton::{Joint, JointId, Skeleton, SkeletonBuilder};
pub use demo_motions::{DemoMotion, DEMO_MOTIONS, DEMO_VRMA_BASE_URL};
pub use hand::{HandGesture, HAND_GESTURE_JOINT_KEYS};
pub use vrma::{
    import_vrma_from_bytes, import_vrma_from_path, VrmaImportConfig, VrmaImportError,
    VrmaImportResult,
};
pub use yaml::{
    animation_to_file, animation_yaml_path, hand_gesture_yaml_path, load_animation_file,
    load_animations_from_dir, load_hand_gestures_from_dir, load_poses_from_dir, pose_yaml_path,
    resolve_animation, sanitize_asset_filename, write_animation_yaml, write_hand_gesture_yaml,
    write_pose_yaml, YamlLoadError, YamlWriteError,
};
