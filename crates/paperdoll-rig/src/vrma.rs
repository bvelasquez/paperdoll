//! Import [VRM Animation](https://vrm.dev/en/vrma/) (`.vrma` / GLB + `VRMC_vrm_animation`)
//! into paperdoll [`AnimationFile`] keyframes by sampling rotation (and expression) curves.

use crate::animation::{AnimationFile, Easing, KeyframeSpec};
use crate::pose::JointTarget;
use crate::skeleton::Skeleton;
use glam::{Quat, Vec3};
use serde::Deserialize;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::Path;

#[derive(Debug, thiserror::Error)]
pub enum VrmaImportError {
    #[error("failed to read '{0}': {1}")]
    ReadFile(String, std::io::Error),
    #[error("not a valid GLB: {0}")]
    InvalidGlb(String),
    #[error("gltf import failed: {0}")]
    Gltf(String),
    #[error("VRMC_vrm_animation extension missing or invalid: {0}")]
    Extension(String),
    #[error("no animations in file")]
    NoAnimation,
    #[error("no humanoid bones mapped to paperdoll joints")]
    NoMappedBones,
    #[error("animation produced no keyframes")]
    EmptyKeyframes,
}

#[derive(Debug, Clone)]
pub struct VrmaImportConfig {
    pub name: String,
    pub description: Option<String>,
    /// Time between sampled body keyframes (ms).
    pub sample_interval_ms: u32,
    pub looping: bool,
    /// Skip joints whose rest-relative delta is below this (degrees).
    pub min_joint_delta_deg: f32,
    /// Import hips/pelvis translation channels (often floats the avatar off the floor).
    pub import_root_translation: bool,
    /// Scale clip motion on spine, legs, head, etc. (1.0 = full).
    pub rotation_strength: f32,
    /// Scale arm-chain motion (clavicle → hand) to reduce hand-hand penetration on retarget.
    pub arm_rotation_strength: f32,
    /// Scale finger motion (often overlaps during clap).
    pub finger_rotation_strength: f32,
}

impl Default for VrmaImportConfig {
    fn default() -> Self {
        Self {
            name: "vrma_import".into(),
            description: None,
            sample_interval_ms: 100,
            looping: false,
            min_joint_delta_deg: 0.5,
            import_root_translation: true,
            rotation_strength: 1.0,
            arm_rotation_strength: 0.72,
            finger_rotation_strength: 0.82,
        }
    }
}

impl VrmaImportConfig {
    pub fn from_path_stem(path: &Path) -> Self {
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("vrma_import");
        Self {
            name: crate::yaml::sanitize_asset_filename(stem),
            description: Some(format!(
                "Imported from VRM Animation `{}`",
                path.file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("unknown")
            )),
            sample_interval_ms: 100,
            looping: false,
            min_joint_delta_deg: 0.5,
            import_root_translation: true,
            rotation_strength: 1.0,
            arm_rotation_strength: 0.72,
            finger_rotation_strength: 0.82,
        }
    }
}

#[derive(Debug, Clone)]
pub struct VrmaImportResult {
    pub animation_file: AnimationFile,
    pub duration_ms: u32,
    pub keyframe_count: usize,
    pub mapped_bone_count: usize,
}

pub fn import_vrma_from_path(path: &Path, config: VrmaImportConfig) -> Result<VrmaImportResult, VrmaImportError> {
    let bytes = std::fs::read(path).map_err(|e| VrmaImportError::ReadFile(path.display().to_string(), e))?;
    import_vrma_from_bytes(&bytes, config)
}

pub fn import_vrma_from_bytes(
    bytes: &[u8],
    config: VrmaImportConfig,
) -> Result<VrmaImportResult, VrmaImportError> {
    import_vrma_from_bytes_inner(bytes, config)
}

