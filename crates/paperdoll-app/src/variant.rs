//! Visual variant A/B: **v2** VRM skinned mesh (default) vs **v1** procedural doll.
//!
//! Selected at launch (`--variant` / `PAPERDOLL_VARIANT`) and at runtime via
//! `GET`/`POST /variant`. Pose/animation HTTP APIs are shared; only the mesh
//! representation changes.

use bevy::prelude::Resource;
use serde::{Deserialize, Serialize};
use std::env;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

/// Default Bevy asset path (under the `assets/` folder) for the v2 character.
pub const DEFAULT_V2_CHARACTER: &str = "characters/default.vrm";

/// On-disk path relative to the paperdoll asset root (contains `assets/`).
pub fn v2_character_disk_path(asset_path: &str) -> PathBuf {
    PathBuf::from("assets").join(asset_path)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DollVariant {
    V1,
    V2,
}

impl DollVariant {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::V1 => "v1",
            Self::V2 => "v2",
        }
    }

    pub fn parse(s: &str) -> Result<Self, String> {
        match s.trim().to_ascii_lowercase().as_str() {
            "v1" | "1" | "procedural" => Ok(Self::V1),
            "v2" | "2" | "vrm" => Ok(Self::V2),
            other => Err(format!(
                "unknown variant '{other}' (expected v1 or v2)"
            )),
        }
    }
}

impl std::fmt::Display for DollVariant {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Launch-time configuration resolved before `App::run`.
#[derive(Debug, Clone)]
pub struct LaunchConfig {
    pub variant: DollVariant,
    /// Bevy `AssetServer` path for the v2 VRM (e.g. `characters/default.vrm`).
    pub v2_character: String,
}

impl LaunchConfig {
    /// Resolve from CLI args then env. Default variant is **v2**.
    ///
    /// CLI: `paperdoll --variant v1` or `paperdoll --variant=v1`
    /// Env: `PAPERDOLL_VARIANT=v1`, optional `PAPERDOLL_V2_CHARACTER=characters/foo.vrm`
    pub fn from_env_and_args() -> Result<Self, String> {
        let mut variant = env::var("PAPERDOLL_VARIANT")
            .ok()
            .as_deref()
            .map(DollVariant::parse)
            .transpose()?
            .unwrap_or(DollVariant::V2);

        let args: Vec<String> = env::args().skip(1).collect();
        let mut i = 0;
        while i < args.len() {
            let arg = &args[i];
            if arg == "--variant" || arg == "-v" {
                let value = args.get(i + 1).ok_or_else(|| {
                    "--variant requires a value (v1 or v2)".to_string()
                })?;
                variant = DollVariant::parse(value)?;
                i += 2;
                continue;
            }
            if let Some(value) = arg.strip_prefix("--variant=") {
                variant = DollVariant::parse(value)?;
                i += 1;
                continue;
            }
            if arg == "--help" || arg == "-h" {
                print_usage();
                std::process::exit(0);
            }
            return Err(format!(
                "unknown argument '{arg}' (try --help)"
            ));
        }

        let v2_character = env::var("PAPERDOLL_V2_CHARACTER")
            .unwrap_or_else(|_| DEFAULT_V2_CHARACTER.to_string());

        Ok(Self {
            variant,
            v2_character,
        })
    }

    /// Fail fast when launching directly into v2 without a character file.
    pub fn ensure_assets_for_launch(&self) -> Result<(), String> {
        if self.variant != DollVariant::V2 {
            return Ok(());
        }
        let disk = v2_character_disk_path(&self.v2_character);
        if !disk.is_file() {
            return Err(format!(
                "variant v2 requires VRM at '{}' (Bevy path '{}'). \
                 Place a VRM 1.0 humanoid there, or start with --variant v1.",
                disk.display(),
                self.v2_character
            ));
        }
        Ok(())
    }
}

fn print_usage() {
    eprintln!(
        "\
paperdoll — poseable paper-doll character

Usage:
  paperdoll [--variant v1|v2]

Options:
  --variant, -v   Visual variant: v2 (VRM skinned, default) or v1 (procedural)
  --help, -h      Show this help

Environment:
  PAPERDOLL_VARIANT       Same as --variant
  PAPERDOLL_V2_CHARACTER  Bevy asset path under assets/ (default: {DEFAULT_V2_CHARACTER})
  PAPERDOLL_ROOT          Asset root containing assets/poses, assets/animations
"
    );
}

/// Snapshot shared with the HTTP thread for `GET /variant` / capabilities.
#[derive(Debug, Clone, Serialize)]
pub struct VariantSnapshot {
    pub variant: DollVariant,
    pub available: Vec<&'static str>,
    pub v2_character: String,
    pub v2_asset_present: bool,
    pub description: &'static str,
}

impl VariantSnapshot {
    pub fn new(active: DollVariant, v2_character: String) -> Self {
        let disk = v2_character_disk_path(&v2_character);
        Self {
            variant: active,
            available: vec!["v1", "v2"],
            v2_asset_present: disk.is_file(),
            v2_character,
            description: "v2 = VRM 1.0 skinned humanoid (default); \
                v1 = procedural capsule doll. Pose/animation APIs are shared.",
        }
    }
}

/// `Arc` clone held by Bevy + the HTTP API.
#[derive(Resource, Clone)]
pub struct SharedVariantState(pub Arc<RwLock<VariantSnapshot>>);

impl SharedVariantState {
    pub fn new(launch: &LaunchConfig) -> Self {
        Self(Arc::new(RwLock::new(VariantSnapshot::new(
            launch.variant,
            launch.v2_character.clone(),
        ))))
    }

    pub fn set_active(&self, variant: DollVariant) {
        let mut guard = self.0.write().unwrap();
        guard.variant = variant;
        guard.v2_asset_present = Path::new("assets")
            .join(&guard.v2_character)
            .is_file();
    }

    pub fn snapshot(&self) -> VariantSnapshot {
        self.0.read().unwrap().clone()
    }

    pub fn v2_ready(&self) -> bool {
        let guard = self.0.read().unwrap();
        guard.v2_asset_present
    }

    pub fn v2_character(&self) -> String {
        self.0.read().unwrap().v2_character.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_variant_aliases() {
        assert_eq!(DollVariant::parse("v1").unwrap(), DollVariant::V1);
        assert_eq!(DollVariant::parse("V2").unwrap(), DollVariant::V2);
        assert_eq!(DollVariant::parse("procedural").unwrap(), DollVariant::V1);
        assert_eq!(DollVariant::parse("vrm").unwrap(), DollVariant::V2);
        assert!(DollVariant::parse("v3").is_err());
    }
}
