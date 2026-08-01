use crate::animation::{Animation, AnimationFile, Keyframe, KeyframeSpec};
use crate::camera::merge_camera_targets;
use crate::pose::Pose;
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, thiserror::Error)]
pub enum YamlLoadError {
    #[error("failed to read directory '{0}': {1}")]
    ReadDir(String, std::io::Error),
    #[error("failed to read file '{0}': {1}")]
    ReadFile(String, std::io::Error),
    #[error("failed to parse yaml file '{0}': {1}")]
    Parse(String, serde_yaml::Error),
    #[error("animation '{animation}' keyframe {index} references unknown pose '{pose_name}'")]
    UnknownPoseRef {
        animation: String,
        index: usize,
        pose_name: String,
    },
    #[error(
        "animation '{animation}' keyframe {index} must set `pose`, `joints`, `camera`, \
         and/or `expressions` (and must not set both `pose` and `joints`)"
    )]
    InvalidKeyframe { animation: String, index: usize },
}

/// Scans `dir` for `*.yaml` files and parses each as a [`Pose`], keyed by pose name.
/// Returns an empty map (not an error) if `dir` doesn't exist, since a fresh project
/// may not have authored any poses yet.
pub fn load_poses_from_dir(dir: &Path) -> Result<HashMap<String, Pose>, YamlLoadError> {
    let mut poses = HashMap::new();
    if !dir.exists() {
        return Ok(poses);
    }
    for path in yaml_files_in(dir)? {
        let pose: Pose = parse_yaml_file(&path)?;
        poses.insert(pose.name.clone(), pose);
    }
    Ok(poses)
}

/// Scans `dir` for `*.yaml` files and parses each as an [`Animation`], resolving every
/// keyframe's `pose:` reference against the already-loaded `poses` registry via
/// [`resolve_animation`]. Fails fast (rather than panicking at runtime) if a keyframe
/// references a pose that doesn't exist, or specifies neither/both of `pose`/`joints`.
pub fn load_animations_from_dir(
    dir: &Path,
    poses: &HashMap<String, Pose>,
) -> Result<HashMap<String, Animation>, YamlLoadError> {
    let mut animations = HashMap::new();
    if !dir.exists() {
        return Ok(animations);
    }
    for path in yaml_files_in(dir)? {
        let file: AnimationFile = parse_yaml_file(&path)?;
        let animation = resolve_animation(file, poses)?;
        animations.insert(animation.name.clone(), animation);
    }
    Ok(animations)
}

/// Resolves a raw [`AnimationFile`] (parsed from a YAML file, or from JSON POSTed to
/// an HTTP API's animation-registration endpoint) against a pose registry,
/// materializing each keyframe's `pose:` name reference or inline `joints` into a
/// concrete [`Pose`]. Pulled out of [`load_animations_from_dir`] so a caller that
/// already has an in-memory `AnimationFile` (not a file on disk) validates and
/// resolves it identically, instead of duplicating this logic.
pub fn resolve_animation(
    file: AnimationFile,
    poses: &HashMap<String, Pose>,
) -> Result<Animation, YamlLoadError> {
    let mut keyframes = Vec::with_capacity(file.keyframes.len());
    for (index, spec) in file.keyframes.iter().enumerate() {
        let pose = resolve_keyframe_pose(&file.name, index, spec, poses)?;
        keyframes.push(Keyframe {
            pose,
            duration_ms: spec.duration_ms,
            easing: spec.easing,
        });
    }
    Ok(Animation {
        name: file.name,
        description: file.description,
        looping: file.looping,
        keyframes,
    })
}