fn import_vrma_from_bytes_inner(
    bytes: &[u8],
    config: VrmaImportConfig,
) -> Result<VrmaImportResult, VrmaImportError> {
    let root = glb_json_value(bytes)?;
    let ext = root
        .pointer("/extensions/VRMC_vrm_animation")
        .ok_or_else(|| VrmaImportError::Extension("missing VRMC_vrm_animation".into()))?;

    let node_to_joint = humanoid_node_map(ext)?;
    let node_to_expression = expression_node_map(ext);
    if node_to_joint.is_empty() {
        return Err(VrmaImportError::NoMappedBones);
    }

    let (document, buffers, _) =
        gltf::import_slice(bytes).map_err(|e| VrmaImportError::Gltf(e.to_string()))?;

    let buffer_data = |buffer: gltf::Buffer| Some(buffers[buffer.index()].0.as_slice());

    let animation = document
        .animations()
        .next()
        .ok_or(VrmaImportError::NoAnimation)?;

    let skeleton = Skeleton::humanoid_default();
    let interval_s = (config.sample_interval_ms.max(1) as f32) / 1000.0;
    let _ = interval_s;

    let mut duration_s = 0.0f32;
    let mut rotation_channels: HashMap<usize, RotationChannel> = HashMap::new();
    let mut translation_channels: HashMap<usize, TranslationChannel> = HashMap::new();
    let mut expression_channels: HashMap<usize, TranslationChannel> = HashMap::new();

    for channel in animation.channels() {
        let target_node = channel.target().node().index();
        let reader = channel.reader(buffer_data);
        let Some(inputs) = reader.read_inputs() else {
            continue;
        };
        let times: Vec<f32> = inputs.collect();
        if let Some(&last) = times.last() {
            duration_s = duration_s.max(last);
        }

        let Some(outputs) = reader.read_outputs() else {
            continue;
        };

        match channel.target().property() {
            gltf::animation::Property::Rotation => {
                if node_to_expression.contains_key(&target_node) {
                    continue;
                }
                let Some(joint) = node_to_joint.get(&target_node) else {
                    continue;
                };
                if skeleton.joint_by_name(joint).is_none() {
                    continue;
                }
                let gltf::animation::util::ReadOutputs::Rotations(rots) = outputs else {
                    continue;
                };
                let rotations: Vec<[f32; 4]> = rots.into_f32().collect();
                rotation_channels.insert(
                    target_node,
                    RotationChannel {
                        joint: joint.clone(),
                        times,
                        rotations,
                    },
                );
            }
            gltf::animation::Property::Translation => {
                let gltf::animation::util::ReadOutputs::Translations(trans) = outputs else {
                    continue;
                };
                let translations: Vec<[f32; 3]> = trans.collect();
                if let Some(expr) = node_to_expression.get(&target_node) {
                    expression_channels.insert(
                        target_node,
                        TranslationChannel {
                            label: expr.clone(),
                            times,
                            translations,
                        },
                    );
                } else if node_to_joint.get(&target_node).is_some_and(|j| j == "pelvis") {
                    translation_channels.insert(
                        target_node,
                        TranslationChannel {
                            label: "pelvis".into(),
                            times,
                            translations,
                        },
                    );
                }
            }
            _ => {}
        }
    }

    if rotation_channels.is_empty() {
        return Err(VrmaImportError::NoMappedBones);
    }

    let hips_trans_t0 = translation_channels
        .values()
        .next()
        .and_then(|ch| sample_translation(&ch.times, &ch.translations, 0.0))
        .map(Vec3::from);

    let mut rotation_at_t0: HashMap<usize, Quat> = HashMap::new();
    for (&node_idx, ch) in &rotation_channels {
        if let Some(q) = sample_rotation(&ch.times, &ch.rotations, 0.0) {
            rotation_at_t0.insert(node_idx, q);
        }
    }

    let duration_ms = (duration_s * 1000.0).ceil() as u32;
    let sample_interval_ms = config.sample_interval_ms.max(1);
    let mut sample_times_ms = vec![0u32];
    let mut t_ms = sample_interval_ms;
    while t_ms < duration_ms {
        sample_times_ms.push(t_ms);
        t_ms += sample_interval_ms;
    }
    if *sample_times_ms.last().unwrap_or(&0) != duration_ms && duration_ms > 0 {
        sample_times_ms.push(duration_ms);
    }

    let tracked_joints: HashSet<String> = rotation_channels
        .values()
        .map(|c| c.joint.clone())
        .collect();

    let mut keyframes = Vec::with_capacity(sample_times_ms.len());
    for (i, &time_ms) in sample_times_ms.iter().enumerate() {
        let t = time_ms as f32 / 1000.0;
        let mut joints = HashMap::new();

        for (&node_idx, ch) in &rotation_channels {
            let Some(q) = sample_rotation(&ch.times, &ch.rotations, t) else {
                continue;
            };
            let Some(_joint_id) = skeleton.joint_by_name(&ch.joint) else {
                continue;
            };
            let q0 = rotation_at_t0
                .get(&node_idx)
                .copied()
                .unwrap_or(q);
            let delta = q0.inverse() * q;
            let strength = joint_rotation_strength(&ch.joint, &config);
            let delta_scaled = if strength >= 1.0 - 1e-5 {
                delta
            } else {
                Quat::IDENTITY.slerp(delta, strength)
            };
            let local = q0 * delta_scaled;
            let min_rad = config.min_joint_delta_deg.to_radians();
            if delta_scaled.angle_between(Quat::IDENTITY) < min_rad && time_ms != 0 {
                continue;
            }
            joints.insert(
                ch.joint.clone(),
                JointTarget {
                    rotation_deg: None,
                    rotation_quat: Some(local.to_array()),
                    translation: None,
                },
            );
        }

        if config.import_root_translation {
            if let Some(ch) = translation_channels.values().next() {
                if let Some(tr) = sample_translation(&ch.times, &ch.translations, t) {
                    let base = hips_trans_t0.unwrap_or(Vec3::ZERO);
                    let offset = Vec3::from(tr) - base;
                    if offset.length_squared() > 1e-8 {
                        joints
                            .entry("pelvis".into())
                            .or_insert_with(JointTarget::default)
                            .translation = Some(offset.to_array());
                    }
                }
            }
        }

        let mut expressions = HashMap::new();
        for ch in expression_channels.values() {
            if let Some(tr) = sample_translation(&ch.times, &ch.translations, t) {
                let weight = tr[0].clamp(0.0, 1.0);
                if weight > 1e-4 {
                    expressions.insert(ch.label.clone(), weight);
                }
            }
        }

        let duration_ms = if i == 0 {
            0
        } else {
            time_ms.saturating_sub(sample_times_ms[i - 1])
        };

        keyframes.push(KeyframeSpec {
            pose: None,
            joints: if joints.is_empty() && expressions.is_empty() {
                None
            } else {
                Some(joints)
            },
            camera: None,
            expressions: if expressions.is_empty() {
                None
            } else {
                Some(expressions)
            },
            hold: None,
            duration_ms,
            easing: Easing::Linear,
        });
    }

    keyframes.retain(|kf| {
        kf.joints.is_some() || kf.expressions.is_some() || kf.duration_ms == 0
    });
    if keyframes.is_empty() {
        return Err(VrmaImportError::EmptyKeyframes);
    }
    // Ensure every tracked joint appears on each body keyframe so interpolation
    // does not reset unlisted joints to rest mid-clip.
    fill_tracked_joints(
        &mut keyframes,
        &tracked_joints,
        &rotation_channels,
        &rotation_at_t0,
        config.min_joint_delta_deg,
    );

    let animation_file = AnimationFile {
        name: config.name.clone(),
        description: config.description,
        looping: config.looping,
        vrm_local_rotations: true,
        play_automatically: false,
        keyframes,
    };

    Ok(VrmaImportResult {
        keyframe_count: animation_file.keyframes.len(),
        duration_ms,
        mapped_bone_count: tracked_joints.len(),
        animation_file,
    })
}

