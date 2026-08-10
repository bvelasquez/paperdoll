//! Block timeline for pose-centric animation drafts.
//!
//! Each block `i` (i ≥ 1) spans the blend **into** keyframe `i`; width ∝ `duration_ms`.
//! `#0` stays pinned at t = 0.

use crate::editor::apply::capture_scene_to_pose_draft;
use bevy_egui::egui::{self, Color32, Id, Pos2, Rect, Sense, Stroke, Vec2};
use paperdoll_rig::{Animation, Easing, KeyframeSpec, PlaybackState, Pose, Skeleton};
use std::collections::HashMap;

const LABEL_WIDTH: f32 = 132.0;
const ROW_HEIGHT: f32 = 22.0;
const RULER_HEIGHT: f32 = 20.0;
const TRACK_PAD_X: f32 = 8.0;
const MIN_TRACK_WIDTH: f32 = 200.0;
const BLOCK_ROW_HEIGHT: f32 = 30.0;
const MIN_BLOCK_WIDTH: f32 = 28.0;
const EDGE_HANDLE_WIDTH: f32 = 8.0;
pub const MIN_BLEND_MS: u32 = 40;

const BG: Color32 = Color32::from_rgb(22, 28, 42);
const GRID: Color32 = Color32::from_rgb(38, 48, 68);
const BLOCK_POSE: Color32 = Color32::from_rgb(56, 100, 180);
const BLOCK_INLINE: Color32 = Color32::from_rgb(120, 72, 160);
const BLOCK_DELTA_BADGE: Color32 = Color32::from_rgb(220, 175, 55);
const BLOCK_SELECTED: Color32 = Color32::from_rgb(90, 130, 200);
const HANDLE: Color32 = Color32::from_rgb(200, 210, 230);
const PLAYHEAD: Color32 = Color32::from_rgb(230, 140, 60);
const RULER_TEXT: Color32 = Color32::from_rgb(140, 155, 175);

/// Milliseconds along the playable timeline when keyframe `index` is fully reached.
pub fn keyframe_arrival_ms(keyframes: &[KeyframeSpec], index: usize) -> u32 {
    if index == 0 {
        return 0;
    }
    keyframes
        .iter()
        .skip(1)
        .take(index)
        .map(|kf| kf.duration_ms)
        .sum()
}

pub fn playable_total_ms(keyframes: &[KeyframeSpec]) -> u32 {
    keyframes.iter().skip(1).map(|kf| kf.duration_ms).sum()
}

fn keyframe_label(kf: &KeyframeSpec, index: usize) -> String {
    let mut parts = vec![format!("#{index}")];
    if let Some(pose) = &kf.pose {
        parts.push(pose.clone());
    } else if kf.joints.is_some() {
        parts.push("(inline)".into());
    }
    if kf.camera.is_some() {
        parts.push("[cam]".into());
    }
    if parts.len() == 1 {
        parts.push("(empty)".into());
    }
    parts.join(" ")
}

fn ms_to_x(time_ms: u32, track: Rect, total_ms: u32) -> f32 {
    if total_ms == 0 {
        return track.left();
    }
    let t = (time_ms as f32 / total_ms as f32).clamp(0.0, 1.0);
    track.left() + t * track.width()
}

fn x_to_ms(x: f32, track: Rect, total_ms: u32) -> u32 {
    if total_ms == 0 || track.width() <= 0.0 {
        return 0;
    }
    let t = ((x - track.left()) / track.width()).clamp(0.0, 1.0);
    (t * total_ms as f32).round() as u32
}

fn ruler_tick_step_ms(total_ms: u32, track_width: f32) -> u32 {
    let target_px = 64.0;
    let ms_per_px = total_ms as f32 / track_width.max(1.0);
    let raw = (target_px * ms_per_px).max(1.0) as u32;
    let candidates = [10, 25, 50, 100, 250, 500, 1000, 2500, 5000, 10_000];
    candidates
        .into_iter()
        .find(|&step| step >= raw)
        .unwrap_or(10_000)
}

fn block_colors(kf: &KeyframeSpec, selected: bool) -> (Color32, bool) {
    let has_delta = kf.joints.as_ref().is_some_and(|j| !j.is_empty());
    let fill = if selected {
        BLOCK_SELECTED
    } else if kf.pose.is_some() {
        BLOCK_POSE
    } else {
        BLOCK_INLINE
    };
    (fill, has_delta)
}

