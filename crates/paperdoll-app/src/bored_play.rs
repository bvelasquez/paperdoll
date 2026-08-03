//! Idle “bored” autoplay: while in play mode, periodically trigger a random animation
//! flagged with `play_automatically` in its YAML.

use crate::editor::EditorSession;
use crate::rig_bridge::{
    AnimationLibrary, IdleRevert, RigCommand, RigCommandSender, RigPlayback,
};
use bevy::prelude::*;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

#[derive(Resource, Clone)]
pub struct BoredPlay {
    pub enabled: bool,
    pub interval_secs: f32,
    /// Last time we started a bored clip or reset the timer (app seconds).
    pub last_play_secs: f32,
    /// After startup, wait one full interval before the first bored pick.
    pub armed: bool,
}

impl BoredPlay {
    pub fn from_launch(enabled: bool, interval_secs: f32) -> Self {
        Self {
            enabled,
            interval_secs: interval_secs.max(1.0),
            last_play_secs: 0.0,
            armed: false,
        }
    }

    pub fn note_directed_playback(&mut self, now: f32) {
        self.last_play_secs = now;
        self.armed = true;
    }
}

/// Snap to the default idle pose when the window opens (play mode, not editor preview).
pub fn queue_startup_idle(commands: Res<RigCommandSender>) {
    let _ = commands.0.send(RigCommand::Pose {
        name: "idle".into(),
        speed_deg_per_sec: Some(720.0),
    });
}

pub fn bored_autoplay(
    time: Res<Time>,
    session: Res<EditorSession>,
    mut bored: ResMut<BoredPlay>,
    playback: Res<RigPlayback>,
    idle: Res<IdleRevert>,
    animations: Res<AnimationLibrary>,
    commands: Res<RigCommandSender>,
) {
    if !bored.enabled || session.open {
        return;
    }
    if !playback.0.is_idle() || !idle.is_holding_default_pose() {
        return;
    }

    let now = time.elapsed_secs();
    if !bored.armed {
        bored.armed = true;
        bored.last_play_secs = now;
        return;
    }
    if now - bored.last_play_secs < bored.interval_secs {
        return;
    }

    let pool: Vec<String> = {
        let guard = animations.0.read().unwrap();
        guard
            .values()
            .filter(|a| a.play_automatically)
            .map(|a| a.name.clone())
            .collect()
    };
    if pool.is_empty() {
        bored.last_play_secs = now;
        return;
    }

    let idx = pick_index(&pool, now);
    let name = pool[idx].clone();
    if commands
        .0
        .send(RigCommand::Animation { name })
        .is_ok()
    {
        bored.last_play_secs = now;
    }
}

fn pick_index(pool: &[String], now: f32) -> usize {
    let mut h = DefaultHasher::new();
    now.to_bits().hash(&mut h);
    pool.len().hash(&mut h);
    (h.finish() as usize) % pool.len()
}