fn resolve_keyframe_pose(
    animation_name: &str,
    index: usize,
    spec: &KeyframeSpec,
    poses: &HashMap<String, Pose>,
) -> Result<Pose, YamlLoadError> {
    let has_expressions = spec
        .expressions
        .as_ref()
        .is_some_and(|e| !e.is_empty());
    let mut pose = match (&spec.pose, &spec.joints) {
        (Some(name), None) => poses
            .get(name)
            .cloned()
            .ok_or_else(|| YamlLoadError::UnknownPoseRef {
                animation: animation_name.to_string(),
                index,
                pose_name: name.clone(),
            })?,
        (None, Some(joints)) => Pose {
            name: format!("{animation_name}#{index}"),
            description: None,
            joints: joints.clone(),
            camera: None,
            expressions: HashMap::new(),
            hold_joints: false,
        },
        (None, None) if spec.camera.is_some() || has_expressions => Pose {
            // Camera-only and/or expression-only: empty joints + hold_joints so the
            // body freezes while the camera / face morphs move (without hold_joints,
            // empty joints would reset to T-pose).
            name: format!("{animation_name}#{index}"),
            description: None,
            joints: HashMap::new(),
            camera: None,
            expressions: HashMap::new(),
            hold_joints: true,
        },
        _ => {
            return Err(YamlLoadError::InvalidKeyframe {
                animation: animation_name.to_string(),
                index,
            })
        }
    };
    // `hold: true` turns the keyframe into a sparse overlay: only the listed joints
    // / expressions move; everything else keeps its current value.
    if spec.hold.unwrap_or(false) {
        pose.hold_joints = true;
    }
    pose.camera = merge_camera_targets(pose.camera.take(), spec.camera.clone());
    if let Some(expr) = &spec.expressions {
        for (k, v) in expr {
            pose.expressions.insert(k.clone(), *v);
        }
    }
    Ok(pose)
}

fn yaml_files_in(dir: &Path) -> Result<Vec<std::path::PathBuf>, YamlLoadError> {
    let mut paths = Vec::new();
    let entries =
        std::fs::read_dir(dir).map_err(|e| YamlLoadError::ReadDir(dir.display().to_string(), e))?;
    for entry in entries {
        let entry = entry.map_err(|e| YamlLoadError::ReadDir(dir.display().to_string(), e))?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("yaml") {
            paths.push(path);
        }
    }
    paths.sort();
    Ok(paths)
}