fn segment_rect(
    index: usize,
    keyframes: &[KeyframeSpec],
    track: Rect,
    total_ms: u32,
) -> Option<Rect> {
    if index == 0 {
        let x = ms_to_x(0, track, total_ms.max(1));
        return Some(Rect::from_center_size(
            Pos2::new(x + 6.0, track.center().y),
            Vec2::new(12.0, BLOCK_ROW_HEIGHT - 8.0),
        ));
    }
    let x0 = ms_to_x(keyframe_arrival_ms(keyframes, index - 1), track, total_ms);
    let x1 = ms_to_x(keyframe_arrival_ms(keyframes, index), track, total_ms);
    let w = (x1 - x0).max(MIN_BLOCK_WIDTH);
    Some(Rect::from_min_size(
        Pos2::new(x0, track.top() + 4.0),
        Vec2::new(w, BLOCK_ROW_HEIGHT - 8.0),
    ))
}

fn right_edge_rect(block: Rect) -> Rect {
    Rect::from_min_max(
        Pos2::new(block.right() - EDGE_HANDLE_WIDTH, block.top()),
        block.right_bottom(),
    )
}

fn clamp_blend_ms(ms: u32) -> u32 {
    ms.max(MIN_BLEND_MS)
}

fn set_keyframe_duration(keyframes: &mut [KeyframeSpec], index: usize, duration_ms: u32) {
    if index == 0 {
        return;
    }
    if let Some(kf) = keyframes.get_mut(index) {
        kf.duration_ms = clamp_blend_ms(duration_ms);
    }
}

