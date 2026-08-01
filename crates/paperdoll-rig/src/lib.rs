//! Renderer-agnostic paper-doll rig: skeleton, pose/animation data model, and the
//! interpolation engine that drives smooth transitions between poses. Deliberately
//! has no dependency on Bevy (or any renderer) so it can be unit-tested in isolation
//! and reused by a future headless/agent-driven build.

pub mod animation;
pub mod camera;
pub mod interpolation;
pub mod pose;
pub mod skeleton;
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
pub use yaml::{load_animations_from_dir, load_poses_from_dir, resolve_animation, YamlLoadError};
