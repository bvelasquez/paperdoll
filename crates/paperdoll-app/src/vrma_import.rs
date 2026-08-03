//! CLI and shared helpers for importing `.vrma` into the animation library.

use crate::rig_bridge::ANIMATIONS_DIR;
use paperdoll_rig::{
    import_vrma_from_bytes, import_vrma_from_path, resolve_animation, write_animation_yaml,
    Animation, VrmaImportConfig, VrmaImportError,
};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub const MOTIONS_DIR: &str = "assets/motions";

#[derive(Debug, Clone)]
pub struct ImportVrmaCli {
    pub path: PathBuf,
    pub name: Option<String>,
    pub sample_interval_ms: u32,
    pub write_yaml: bool,
    pub looping: bool,
}

/// Parse `paperdoll import-vrma …` from argv (after the subcommand).
pub fn parse_import_vrma_args(args: &[String]) -> Result<ImportVrmaCli, String> {
    if args.is_empty() {
        return Err("import-vrma requires a .vrma file path".into());
    }
    let mut path = PathBuf::from(&args[0]);
    let mut name = None;
    let mut sample_interval_ms = 100u32;
    let mut write_yaml = true;
    let mut looping = false;
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--name" => {
                name = Some(
                    args.get(i + 1)
                        .ok_or("--name requires a value")?
                        .clone(),
                );
                i += 2;
            }
            "--interval-ms" => {
                sample_interval_ms = args
                    .get(i + 1)
                    .ok_or("--interval-ms requires a value")?
                    .parse()
                    .map_err(|_| "invalid --interval-ms")?;
                i += 2;
            }
            "--no-write" => {
                write_yaml = false;
                i += 1;
            }
            "--loop" => {
                looping = true;
                i += 1;
            }
            "--help" | "-h" => {
                print_import_usage();
                std::process::exit(0);
            }
            other => return Err(format!("unknown import-vrma argument '{other}'")),
        }
    }
    if !path.extension().is_some_and(|e| e.eq_ignore_ascii_case("vrma")) {
        path.set_extension("vrma");
    }
    Ok(ImportVrmaCli {
        path,
        name,
        sample_interval_ms,
        write_yaml,
        looping,
    })
}

pub fn print_import_usage() {
    eprintln!(
        "\
Import a VRM Animation (.vrma) into paperdoll YAML + optional library file.

Usage:
  paperdoll import-vrma <file.vrma> [options]

Options:
  --name <id>         Animation name (default: file stem, sanitized)
  --interval-ms <n>   Sample interval in ms (default: 100)
  --no-write          Do not write assets/animations/<name>.yaml
  --loop              Set animation loop flag in YAML
  --help, -h          Show this help

Examples:
  paperdoll import-vrma assets/motions/Clapping.vrma --name vrma_clapping
"
    );
}

/// Resolve a user path: absolute paths as-is; otherwise relative to asset root / cwd.
pub fn resolve_vrma_input_path(asset_root: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        return path.to_path_buf();
    }
    let from_cwd = PathBuf::from(path);
    if from_cwd.is_file() {
        return from_cwd;
    }
    asset_root.join(path)
}

/// Path under `assets/` only — rejects `..` and absolute paths.
pub fn safe_assets_relative_path(user: &str) -> Result<PathBuf, String> {
    let trimmed = user.trim();
    if trimmed.is_empty() {
        return Err("path must not be empty".into());
    }
    if trimmed.contains("..") || Path::new(trimmed).is_absolute() {
        return Err("path must be relative to assets/ without '..'".into());
    }
    Ok(PathBuf::from(trimmed))
}

pub struct ImportVrmaOutcome {
    pub result: paperdoll_rig::VrmaImportResult,
    pub animation: Animation,
    pub yaml_path: Option<PathBuf>,
}

pub fn import_vrma_file(
    vrma_path: &Path,
    config: VrmaImportConfig,
    animations_dir: &Path,
    write_yaml: bool,
) -> Result<ImportVrmaOutcome, VrmaImportError> {
    let imported = if vrma_path.is_file() {
        import_vrma_from_path(vrma_path, config)?
    } else {
        return Err(VrmaImportError::ReadFile(
            vrma_path.display().to_string(),
            std::io::Error::new(std::io::ErrorKind::NotFound, "file not found"),
        ));
    };

    let animation = resolve_animation(imported.animation_file.clone(), &HashMap::new())
        .map_err(|e| VrmaImportError::Gltf(format!("resolve animation: {e}")))?;

    let yaml_path = if write_yaml {
        let path = paperdoll_rig::animation_yaml_path(animations_dir, &animation.name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                VrmaImportError::ReadFile(parent.display().to_string(), e)
            })?;
        }
        write_animation_yaml(&path, &imported.animation_file).map_err(|e| {
            VrmaImportError::Gltf(format!("write yaml: {e}"))
        })?;
        Some(path)
    } else {
        None
    };

    Ok(ImportVrmaOutcome {
        result: imported,
        animation,
        yaml_path,
    })
}

pub fn import_vrma_bytes(
    bytes: &[u8],
    config: VrmaImportConfig,
    animations_dir: &Path,
    write_yaml: bool,
) -> Result<ImportVrmaOutcome, VrmaImportError> {
    let imported = import_vrma_from_bytes(bytes, config)?;
    let animation = resolve_animation(imported.animation_file.clone(), &HashMap::new())
        .map_err(|e| VrmaImportError::Gltf(format!("resolve animation: {e}")))?;
    let yaml_path = if write_yaml {
        let path = paperdoll_rig::animation_yaml_path(animations_dir, &animation.name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                VrmaImportError::ReadFile(parent.display().to_string(), e)
            })?;
        }
        write_animation_yaml(&path, &imported.animation_file).map_err(|e| {
            VrmaImportError::Gltf(format!("write yaml: {e}"))
        })?;
        Some(path)
    } else {
        None
    };
    Ok(ImportVrmaOutcome {
        result: imported,
        animation,
        yaml_path,
    })
}