/// After sparse per-frame joint extraction, copy previous sample for any tracked
/// bone missing on a keyframe so playback stays continuous.
fn fill_tracked_joints(
    keyframes: &mut [KeyframeSpec],
    tracked: &HashSet<String>,
    channels: &HashMap<usize, RotationChannel>,
    rotation_at_t0: &HashMap<usize, Quat>,
    _min_deg: f32,
) {
    let mut last: HashMap<String, JointTarget> = HashMap::new();
    let joint_to_node: HashMap<&str, usize> = channels
        .iter()
        .map(|(&node, ch)| (ch.joint.as_str(), node))
        .collect();
    for kf in keyframes.iter_mut() {
        let joints = kf.joints.get_or_insert_with(HashMap::new);
        for name in tracked {
            if joints.contains_key(name) {
                if let Some(jt) = joints.get(name) {
                    last.insert(name.clone(), jt.clone());
                }
                continue;
            }
            if let Some(prev) = last.get(name) {
                joints.insert(name.clone(), prev.clone());
            }
        }
        // Frame 0: ensure tracked bones that never exceeded min_deg still get clip rest pose
        if kf.duration_ms == 0 {
            for ch in channels.values() {
                if joints.contains_key(&ch.joint) {
                    continue;
                }
                let node = joint_to_node.get(ch.joint.as_str()).copied();
                let q0 = node
                    .and_then(|n| rotation_at_t0.get(&n))
                    .copied()
                    .unwrap_or(Quat::IDENTITY);
                joints.insert(
                    ch.joint.clone(),
                    JointTarget {
                        rotation_deg: None,
                        rotation_quat: Some(q0.to_array()),
                        translation: None,
                    },
                );
            }
        }
    }
}

