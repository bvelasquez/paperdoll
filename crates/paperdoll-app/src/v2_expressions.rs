//! VRM expression (blend-shape) control for variant v2.
//!
//! Parses `VRMC_vrm.expressions.preset` from the loaded `.vrm` (GLB) and applies
//! weights to Bevy [`MorphWeights`] after the skinned meshes spawn.

use bevy::prelude::Resource;
use bevy::prelude::*;
use bevy_vrm1::prelude::{Initialized, Vrm};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use crate::variant::SharedVariantState;

/// One morph-target bind from the VRM expression preset.
#[derive(Debug, Clone)]
struct ExpressionBindSpec {
    node_name: String,
    morph_index: usize,
}

#[derive(Debug, Clone)]
pub(crate) struct ExpressionSpec {
    name: String,
    binds: Vec<ExpressionBindSpec>,
}

/// Live expression catalog + weights, shared with the HTTP thread.
#[derive(Resource, Clone)]
pub struct SharedExpressionState(pub Arc<RwLock<ExpressionSnapshot>>);

impl Default for SharedExpressionState {
    fn default() -> Self {
        Self(Arc::new(RwLock::new(ExpressionSnapshot::default())))
    }
}

impl SharedExpressionState {
    pub fn snapshot(&self) -> ExpressionSnapshot {
        self.0.read().unwrap().clone()
    }

    pub fn set_catalog(&self, names: Vec<String>) {
        let mut g = self.0.write().unwrap();
        g.available = names.clone();
        g.weights.clear();
        for name in names {
            g.weights.insert(name, 0.0);
        }
        g.ready = !g.available.is_empty();
    }

    pub fn clear(&self) {
        let mut g = self.0.write().unwrap();
        *g = ExpressionSnapshot::default();
    }

    pub fn apply_weights(&self, weights: &HashMap<String, f32>) -> Result<(), String> {
        let mut g = self.0.write().unwrap();
        if !g.ready {
            return Err("expressions not available (v2 VRM not bound, or model has none)".into());
        }
        for (name, weight) in weights {
            if !g.available.iter().any(|n| n == name) {
                return Err(format!("unknown expression '{name}'"));
            }
            g.weights.insert(name.clone(), (*weight).clamp(0.0, 1.0));
        }
        g.pending = true;
        Ok(())
    }
}

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct ExpressionSnapshot {
    pub ready: bool,
    pub available: Vec<String>,
    pub weights: HashMap<String, f32>,
    #[serde(skip)]
    pub pending: bool,
}

/// Resolved morph targets for the active VRM (Bevy entities).
#[derive(Resource, Default)]
pub struct V2ExpressionBindings {
    /// expression name → list of (mesh entity with MorphWeights, morph index)
    pub binds: HashMap<String, Vec<(Entity, usize)>>,
}

#[derive(Component)]
pub struct V2PendingExpressions;

