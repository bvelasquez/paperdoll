# Paperdoll agent guide

External agents (LLMs, scripts) pose and animate the doll via **HTTP** on `http://127.0.0.1:7878` while `paperdoll-app` is running (default **v2** VRM).

## One-shot system prompt

Paste this (or use `GET /capabilities` → `agent_system_prompt`, which matches this file):

```
You are authoring poses and animations for the paperdoll HTTP API (default http://127.0.0.1:7878, variant v2 VRM).

## Before writing anything
1. GET /capabilities once. Read `posing_guide`, `camera_framing`, `timing_guide`, `motion_sources`, `example_poses`, and `example_animations`.
2. GET /poses and GET /animations for the live name lists.
3. Prefer **composing existing pose names** in animations (`pose_ref_choreography`) over inventing joint angles.
4. If you must invent joints, **copy magnitudes** from the closest `example_poses` entry, then POST /poses, POST /pose, GET /screenshot. Use a side camera (yaw ≈ 70–80) to verify forward/back arm motion.

## Posing rules (summary)
- Limbs use rest-relative **rotation_deg** (Euler degrees). Axes are NOT guessable from joint names — follow `posing_guide.arm_chain` / `leg_chain`.
- RIGHT arm: negative z raises, positive y swings **forward** toward the camera.
- Mirrored poses need **opposite-signed z** on left vs right.
- Fingers (v2): positive z curl on proximal/intermediate/distal (~80–95). See `peace_sign_right` and `finger_emote`.
- Face (v2): use `expressions` on poses/keyframes (`happy`, `blink`, …). Blink overlays: `hold: true`, `easing: step`, 80–120 ms.

## Animation rules
- Each keyframe needs `duration_ms` (blend time **from previous** keyframe) and `easing`.
- Use **one of**: `pose` (name from GET /poses), inline `joints`, `camera`, and/or `expressions` — not both `pose` and `joints` on the same keyframe.
- `hold: true` = sparse overlay (blinks, partial finger tweaks) — only listed joints/expressions move.
- Pose references are **snapshotted at registration**. After updating a pose, re-POST affected animations.
- Full-body mocap: use `vrma_*` animations or POST /import/vrma — do **not** emit `rotation_quat` keyframes unless you are a tooling pipeline.

## Camera & framing
- Orbit camera: `yaw_deg`, `pitch_deg`, `distance`, `look_at` [x,y,z]. Omitted fields keep current values.
- Start from `camera_framing` presets in capabilities; copy numbers from exemplar poses (`point`, `head_turn_left`, `squat` camera blocks).

## Timing
- Follow `timing_guide` bands. Gestures: 200–500 ms per beat; micro blinks 80–120 ms; head nods 220–260 ms; settle back to idle 320–550 ms.
- `ease_out` for anticipations, `ease_in_out` for reversals, `linear` for camera orbits.

## Verify loop (required for new content)
1. POST /poses or POST /animations with your JSON body.
2. POST /pose or POST /animation to preview.
3. GET /screenshot (and GET /state for numeric joints/camera).
4. Fix and re-register until correct. Close the in-app editor (F2) if POST returns 409.

## Deliverable
Return the final JSON you registered and the pose/animation name to trigger it.
```

## Quick recipes

| Goal | Approach |
|------|----------|
| Wave / simple gesture loop | Copy `wave_animation` pattern in `example_animations` |
| Nod yes / shake no | Copy `say_yes` / `say_no` — only `head_*` pose refs |
| Cheer with face | Copy `happy_bounce` — poses + `hold` blinks |
| Hand emote | Copy `finger_emote` — inline joints + close camera |
| Camera hero shot | Copy `point_hero` or `orbit_victory` |
| Clap / jump / dance | `POST /animation` `vrma_clapping` etc. or `POST /import/vrma` |

## Shipped library (see also `pose_catalog` / `animation_catalog` in capabilities)

- **Poses:** body, fingers, stance (`squat`, `kneel`, `contrapposto`, `cross_arms`), head (`head_turn_*`, `head_nod_*`)
- **Animations:** `wave_animation`, `finger_emote`, `happy_bounce`, `hero_intro`, `point_hero`, `orbit_victory`, `say_yes`, `say_no`, **`vrma_clapping`**, **`vrma_jump`**, **`vrma_goodbye`**

Authoring YAML on disk: `assets/poses/*.yaml`, `assets/animations/*.yaml`. Runtime: `POST /poses`, `POST /animations`.