fn joint_rotation_strength(joint: &str, config: &VrmaImportConfig) -> f32 {
    if joint.contains("_thumb_")
        || joint.contains("_index_")
        || joint.contains("_middle_")
        || joint.contains("_ring_")
        || joint.contains("_little_")
    {
        config.finger_rotation_strength.clamp(0.0, 1.0)
    } else if joint.contains("_clavicle")
        || joint.contains("_shoulder")
        || joint.contains("_elbow")
        || joint.contains("_wrist")
        || joint.ends_with("_hand")
    {
        config.arm_rotation_strength.clamp(0.0, 1.0)
    } else {
        config.rotation_strength.clamp(0.0, 1.0)
    }
}

struct RotationChannel {
    joint: String,
    times: Vec<f32>,
    rotations: Vec<[f32; 4]>,
}

struct TranslationChannel {
    label: String,
    times: Vec<f32>,
    translations: Vec<[f32; 3]>,
}

fn sample_rotation(times: &[f32], rotations: &[[f32; 4]], t: f32) -> Option<Quat> {
    if times.is_empty() || rotations.is_empty() {
        return None;
    }
    if t <= times[0] {
        return Some(quat_from_gltf(rotations[0]));
    }
    if t >= *times.last()? {
        return Some(quat_from_gltf(*rotations.last()?));
    }
    let idx = times.partition_point(|&s| s < t).saturating_sub(1);
    let t0 = times[idx];
    let t1 = times.get(idx + 1)?;
    let u = ((t - t0) / (t1 - t0)).clamp(0.0, 1.0);
    let a = quat_from_gltf(rotations[idx]);
    let b = quat_from_gltf(rotations[idx + 1]);
    Some(a.slerp(b, u))
}

fn sample_translation(times: &[f32], translations: &[[f32; 3]], t: f32) -> Option<[f32; 3]> {
    if times.is_empty() || translations.is_empty() {
        return None;
    }
    if t <= times[0] {
        return Some(translations[0]);
    }
    if t >= *times.last()? {
        return Some(*translations.last()?);
    }
    let idx = times.partition_point(|&s| s < t).saturating_sub(1);
    let t0 = times[idx];
    let t1 = times.get(idx + 1)?;
    let u = ((t - t0) / (t1 - t0)).clamp(0.0, 1.0);
    let a = translations[idx];
    let b = translations[idx + 1];
    Some([
        a[0] + (b[0] - a[0]) * u,
        a[1] + (b[1] - a[1]) * u,
        a[2] + (b[2] - a[2]) * u,
    ])
}

fn quat_from_gltf(v: [f32; 4]) -> Quat {
    Quat::from_xyzw(v[0], v[1], v[2], v[3]).normalize()
}