/// Parse expression presets from a VRM/GLB on disk.
pub fn parse_vrm_expressions(path: &Path) -> Result<Vec<ExpressionSpec>, String> {
    let data = std::fs::read(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    if data.len() < 20 {
        return Err("file too small to be a GLB/VRM".into());
    }
    let magic = &data[0..4];
    if magic != b"glTF" {
        return Err("not a GLB/VRM (missing glTF magic)".into());
    }
    let chunk_len = u32::from_le_bytes(data[12..16].try_into().unwrap()) as usize;
    let chunk_type = &data[16..20];
    if chunk_type != b"JSON" {
        return Err("GLB first chunk is not JSON".into());
    }
    let json_bytes = data
        .get(20..20 + chunk_len)
        .ok_or("GLB JSON chunk truncated")?;
    let root: serde_json::Value =
        serde_json::from_slice(json_bytes).map_err(|e| format!("VRM JSON: {e}"))?;

    #[derive(Deserialize)]
    struct MorphBind {
        node: usize,
        index: usize,
        #[serde(default)]
        weight: f32,
    }
    #[derive(Deserialize)]
    struct Preset {
        #[serde(rename = "morphTargetBinds", default)]
        morph_target_binds: Vec<MorphBind>,
    }

    let nodes = root
        .get("nodes")
        .and_then(|n| n.as_array())
        .ok_or("VRM missing nodes[]")?;
    let vrm = root
        .pointer("/extensions/VRMC_vrm")
        .ok_or("missing extensions.VRMC_vrm (need VRM 1.0)")?;
    let preset = vrm
        .pointer("/expressions/preset")
        .and_then(|p| p.as_object())
        .ok_or("VRM has no expressions.preset")?;

    let mut out = Vec::new();
    for (name, value) in preset {
        let p: Preset = serde_json::from_value(value.clone())
            .map_err(|e| format!("expression '{name}': {e}"))?;
        let mut binds = Vec::new();
        for b in p.morph_target_binds {
            let node_name = nodes
                .get(b.node)
                .and_then(|n| n.get("name"))
                .and_then(|n| n.as_str())
                .unwrap_or("")
                .to_string();
            if node_name.is_empty() {
                continue;
            }
            let _ = b.weight; // VRM bind weight; we scale by runtime expression weight
            binds.push(ExpressionBindSpec {
                node_name,
                morph_index: b.index,
            });
        }
        if !binds.is_empty() {
            out.push(ExpressionSpec {
                name: name.clone(),
                binds,
            });
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

fn vrm_disk_path(v2_character: &str) -> PathBuf {
    PathBuf::from("assets").join(v2_character)
}

/// After VRM init, resolve expression morph targets to mesh entities by node name.
pub fn bind_v2_expressions(
    mut commands: Commands,
    mut bindings: ResMut<V2ExpressionBindings>,
    shared: Res<SharedExpressionState>,
    shared_variant: Res<SharedVariantState>,
    pending: Query<Entity, (With<Vrm>, With<Initialized>, With<V2PendingExpressions>)>,
    names: Query<(Entity, &Name)>,
    morphs: Query<Entity, With<MorphWeights>>,
    children: Query<&Children>,
) {
    let Ok(root) = pending.single() else {
        return;
    };

    let path = vrm_disk_path(&shared_variant.v2_character());
    let specs = match parse_vrm_expressions(&path) {
        Ok(s) => s,
        Err(e) => {
            warn!("v2 expressions: failed to parse {}: {e}", path.display());
            commands.entity(root).remove::<V2PendingExpressions>();
            shared.clear();
            bindings.binds.clear();
            return;
        }
    };

    // Build name → entity for nodes that have MorphWeights (search whole scene under root).
    let mut morph_by_name: HashMap<String, Entity> = HashMap::new();
    let mut stack = vec![root];
    while let Some(e) = stack.pop() {
        if morphs.get(e).is_ok() {
            if let Ok((_, name)) = names.get(e) {
                morph_by_name.insert(name.as_str().to_string(), e);
            }
        }
        if let Ok(kids) = children.get(e) {
            for child in kids.iter() {
                stack.push(child);
            }
        }
    }

    // Also match by Name even if MorphWeights is on the same entity.
    for (entity, name) in names.iter() {
        if morphs.get(entity).is_ok() {
            morph_by_name
                .entry(name.as_str().to_string())
                .or_insert(entity);
        }
    }

    let mut resolved: HashMap<String, Vec<(Entity, usize)>> = HashMap::new();
    let mut available = Vec::new();
    for spec in &specs {
        let mut list = Vec::new();
        for bind in &spec.binds {
            if let Some(&entity) = morph_by_name.get(&bind.node_name) {
                list.push((entity, bind.morph_index));
            }
        }
        if !list.is_empty() {
            available.push(spec.name.clone());
            resolved.insert(spec.name.clone(), list);
        }
    }

    bindings.binds = resolved;
    shared.set_catalog(available.clone());
    commands.entity(root).remove::<V2PendingExpressions>();
    info!(
        "v2 expressions bound: {} presets (from {})",
        available.len(),
        path.display()
    );
}

/// Apply pending expression weights from [`SharedExpressionState`] onto morph targets.
pub fn apply_v2_expressions(
    shared: Res<SharedExpressionState>,
    bindings: Res<V2ExpressionBindings>,
    mut morphs: Query<&mut MorphWeights>,
) {
    let mut snap = shared.0.write().unwrap();
    if !snap.pending {
        return;
    }
    snap.pending = false;
    let weights = snap.weights.clone();
    drop(snap);

    // Zero every bound morph slot, then write current weights (presets can share targets).
    for binds in bindings.binds.values() {
        for &(entity, index) in binds {
            if let Ok(mut mw) = morphs.get_mut(entity) {
                let w = mw.weights_mut();
                if index < w.len() {
                    w[index] = 0.0;
                }
            }
        }
    }
    for (name, weight) in weights {
        let Some(binds) = bindings.binds.get(&name) else {
            continue;
        };
        for &(entity, index) in binds {
            if let Ok(mut mw) = morphs.get_mut(entity) {
                let w = mw.weights_mut();
                if index < w.len() {
                    w[index] = (w[index] + weight).clamp(0.0, 1.0);
                }
            }
        }
    }
}

/// Clear expression state when leaving v2.
pub fn clear_expression_state(
    shared: &SharedExpressionState,
    bindings: &mut V2ExpressionBindings,
) {
    shared.clear();
    bindings.binds.clear();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn parses_alicia_expression_presets() {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../assets/characters/default.vrm");
        let specs = parse_vrm_expressions(&path).expect("parse VRM expressions");
        assert!(specs.len() >= 10, "expected many presets, got {}", specs.len());
        assert!(specs.iter().any(|s| s.name == "happy"));
        assert!(specs.iter().any(|s| s.name == "blink"));
    }
}