fn duration_from_x(
    index: usize,
    keyframes: &[KeyframeSpec],
    x: f32,
    track: Rect,
    total_ms: u32,
) -> u32 {
    let arrival_prev = keyframe_arrival_ms(keyframes, index - 1);
    let target_total = total_ms.max(1);
    let at_x = x_to_ms(x, track, target_total);
    clamp_blend_ms(at_x.saturating_sub(arrival_prev))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DragKind {
    TrimRight,
    MoveBody,
}

#[derive(Clone, Copy, Debug)]
struct TimelineDrag {
    kind: DragKind,
    index: usize,
    start_duration: u32,
    start_pointer_x: f32,
    track_left: f32,
    track_width: f32,
    total_ms: u32,
}

impl TimelineDrag {
    fn apply(&self, pointer_x: f32, keyframes: &mut [KeyframeSpec]) {
        if self.index == 0 || self.index >= keyframes.len() {
            return;
        }
        let track = Rect::from_min_max(
            Pos2::new(self.track_left, 0.0),
            Pos2::new(self.track_left + self.track_width, 1.0),
        );
        let new_duration = match self.kind {
            DragKind::TrimRight => {
                let total_ms = playable_total_ms(keyframes).max(1);
                duration_from_x(self.index, keyframes, pointer_x, track, total_ms)
            }
            DragKind::MoveBody => {
                let dx = pointer_x - self.start_pointer_x;
                let ms_per_px = self.total_ms as f32 / self.track_width.max(1.0);
                let delta_ms = (dx * ms_per_px).round() as i32;
                clamp_blend_ms(
                    (self.start_duration as i32 + delta_ms).max(MIN_BLEND_MS as i32) as u32,
                )
            }
        };
        set_keyframe_duration(keyframes, self.index, new_duration);
    }
}

fn keyframe_spec_from_resolved(
    snapshot: &paperdoll_rig::ResolvedPose,
    skeleton: &Skeleton,
    duration_ms: u32,
    easing: Easing,
) -> KeyframeSpec {
    let mut tmp = Pose {
        name: "split".into(),
        description: None,
        joints: HashMap::new(),
        camera: None,
        expressions: HashMap::new(),
        hold_joints: false,
    };
    capture_scene_to_pose_draft(skeleton, snapshot, &mut tmp);
    KeyframeSpec {
        pose: None,
        joints: if tmp.joints.is_empty() {
            None
        } else {
            Some(tmp.joints)
        },
        camera: tmp.camera,
        expressions: if tmp.expressions.is_empty() {
            None
        } else {
            Some(tmp.expressions)
        },
        hold: None,
        duration_ms,
        easing,
    }
}

/// Split the segment under `playhead_ms`, inserting an inline keyframe at the sampled pose.
/// Returns the index of the inserted keyframe, or `None` if not splittable.
pub fn split_keyframes_at_playhead(
    keyframes: &mut Vec<KeyframeSpec>,
    playhead_ms: u32,
    skeleton: &Skeleton,
    animation: &Animation,
) -> Option<usize> {
    if keyframes.len() < 2 {
        return None;
    }
    let mut seg = None;
    for i in 1..keyframes.len() {
        let a0 = keyframe_arrival_ms(keyframes, i - 1);
        let a1 = keyframe_arrival_ms(keyframes, i);
        if playhead_ms > a0 && playhead_ms < a1 {
            seg = Some(i);
            break;
        }
    }
    let seg = seg?;
    let a0 = keyframe_arrival_ms(keyframes, seg - 1);
    let a1 = keyframe_arrival_ms(keyframes, seg);
    let first_ms = playhead_ms - a0;
    let second_ms = a1 - playhead_ms;
    if first_ms < MIN_BLEND_MS || second_ms < MIN_BLEND_MS {
        return None;
    }

    let sampled = PlaybackState::pose_at_animation_time(skeleton, animation, playhead_ms);
    let easing = keyframes[seg].easing;
    let mut insert = keyframe_spec_from_resolved(&sampled, skeleton, first_ms, easing);
    keyframes[seg].duration_ms = second_ms;
    keyframes.insert(seg, insert);
    Some(seg)
}

pub struct AnimationTimelineResponse {
    pub selected_keyframe: Option<usize>,
    pub playhead_ms: Option<u32>,
    pub preview_camera_keyframe: Option<usize>,
    pub timing_edited: bool,
}

/// Interactive block timeline: scrub, select, drag-to-stagger, trim edges.
pub fn animation_timeline(
    ui: &mut egui::Ui,
    keyframes: &mut [KeyframeSpec],
    selected_keyframe: usize,
    playhead_ms: u32,
) -> AnimationTimelineResponse {
    let mut response = AnimationTimelineResponse {
        selected_keyframe: None,
        playhead_ms: None,
        preview_camera_keyframe: None,
        timing_edited: false,
    };

    let total_ms = playable_total_ms(keyframes);
    let has_camera = keyframes.iter().any(|kf| kf.camera.is_some());
    let extra_row = if has_camera { ROW_HEIGHT } else { 0.0 };
    let outer_width = ui.available_width();
    let track_width = (outer_width - LABEL_WIDTH).max(48.0);
    let content_h = RULER_HEIGHT + BLOCK_ROW_HEIGHT + extra_row + 6.0;
    let size = Vec2::new(outer_width, content_h);

    let drag_id = Id::new("animation_timeline_drag");
    let (rect, sense) = ui.allocate_exact_size(size, Sense::click_and_drag());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 4.0, BG);

    let track_col = Rect::from_min_max(
        Pos2::new(rect.min.x + LABEL_WIDTH, rect.min.y),
        rect.max,
    );
    let track_inner = track_col.shrink2(Vec2::new(TRACK_PAD_X, 0.0));

    // --- time ruler ---
    let ruler = Rect::from_min_size(track_col.min, Vec2::new(track_col.width(), RULER_HEIGHT));
    painter.line_segment(
        [Pos2::new(track_col.min.x, ruler.max.y), track_col.right_top()],
        Stroke::new(1.0, GRID),
    );
    if total_ms > 0 {
        let step = ruler_tick_step_ms(total_ms, track_inner.width());
        let mut t = 0u32;
        while t <= total_ms {
            let x = ms_to_x(t, track_inner, total_ms);
            painter.line_segment(
                [Pos2::new(x, ruler.min.y + 4.0), Pos2::new(x, ruler.max.y)],
                Stroke::new(1.0, GRID),
            );
            painter.text(
                Pos2::new(x + 2.0, ruler.min.y + 2.0),
                egui::Align2::LEFT_TOP,
                format!("{t}"),
                egui::FontId::proportional(10.0),
                RULER_TEXT,
            );
            t = t.saturating_add(step);
        }
    }

    let block_track = Rect::from_min_max(
        Pos2::new(track_col.min.x, rect.min.y + RULER_HEIGHT),
        Pos2::new(track_col.max.x, rect.min.y + RULER_HEIGHT + BLOCK_ROW_HEIGHT),
    );
    painter.text(
        Pos2::new(rect.min.x + 6.0, block_track.center().y),
        egui::Align2::LEFT_CENTER,
        "keyframes",
        egui::FontId::proportional(11.0),
        RULER_TEXT,
    );

    let playhead_x = ms_to_x(playhead_ms, track_inner, total_ms.max(1));

    // --- keyframe blocks ---
    for (i, kf) in keyframes.iter().enumerate() {
        let selected = selected_keyframe == i;
        let Some(block) = segment_rect(i, keyframes, track_inner, total_ms) else {
            continue;
        };
        let (fill, has_delta) = block_colors(kf, selected);
        painter.rect_filled(block, 4.0, fill);
        if selected {
            painter.rect_stroke(block, 4.0, Stroke::new(2.0, Color32::WHITE), egui::StrokeKind::Inside);
            if i > 0 {
                let edge = right_edge_rect(block);
                painter.rect_filled(edge, 2.0, HANDLE);
            }
        }
        if has_delta {
            let badge = Rect::from_min_size(block.right_top() + Vec2::new(-14.0, 2.0), Vec2::splat(12.0));
            painter.rect_filled(badge, 2.0, BLOCK_DELTA_BADGE);
            painter.text(
                badge.center(),
                egui::Align2::CENTER_CENTER,
                "Δ",
                egui::FontId::proportional(9.0),
                Color32::BLACK,
            );
        }
        if kf.camera.is_some() {
            painter.circle_filled(block.right_bottom() + Vec2::new(-6.0, -6.0), 3.0, PLAYHEAD);
        }
        if kf.expressions.as_ref().is_some_and(|e| !e.is_empty()) {
            painter.circle_filled(block.left_bottom() + Vec2::new(6.0, -6.0), 3.0, Color32::from_rgb(80, 200, 100));
        }
        let label = keyframe_label(kf, i);
        painter.text(
            block.left_top() + Vec2::new(4.0, 3.0),
            egui::Align2::LEFT_TOP,
            label,
            egui::FontId::proportional(10.0),
            Color32::WHITE,
        );
    }

    // --- camera row ---
    if has_camera {
        let cam_row = Rect::from_min_max(
            Pos2::new(rect.min.x, block_track.max.y),
            Pos2::new(rect.max.x, block_track.max.y + ROW_HEIGHT),
        );
        painter.text(
            Pos2::new(cam_row.min.x + 6.0, cam_row.center().y),
            egui::Align2::LEFT_CENTER,
            "camera",
            egui::FontId::proportional(11.0),
            RULER_TEXT,
        );
        for (i, kf) in keyframes.iter().enumerate() {
            if kf.camera.is_none() {
                continue;
            }
            let x = ms_to_x(keyframe_arrival_ms(keyframes, i), track_inner, total_ms.max(1));
            painter.circle_filled(Pos2::new(x, cam_row.center().y), 3.5, PLAYHEAD);
        }
    }

    // playhead
    let play_top = rect.min.y + 2.0;
    let play_bottom = rect.max.y - 2.0;
    painter.line_segment(
        [Pos2::new(playhead_x, play_top + 8.0), Pos2::new(playhead_x, play_bottom)],
        Stroke::new(1.5, PLAYHEAD),
    );
    painter.rect_filled(
        Rect::from_center_size(Pos2::new(playhead_x, play_top + 6.0), Vec2::new(8.0, 8.0)),
        1.0,
        PLAYHEAD,
    );

    let mut drag: Option<TimelineDrag> = ui.ctx().data_mut(|d| d.get_temp(drag_id));

    let pointer = ui.input(|i| i.pointer.interact_pos());
    let mut hit_edge: Option<usize> = None;
    let mut hit_body: Option<usize> = None;
    let mut hit_block: Option<usize> = None;

    if let Some(pos) = pointer {
        if drag.is_none() {
            for i in (1..keyframes.len()).rev() {
                if let Some(block) = segment_rect(i, keyframes, track_inner, total_ms) {
                    if right_edge_rect(block).expand(3.0).contains(pos) {
                        hit_edge = Some(i);
                        break;
                    }
                }
            }
            if hit_edge.is_none() {
                for i in (1..keyframes.len()).rev() {
                    if let Some(block) = segment_rect(i, keyframes, track_inner, total_ms) {
                        let mut body = block;
                        body.max.x -= EDGE_HANDLE_WIDTH;
                        if body.expand(2.0).contains(pos) {
                            hit_body = Some(i);
                            break;
                        }
                    }
                }
            }
            for i in (0..keyframes.len()).rev() {
                if let Some(block) = segment_rect(i, keyframes, track_inner, total_ms) {
                    if block.expand(2.0).contains(pos) {
                        hit_block = Some(i);
                        break;
                    }
                }
            }
        }
    }

    if let Some(pos) = pointer {
        let active = drag.is_some() || sense.hovered() || sense.is_pointer_button_down_on();
        if active {
        // Click (no drag): select block or scrub playhead.
        if sense.clicked() {
            if let Some(i) = hit_block {
                response.selected_keyframe = Some(i);
                if sense.double_clicked() && keyframes[i].camera.is_some() {
                    response.preview_camera_keyframe = Some(i);
                }
            } else if track_col.contains(pos) {
                response.playhead_ms = Some(x_to_ms(pos.x, track_inner, total_ms.max(1)));
            }
        }

        // Begin a block timing drag.
        if sense.drag_started() {
            if let Some(i) = hit_edge {
                drag = Some(TimelineDrag {
                    kind: DragKind::TrimRight,
                    index: i,
                    start_duration: keyframes[i].duration_ms,
                    start_pointer_x: pos.x,
                    track_left: track_inner.left(),
                    track_width: track_inner.width(),
                    total_ms: total_ms.max(1),
                });
                response.selected_keyframe = Some(i);
            } else if let Some(i) = hit_body {
                drag = Some(TimelineDrag {
                    kind: DragKind::MoveBody,
                    index: i,
                    start_duration: keyframes[i].duration_ms,
                    start_pointer_x: pos.x,
                    track_left: track_inner.left(),
                    track_width: track_inner.width(),
                    total_ms: total_ms.max(1),
                });
                response.selected_keyframe = Some(i);
            } else if hit_block.is_none() && track_col.contains(pos) {
                response.playhead_ms = Some(x_to_ms(pos.x, track_inner, total_ms.max(1)));
            }
        }

        if let Some(d) = drag.as_mut() {
            if sense.dragged() {
                d.apply(pos.x, keyframes);
                response.timing_edited = true;
            }
        }
        if sense.drag_stopped() {
            drag = None;
        } else if drag.is_none() && sense.dragged() && hit_block.is_none() && track_col.contains(pos) {
            response.playhead_ms = Some(x_to_ms(pos.x, track_inner, total_ms.max(1)));
        }

        if drag.is_some() || hit_edge.is_some() || hit_body.is_some() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
        } else if sense.hovered() && track_col.contains(pos) {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
        }
        }
    }

    ui.ctx().data_mut(|d| d.insert_temp(drag_id, drag));

    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        ui.label(
            egui::RichText::new(format!("{} keyframes | {total_ms} ms", keyframes.len()))
                .small()
                .color(RULER_TEXT),
        );
        ui.label(
            egui::RichText::new(format!("playhead {playhead_ms} ms"))
                .small()
                .color(RULER_TEXT),
        );
        ui.label(
            egui::RichText::new("drag block = stagger · right edge = trim")
                .small()
                .weak(),
        );
    });

    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use paperdoll_rig::{Easing, KeyframeSpec};

    fn kf(duration_ms: u32) -> KeyframeSpec {
        KeyframeSpec {
            pose: Some("idle".into()),
            joints: None,
            camera: None,
            expressions: None,
            hold: None,
            duration_ms,
            easing: Easing::Linear,
        }
    }

    #[test]
    fn arrival_ms_accumulates_blend_durations() {
        let keyframes = vec![kf(0), kf(400), kf(300), kf(540)];
        assert_eq!(keyframe_arrival_ms(&keyframes, 0), 0);
        assert_eq!(keyframe_arrival_ms(&keyframes, 1), 400);
        assert_eq!(keyframe_arrival_ms(&keyframes, 2), 700);
        assert_eq!(keyframe_arrival_ms(&keyframes, 3), 1240);
    }

    #[test]
    fn set_duration_clamps_minimum() {
        let mut keyframes = vec![kf(0), kf(400)];
        set_keyframe_duration(&mut keyframes, 1, 5);
        assert_eq!(keyframes[1].duration_ms, MIN_BLEND_MS);
    }
}