fn glb_json_value(bytes: &[u8]) -> Result<Value, VrmaImportError> {
    if bytes.len() < 20 || &bytes[0..4] != b"glTF" {
        return Err(VrmaImportError::InvalidGlb("missing glTF magic".into()));
    }
    let json_len = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
    let json_start: usize = 20;
    let json_end = json_start
        .checked_add(json_len)
        .ok_or_else(|| VrmaImportError::InvalidGlb("json chunk length overflow".into()))?;
    if json_end > bytes.len() {
        return Err(VrmaImportError::InvalidGlb("json chunk truncated".into()));
    }
    serde_json::from_slice(&bytes[json_start..json_end])
        .map_err(|e| VrmaImportError::InvalidGlb(e.to_string()))
}

#[derive(Deserialize)]
struct HumanBoneRef {
    node: usize,
}

fn humanoid_node_map(ext: &Value) -> Result<HashMap<usize, String>, VrmaImportError> {
    let bones = ext
        .pointer("/humanoid/humanBones")
        .ok_or_else(|| VrmaImportError::Extension("missing humanoid.humanBones".into()))?;

    let mut map = HashMap::new();
    let obj = bones
        .as_object()
        .ok_or_else(|| VrmaImportError::Extension("humanBones not an object".into()))?;

    for (vrm_name, value) in obj {
        let Some(paperdoll) = vrm_bone_to_paperdoll(vrm_name) else {
            continue;
        };
        let hb: HumanBoneRef = serde_json::from_value(value.clone())
            .map_err(|e| VrmaImportError::Extension(e.to_string()))?;
        map.insert(hb.node, paperdoll.to_string());
    }
    Ok(map)
}

fn expression_node_map(ext: &Value) -> HashMap<usize, String> {
    let mut map = HashMap::new();
    for section in ["preset", "custom"] {
        let Some(obj) = ext.pointer(&format!("/expressions/{section}")).and_then(|v| v.as_object())
        else {
            continue;
        };
        for (name, value) in obj {
            let Ok(hb) = serde_json::from_value::<HumanBoneRef>(value.clone()) else {
                continue;
            };
            if let Some(paperdoll) = vrm_expression_to_paperdoll(name) {
                map.insert(hb.node, paperdoll);
            }
        }
    }
    map
}

fn vrm_expression_to_paperdoll(vrm: &str) -> Option<String> {
    match vrm {
        "blinkLeft" | "blinkRight" => Some("blink".into()),
        "happy" | "angry" | "sad" | "relaxed" | "surprised" | "blink" | "aa" | "ih" | "ou"
        | "ee" | "oh" => Some(vrm.to_string()),
        other => {
            let lower = other.to_ascii_lowercase();
            match lower.as_str() {
                "happy" | "angry" | "sad" | "relaxed" | "surprised" | "blink" => Some(other.to_string()),
                _ => None,
            }
        }
    }
}

