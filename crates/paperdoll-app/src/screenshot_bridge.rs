//! Bridges HTTP `GET /screenshot` requests (`http_api.rs`, on its own OS thread) into
//! Bevy's built-in screenshot capture, which — like any render operation — can only be
//! triggered from Bevy's own world via `Commands`. Mirrors the channel-based bridge
//! pattern in `rig_bridge.rs`'s `RigCommand`, but one-shot: each request carries its
//! own reply channel instead of going through a shared queue, since a screenshot needs
//! to send bytes *back* to one specific waiting HTTP handler, not just start something.
//!
//! This exists so a caller (a human, or an AI agent driving the rig) can see the
//! result of a pose/animation command without needing eyes on the actual window —
//! `POST /pose` then `GET /screenshot` closes the loop entirely over HTTP.

use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};

/// One pending `GET /screenshot` request: capture the primary window and send the
/// PNG-encoded bytes back over `reply`.
pub struct ScreenshotRequest {
    pub reply: crossbeam_channel::Sender<Vec<u8>>,
}

#[derive(Resource)]
pub struct ScreenshotRequestReceiver(pub crossbeam_channel::Receiver<ScreenshotRequest>);

/// Drains pending screenshot requests and spawns one capture entity per request.
/// Capture happens asynchronously over the next render frame or two; when Bevy fires
/// `ScreenshotCaptured` on the spawned entity, the attached observer encodes the image
/// to PNG and sends it back over that specific request's `reply` channel, so concurrent
/// requests can't cross-deliver each other's bytes.
pub fn handle_screenshot_requests(receiver: Res<ScreenshotRequestReceiver>, mut commands: Commands) {
    while let Ok(request) = receiver.0.try_recv() {
        let reply = request.reply;
        commands
            .spawn(Screenshot::primary_window())
            .observe(move |captured: On<ScreenshotCaptured>| {
                let bytes = match captured.image.clone().try_into_dynamic() {
                    Ok(dyn_img) => {
                        let mut bytes = Vec::new();
                        match dyn_img
                            .to_rgb8()
                            .write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png)
                        {
                            Ok(()) => bytes,
                            Err(e) => {
                                warn!("screenshot PNG encode failed: {e}");
                                Vec::new()
                            }
                        }
                    }
                    Err(e) => {
                        warn!("screenshot image conversion failed: {e}");
                        Vec::new()
                    }
                };
                // Best-effort: if the HTTP handler already timed out and dropped its
                // receiver, there's nothing meaningful to do with a failed send.
                let _ = reply.send(bytes);
            });
    }
}