fn parse_yaml_file<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, YamlLoadError> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| YamlLoadError::ReadFile(path.display().to_string(), e))?;
    serde_yaml::from_str(&text).map_err(|e| YamlLoadError::Parse(path.display().to_string(), e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    struct TempDir(std::path::PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir()
                .join(format!("paperdoll-rig-test-{name}-{}", std::process::id()));
            std::fs::create_dir_all(&path).unwrap();
            TempDir(path)
        }

        fn write(&self, filename: &str, contents: &str) {
            let mut f = std::fs::File::create(self.0.join(filename)).unwrap();
            f.write_all(contents.as_bytes()).unwrap();
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn loads_poses_from_directory() {
        let dir = TempDir::new("poses");
        dir.write(
            "wave.yaml",
            "name: wave\njoints:\n  right_shoulder:\n    rotation_deg: { z: -80.0 }\n",
        );
        dir.write("t_pose.yaml", "name: t_pose\njoints: {}\n");
        dir.write("not_yaml.txt", "ignored");

        let poses = load_poses_from_dir(&dir.0).unwrap();
        assert_eq!(poses.len(), 2);
        assert!(poses.contains_key("wave"));
        assert!(poses.contains_key("t_pose"));
    }

    #[test]
    fn missing_pose_directory_returns_empty_map_not_error() {
        let poses = load_poses_from_dir(Path::new("/nonexistent/path/for/sure")).unwrap();
        assert!(poses.is_empty());
    }

    #[test]
    fn loads_animation_and_resolves_pose_references() {
        let pose_dir = TempDir::new("anim-poses");
        pose_dir.write(
            "wave.yaml",
            "name: wave\njoints:\n  right_shoulder:\n    rotation_deg: { z: -80.0 }\n",
        );
        pose_dir.write("t_pose.yaml", "name: t_pose\njoints: {}\n");
        let poses = load_poses_from_dir(&pose_dir.0).unwrap();

        let anim_dir = TempDir::new("anims");
        anim_dir.write(
            "wave_animation.yaml",
            r#"
name: wave_animation
loop: false
keyframes:
  - pose: t_pose
    duration_ms: 0
    easing: linear
  - pose: wave
    duration_ms: 500
    easing: ease_in_out
  - pose: t_pose
    duration_ms: 600
    easing: ease_out
"#,
        );

        let animations = load_animations_from_dir(&anim_dir.0, &poses).unwrap();
        let anim = animations.get("wave_animation").unwrap();
        assert_eq!(anim.keyframes.len(), 3);
        assert_eq!(anim.keyframes[1].pose.name, "wave");
    }

    #[test]
    fn resolve_animation_matches_load_animations_from_dir_for_the_same_file() {
        // `resolve_animation` is the function an HTTP registration endpoint would call
        // with an in-memory `AnimationFile` (no file on disk) — this checks it
        // produces the same result `load_animations_from_dir` would for an equivalent
        // file, since both are supposed to share this exact validation/resolution path.
        let mut poses = HashMap::new();
        poses.insert(
            "wave".to_string(),
            Pose {
                name: "wave".to_string(),
                description: None,
                joints: HashMap::new(),
                camera: None,
                expressions: HashMap::new(),
                hold_joints: false,
            },
        );
        let file: AnimationFile = serde_yaml::from_str(
            "name: greet\nkeyframes:\n  - pose: wave\n    duration_ms: 300\n    easing: linear\n",
        )
        .unwrap();
        let animation = resolve_animation(file, &poses).unwrap();
        assert_eq!(animation.name, "greet");
        assert_eq!(animation.keyframes[0].pose.name, "wave");
    }

    #[test]
    fn animation_referencing_unknown_pose_fails_fast() {
        let anim_dir = TempDir::new("bad-anims");
        anim_dir.write(
            "broken.yaml",
            "name: broken\nkeyframes:\n  - pose: does_not_exist\n    duration_ms: 100\n",
        );
        let poses = HashMap::new();
        let err = load_animations_from_dir(&anim_dir.0, &poses).unwrap_err();
        assert!(matches!(err, YamlLoadError::UnknownPoseRef { .. }));
    }

    #[test]
    fn animation_keyframe_with_inline_joints_does_not_need_pose_registry() {
        let anim_dir = TempDir::new("inline-anims");
        anim_dir.write(
            "inline.yaml",
            "name: inline\nkeyframes:\n  - joints:\n      head:\n        rotation_deg: { y: 30.0 }\n    duration_ms: 200\n",
        );
        let poses = HashMap::new();
        let animations = load_animations_from_dir(&anim_dir.0, &poses).unwrap();
        let anim = &animations["inline"];
        assert_eq!(anim.keyframes[0].pose.joints.len(), 1);
    }

    #[test]
    fn keyframe_expressions_overlay_pose_expressions() {
        let mut poses = HashMap::new();
        let mut expr = HashMap::new();
        expr.insert("happy".into(), 0.5);
        poses.insert(
            "wave".to_string(),
            Pose {
                name: "wave".to_string(),
                description: None,
                joints: HashMap::new(),
                camera: None,
                expressions: expr,
                hold_joints: false,
            },
        );
        let file: AnimationFile = serde_yaml::from_str(
            r#"
name: cheer
keyframes:
  - pose: wave
    expressions: { happy: 1.0, blink: 0.3 }
    duration_ms: 200
    easing: ease_out
"#,
        )
        .unwrap();
        let animation = resolve_animation(file, &poses).unwrap();
        let e = &animation.keyframes[0].pose.expressions;
        assert!((e["happy"] - 1.0).abs() < 1e-6);
        assert!((e["blink"] - 0.3).abs() < 1e-6);
    }

    #[test]
    fn expression_only_keyframe_is_valid_hold() {
        let file: AnimationFile = serde_yaml::from_str(
            r#"
name: blink
keyframes:
  - expressions: { blink: 1.0 }
    hold: true
    duration_ms: 90
    easing: step
"#,
        )
        .unwrap();
        let animation = resolve_animation(file, &HashMap::new()).unwrap();
        assert!(animation.keyframes[0].pose.hold_joints);
        assert!((animation.keyframes[0].pose.expressions["blink"] - 1.0).abs() < 1e-6);
    }
}