pub fn run_import_vrma_cli(cli: ImportVrmaCli, asset_root: &Path) -> Result<(), String> {
    let vrma_path = resolve_vrma_input_path(asset_root, &cli.path);
    if !vrma_path.is_file() {
        return Err(format!(
            "VRMA file not found: '{}' (cwd: {})",
            vrma_path.display(),
            std::env::current_dir()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|_| "?".into())
        ));
    }
    let mut config = VrmaImportConfig::from_path_stem(&vrma_path);
    if let Some(n) = cli.name {
        config.name = paperdoll_rig::sanitize_asset_filename(&n);
    }
    config.sample_interval_ms = cli.sample_interval_ms;
    config.looping = cli.looping;

    let outcome = import_vrma_file(
        &vrma_path,
        config,
        Path::new(ANIMATIONS_DIR),
        cli.write_yaml,
    )
    .map_err(|e| e.to_string())?;

    let summary = serde_json::json!({
        "animation": outcome.animation.name,
        "duration_ms": outcome.result.duration_ms,
        "keyframes": outcome.result.keyframe_count,
        "mapped_bones": outcome.result.mapped_bone_count,
        "yaml": outcome.yaml_path.as_ref().map(|p| p.display().to_string()),
    });
    println!("{}", serde_json::to_string_pretty(&summary).unwrap());
    Ok(())
}

/// Download missing catalog `.vrma` files into `assets/motions/` via `curl`.
pub fn fetch_demo_motions(motions_dir: &Path) -> Result<Vec<PathBuf>, String> {
    use paperdoll_rig::{DEMO_MOTIONS, DEMO_VRMA_BASE_URL};

    std::fs::create_dir_all(motions_dir).map_err(|e| {
        format!(
            "create motions dir '{}': {e}",
            motions_dir.display()
        )
    })?;

    let mut downloaded = Vec::new();
    for demo in DEMO_MOTIONS {
        let dest = motions_dir.join(demo.file_name);
        if dest.is_file() {
            continue;
        }
        let url = format!("{DEMO_VRMA_BASE_URL}{}", demo.file_name);
        let status = std::process::Command::new("curl")
            .args(["-fsSL", "-o"])
            .arg(&dest)
            .arg(&url)
            .status()
            .map_err(|e| format!("failed to run curl for {url}: {e}"))?;
        if !status.success() {
            return Err(format!("curl failed ({status}) for {url}"));
        }
        downloaded.push(dest);
    }
    Ok(downloaded)
}

pub fn run_fetch_demo_motions_cli(asset_root: &Path) -> Result<(), String> {
    let motions_dir = asset_root.join(MOTIONS_DIR);
    let downloaded = fetch_demo_motions(&motions_dir)?;
    let summary = serde_json::json!({
        "motions_dir": motions_dir.display().to_string(),
        "downloaded": downloaded.iter().map(|p| p.file_name().unwrap().to_string_lossy().to_string()).collect::<Vec<_>>(),
        "catalog": paperdoll_rig::DEMO_MOTIONS.iter().map(|d| d.file_name).collect::<Vec<_>>(),
    });
    println!("{}", serde_json::to_string_pretty(&summary).unwrap());
    Ok(())
}

/// Fetch (if needed) and import every catalog demo into `assets/animations/*.yaml`.
pub fn import_all_demo_motions(asset_root: &Path, write_yaml: bool) -> Result<Vec<ImportVrmaOutcome>, String> {
    use paperdoll_rig::DEMO_MOTIONS;

    let motions_dir = asset_root.join(MOTIONS_DIR);
    fetch_demo_motions(&motions_dir)?;

    let mut outcomes = Vec::with_capacity(DEMO_MOTIONS.len());
    for demo in DEMO_MOTIONS {
        let vrma_path = motions_dir.join(demo.file_name);
        if !vrma_path.is_file() {
            return Err(format!(
                "missing demo file '{}' after fetch",
                vrma_path.display()
            ));
        }
        let mut config = VrmaImportConfig::from_path_stem(&vrma_path);
        config.name = demo.animation_name.into();
        config.sample_interval_ms = demo.sample_interval_ms;
        config.description = Some(format!(
            "Demo VRMA import from `{}` (see assets/motions/ATTRIBUTION.md)",
            demo.file_name
        ));
        outcomes.push(
            import_vrma_file(&vrma_path, config, Path::new(ANIMATIONS_DIR), write_yaml)
                .map_err(|e| e.to_string())?,
        );
    }
    Ok(outcomes)
}

pub fn run_import_demo_motions_cli(asset_root: &Path, write_yaml: bool) -> Result<(), String> {
    let outcomes = import_all_demo_motions(asset_root, write_yaml)?;
    let rows: Vec<_> = outcomes
        .iter()
        .map(|o| {
            serde_json::json!({
                "animation": o.animation.name,
                "duration_ms": o.result.duration_ms,
                "keyframes": o.result.keyframe_count,
                "yaml": o.yaml_path.as_ref().map(|p| p.display().to_string()),
            })
        })
        .collect();
    println!("{}", serde_json::to_string_pretty(&rows).unwrap());
    Ok(())
}
