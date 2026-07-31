# paperdoll

An experiment in a Rust-based "paper-doll" character: a skinnable, 3D humanoid
skeleton that can be posed and animated via YAML-defined targets, with smooth
interpolation between poses, ultimately driven by an HTTP API so an external
agent (human or AI) can pose the character on demand.

## Workspace layout

- **`crates/paperdoll-rig`** — renderer-agnostic core: joint hierarchy
  (`Skeleton`), pose/animation data model (`Pose`, `Animation`), YAML loading,
  and the interpolation engine (`PlaybackState`) that drives smooth,
  interruptible transitions between poses. No Bevy dependency, so it's fast
  to unit test and reusable outside a graphical app.
- **`crates/paperdoll-app`** — a Bevy binary that renders the rig as a
  procedural placeholder humanoid (capsules per bone, spheres per joint) and
  will host the HTTP pose API.
- **`assets/poses/*.yaml`**, **`assets/animations/*.yaml`** — example pose
  and animation definitions (`t_pose`, `wave`/`wave_return`/`wave_animation`,
  `idle`, `hands_on_hips`, `victory`, `think`, `shrug`, `reach`, `bow`,
  `point`, plus choreography: `hero_intro`, `orbit_victory`, `showcase`,
  `point_hero`).

## Status

Implemented so far (see `paperdoll-rig`'s ~20 unit/integration tests):

- [x] Humanoid joint hierarchy (`Skeleton::humanoid_default`, 24 joints,
      human-proportioned — three-segment spine, clavicle-driven shoulder
      girdle, hands and toes as their own joints)
- [x] YAML pose/animation schema + loader with fail-fast validation
- [x] Quaternion slerp + easing interpolation engine, with smooth
      mid-transition interrupts (no snapping)
- [x] Bevy app rendering the rig as a procedural doll (capsule bones sized
      per joint, not derived from bone length, so torso/limb thickness
      reads as anatomically distinct)
- [x] Loading `assets/poses/*.yaml` and `assets/animations/*.yaml` at
      startup; the doll opens in the default `idle` pose
- [x] Live interpolation driven by the Bevy `Update` schedule, with
      transition duration derived from each pose's angular delta and a
      configurable `TransitionSpeed` (deg/sec) rather than one fixed speed
- [x] HTTP API (axum, on its own thread, bridged into the Bevy ECS via a
      `crossbeam_channel`) — see `crates/paperdoll-app/src/http_api.rs`:
      - `POST /pose`, `POST /animation` — trigger a loaded pose/animation
      - `GET /poses`, `GET /animations` — list what's currently loaded
      - `POST /poses`, `POST /animations` — register a brand new pose or
        animation at runtime (validated the same way a YAML file would be),
        immediately triggerable and immediately referenceable by name from
        a newly-registered animation's keyframes
      - `GET /capabilities` — self-describing doc: every endpoint, the
        pose/animation JSON shape, valid joint names, valid easing values,
        a **posing guide** (which rotation axis actually moves a given
        joint chain — not guessable from joint names alone, see "Posing
        conventions" below), and a small **pose board** of worked examples
        (`example_poses`). The goal is an LLM/agent can `GET /capabilities`
        once and pose the rig correctly without ever reading this repo or
        rediscovering the axis conventions by trial and error.
      - `GET /screenshot` — captures the primary window (Bevy's built-in
        screenshot API) and returns it as a PNG, so a caller can verify a
        pose visually without needing eyes on the actual window
- [x] `idle` default pose (`assets/poses/idle.yaml`) + `IdleRevert`: the rig
      automatically transitions back to it after `TIMEOUT_SECS` (10s) of no
      new `POST /pose`/`POST /animation` commands, so it doesn't stay frozen
      in whatever was last requested
- [x] `GET /state` — live joint + camera + playback-mode snapshot for agents
- [x] Camera choreography (orbit yaw/pitch/distance/look_at) on poses and
      animation keyframes, including camera-only holds

Not yet implemented (see the milestone list below):

- [ ] Real mesh skinning (currently a procedural primitive doll, not a
      skinned mesh)

## Running

```sh
cargo test --workspace   # rig logic + YAML asset validation
cargo run -p paperdoll-app   # opens a window showing the doll in the idle pose

# Or build + install to ~/.local (binary `paperdoll`, assets under share/):
make install
paperdoll
```

With the app running:

```sh
# Discover the whole API in one call — start here if you're an agent/script
# that's never talked to this server before.
curl http://127.0.0.1:7878/capabilities

# Trigger something already loaded from assets/.
curl -X POST http://127.0.0.1:7878/pose -H 'Content-Type: application/json' \
  -d '{"name": "wave", "speed_deg_per_sec": 120}'
curl -X POST http://127.0.0.1:7878/animation -H 'Content-Type: application/json' \
  -d '{"name": "wave_animation"}'
curl http://127.0.0.1:7878/poses
curl http://127.0.0.1:7878/animations

# Register something new, then trigger it — no server restart needed.
curl -X POST http://127.0.0.1:7878/poses -H 'Content-Type: application/json' -d '{
  "name": "salute",
  "joints": { "right_shoulder": {"rotation_deg": {"z": 60}},
              "right_elbow": {"rotation_deg": {"x": -20, "z": 100}} }
}'
curl -X POST http://127.0.0.1:7878/pose -d '{"name": "salute"}'

# See what the rig actually looks like right now.
curl http://127.0.0.1:7878/screenshot -o now.png
```

`paperdoll-app` needs a windowing system and a GPU (or software Vulkan/GL
driver) to run — see [Bevy's Linux dependencies
doc](https://github.com/bevyengine/bevy/blob/latest/docs/linux_dependencies.md)
if `cargo run` fails to find a display or GPU on Linux.

## Posing conventions

Which rotation axis moves a given joint isn't guessable from the joint name —
it depends on which direction that joint's rest offset points. The full
version of this lives in `GET /capabilities`'s `posing_guide` (plus a small
pose board of worked examples in `example_poses`); short version:

- The skeleton stands in a T-pose at rest: arms out to the sides (along local
  X), legs straight down (along local Y). `left_*`/`right_*` joints are
  mirror images of each other.
- For any limb joint, rotating around the axis **parallel** to its own rest
  offset is a no-op (a roll around the limb's own length). Rotating around a
  **perpendicular** axis swings it.
  - Arm chain (shoulder/elbow/wrist/hand, offset along X): `x` = no-op,
    `z` = raise/lower the limb (the useful axis), `y` = forward/back swing.
  - **Sign from T-pose (arms):** right arm — **negative** `z` raises overhead,
    **positive** `z` lowers toward the hip. Left arm is mirrored (positive
    raises, negative lowers). So `victory` is `right.z=-80`, `left.z=+80`.
  - Leg chain (hip/knee/ankle, offset along Y): `y` = no-op, `x` = swing
    forward/back (the useful axis), `z` = sideways swing (unverified).
- Mirrored pairs need **opposite-signed** `z` to produce the same visual
  pose on both sides (e.g. `hands_on_hips`'s `right_shoulder.z = 45` pairs
  with `left_shoulder.z = -45`).
- Sparse poses omit unchanged joints; those joints blend back to rest. Empty
  `joints` (`t_pose`) fully resets. Prefer `spine`/`chest` `x` for a bow —
  `pelvis` `x` tips the legs too.

These were derived empirically against the running rig (`POST /poses` +
`POST /pose` + `GET /screenshot`, not from reasoning about the rotation math
in the abstract) — trust the guide over intuition when authoring a new pose.

## Milestones

| # | Scope |
|---|---|
| M1 | `paperdoll-rig` core: skeleton, pose/animation types, interpolation engine, unit tests — **done** |
| M2 | Bevy app renders a static procedural doll from the skeleton — **done** |
| M3 | Load YAML poses at startup, apply one on launch — **done** |
| M4 | Wire live interpolation into the `Update` schedule — **done** |
| M5 | HTTP API (axum) bridged into the Bevy ECS via a channel — **done** |
| M6 | Full animation sequence playback over the API — **done** (folded into M5; `POST /animation` plays any loaded sequence) |
| M6.5 | Runtime pose/animation registration (`POST /poses`/`POST /animations`) + `GET /capabilities` — **done** |
| M6.6 | `GET /screenshot` + default idle pose with auto-revert-on-timeout — **done** |
| M6.7 | `posing_guide` + `example_poses` pose board in `GET /capabilities`, so the API is self-teaching — **done** |
| M6.8 | `GET /state` live joint/camera snapshot — **done** |
| M7 | Camera choreography (orbit yaw/pitch/distance/look_at on poses + keyframes) — **done** |
| M8 | Real glTF-rigged mesh + linear-blend skinning (stretch) |