/// VRM 1.0 humanoid bone name → paperdoll joint name (matches `v2_vrm` binding).
fn vrm_bone_to_paperdoll(vrm: &str) -> Option<&'static str> {
    Some(match vrm {
        "hips" => "pelvis",
        "spine" => "spine",
        "chest" => "chest",
        "upperChest" => "upper_chest",
        "neck" => "neck",
        "head" => "head",
        "jaw" => "jaw",
        "leftUpperLeg" => "left_hip",
        "rightUpperLeg" => "right_hip",
        "leftLowerLeg" => "left_knee",
        "rightLowerLeg" => "right_knee",
        "leftFoot" => "left_ankle",
        "rightFoot" => "right_ankle",
        "leftToes" => "left_toe",
        "rightToes" => "right_toe",
        "leftShoulder" => "left_clavicle",
        "rightShoulder" => "right_clavicle",
        "leftUpperArm" => "left_shoulder",
        "rightUpperArm" => "right_shoulder",
        "leftLowerArm" => "left_elbow",
        "rightLowerArm" => "right_elbow",
        "leftHand" => "left_wrist",
        "rightHand" => "right_wrist",
        "leftThumbMetacarpal" => "left_thumb_metacarpal",
        "leftThumbProximal" => "left_thumb_proximal",
        "leftThumbDistal" => "left_thumb_distal",
        "leftIndexProximal" => "left_index_proximal",
        "leftIndexIntermediate" => "left_index_intermediate",
        "leftIndexDistal" => "left_index_distal",
        "leftMiddleProximal" => "left_middle_proximal",
        "leftMiddleIntermediate" => "left_middle_intermediate",
        "leftMiddleDistal" => "left_middle_distal",
        "leftRingProximal" => "left_ring_proximal",
        "leftRingIntermediate" => "left_ring_intermediate",
        "leftRingDistal" => "left_ring_distal",
        "leftLittleProximal" => "left_little_proximal",
        "leftLittleIntermediate" => "left_little_intermediate",
        "leftLittleDistal" => "left_little_distal",
        "rightThumbMetacarpal" => "right_thumb_metacarpal",
        "rightThumbProximal" => "right_thumb_proximal",
        "rightThumbDistal" => "right_thumb_distal",
        "rightIndexProximal" => "right_index_proximal",
        "rightIndexIntermediate" => "right_index_intermediate",
        "rightIndexDistal" => "right_index_distal",
        "rightMiddleProximal" => "right_middle_proximal",
        "rightMiddleIntermediate" => "right_middle_intermediate",
        "rightMiddleDistal" => "right_middle_distal",
        "rightRingProximal" => "right_ring_proximal",
        "rightRingIntermediate" => "right_ring_intermediate",
        "rightRingDistal" => "right_ring_distal",
        "rightLittleProximal" => "right_little_proximal",
        "rightLittleIntermediate" => "right_little_intermediate",
        "rightLittleDistal" => "right_little_distal",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imports_clapping_vrma_fixture() {
        let path = Path::new("../../assets/motions/Clapping.vrma");
        if !path.is_file() {
            return;
        }
        let mut config = VrmaImportConfig::from_path_stem(path);
        config.name = "vrma_clapping".into();
        config.sample_interval_ms = 150;
        let result = import_vrma_from_path(path, config).expect("import");
        assert!(result.keyframe_count >= 3, "keyframes {}", result.keyframe_count);
        assert!(result.duration_ms > 500);
        assert!(result.mapped_bone_count >= 20);
        let skeleton = Skeleton::humanoid_default();
        for kf in &result.animation_file.keyframes {
            if let Some(joints) = &kf.joints {
                for name in joints.keys() {
                    assert!(
                        skeleton.joint_by_name(name).is_some(),
                        "unknown joint {name}"
                    );
                }
            }
        }

        let animation = crate::resolve_animation(result.animation_file.clone(), &HashMap::new())
            .expect("resolve imported animation");
        let total = crate::interpolation::PlaybackState::animation_playable_duration_ms(&animation);
        let at_start =
            crate::interpolation::PlaybackState::pose_at_animation_time(&skeleton, &animation, 0);
        let at_mid = crate::interpolation::PlaybackState::pose_at_animation_time(
            &skeleton,
            &animation,
            total / 2,
        );
        let shoulder = skeleton.joint_by_name("right_shoulder").unwrap();
        let r0 = at_start
            .joint_rotations
            .get(&shoulder)
            .copied()
            .unwrap_or(skeleton.joint(shoulder).local_rotation);
        let r1 = at_mid
            .joint_rotations
            .get(&shoulder)
            .copied()
            .unwrap_or(skeleton.joint(shoulder).local_rotation);
        assert!(
            r0.angle_between(r1).to_degrees() > 15.0,
            "clapping should move the right arm substantially mid-clip"
        );
    }

    #[test]
    fn demo_catalog_imports_when_files_present() {
        use crate::demo_motions::DEMO_MOTIONS;

        let root = Path::new("../../assets/motions");
        let mut imported_any = false;
        for demo in DEMO_MOTIONS {
            let path = root.join(demo.file_name);
            if !path.is_file() {
                continue;
            }
            let mut config = VrmaImportConfig::from_path_stem(&path);
            config.name = demo.animation_name.into();
            config.sample_interval_ms = demo.sample_interval_ms;
            let result = import_vrma_from_path(&path, config).expect("import demo");
            assert!(result.keyframe_count >= 2, "{}", demo.animation_name);
            imported_any = true;
        }
        assert!(
            imported_any,
            "no demo .vrma files under assets/motions — run `paperdoll fetch-demo-motions`"
        );
    }
}
