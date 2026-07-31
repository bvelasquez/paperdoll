use bevy::math::primitives::ConicalFrustum;
use bevy::prelude::*;

/// Builds a TAPERED bone segment spanning from a parent joint to its child: wide at
/// the parent end, narrow at the child end, like a wooden mannequin's thigh or calf.
///
/// `radius_start` is the thickness at the parent (proximal) end, `radius_end` at the
/// child (distal) end. The segment is authored along +Y with the wide end at -Y and
/// the narrow end at +Y, then rotated/positioned by the caller to span the bone's
/// actual direction. Flat ends are fine because joint-marker spheres cover both tips.
///
/// A uniform-radius bone is just the special case `radius_start == radius_end` (a
/// cylinder), so this fully replaces the old capsule approach.
pub fn bone_segment_mesh(length: f32, radius_start: f32, radius_end: f32) -> ConicalFrustum {
    ConicalFrustum {
        radius_top: radius_end.max(0.008),
        radius_bottom: radius_start.max(0.008),
        height: (length - 0.01).max(0.02),
    }
}

/// A small sphere marking a joint, so the rig hierarchy is visible even at joints
/// with no meaningful "bone" of their own (e.g. wrists, ankles, head).
///
/// Sized from the joint's own `radius` (the thickness of the bone leading away from
/// it) rather than its `length`, so the marker reads as a rounded joint cap roughly
/// matching the adjoining limb's thickness instead of a bulging ball — a knee or hip
/// with a long outgoing bone (thigh, shin) previously got an oversized sphere just
/// because that bone was long, regardless of how thick it actually was.
pub fn joint_marker_mesh(radius: f32) -> Sphere {
    Sphere::new((radius * 1.15).max(0.02))
}

/// A modest sphere representing bust volume for the feminine figure variant. Attached
/// rigidly to the chest joint as cosmetic geometry (see `rig_bridge::spawn_rig`)
/// rather than as its own joint, so it isn't independently posable.
pub fn bust_mesh(radius: f32) -> Sphere {
    Sphere::new(radius)
}

// ── Face feature meshes ───────────────────────────────────────────────────────
// All meshes are unit spheres/capsules scaled non-uniformly at spawn time (see
// `rig_bridge::spawn_rig`) into soft, feminine anime-style features.

/// White of the eye — scaled (1.0, 1.15, 0.35) into a tall, flat anime ellipse.
/// Modest size: big enough to read at portrait distance, small enough not to
/// dominate the face.
pub fn sclera_mesh() -> Sphere {
    Sphere::new(0.032)
}

/// Colored iris — scaled (1.0, 1.12, 0.4); large but leaving a visible white rim
/// of sclera around it (anime eyes show some white, or they read as sunglasses).
pub fn iris_mesh() -> Sphere {
    Sphere::new(0.022)
}

/// Dark pupil core — scaled (1.0, 1.1, 0.5).
pub fn pupil_mesh() -> Sphere {
    Sphere::new(0.012)
}

/// Tiny white sparkle highlight for the eye — the anime "life in the eyes" dot.
pub fn highlight_mesh() -> Sphere {
    Sphere::new(0.0065)
}

/// Eyelid — a soft skin-toned cap scaled (1.05, 0.85, 0.4) that swings down over
/// the eye when the eyelid joint rotates around X.
pub fn eyelid_mesh() -> Sphere {
    Sphere::new(0.036)
}

/// Upper eyeliner — a thin dark capsule lying along the top edge of the eye.
pub fn eyeliner_mesh() -> Capsule3d {
    Capsule3d::new(0.0045, 0.028)
}

/// A single short lash flick for the outer corner of an eye.
pub fn lash_mesh() -> Capsule3d {
    Capsule3d::new(0.003, 0.010)
}

/// Eyebrow — thin, softly arched capsule; feminine brows are thin, not bars.
pub fn eyebrow_mesh() -> Capsule3d {
    Capsule3d::new(0.0045, 0.027)
}

/// Blush oval — scaled (1.2, 0.65, 0.25), tucked inside the head until a pose
/// translates the blush joint out onto the cheek.
pub fn blush_mesh() -> Sphere {
    Sphere::new(0.020)
}

/// Lower lip — scaled (1.5, 0.55, 0.5) into a small soft mouth, rides the jaw joint.
pub fn lip_mesh() -> Sphere {
    Sphere::new(0.017)
}

/// Dark mouth interior revealed when the jaw drops the lip away from it.
pub fn mouth_interior_mesh() -> Sphere {
    Sphere::new(0.014)
}

/// Hair cap — a slightly oversized sphere behind the face that frames it in dark
/// hair, leaving the front (face side) open.
pub fn hair_cap_mesh() -> Sphere {
    Sphere::new(0.115)
}

/// Twin-tail puff — scaled (0.9, 1.3, 0.9) into a hanging side-tail.
pub fn hair_tail_mesh() -> Sphere {
    Sphere::new(0.045)
}

/// Ahoge (cowlick) — the thin sprout on top of every good anime head.
pub fn ahoge_mesh() -> Capsule3d {
    Capsule3d::new(0.004, 0.03)
}
