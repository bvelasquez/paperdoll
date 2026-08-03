//! Short axis hints for the pose editor (subset of `GET /capabilities` posing_guide).

pub fn hint_for_joint(name: &str) -> Option<&'static str> {
    match name {
        "right_shoulder" | "left_shoulder" => Some(
            "z raises/lowers the arm from T-pose; y twists the upper arm (palm facing).",
        ),
        "right_elbow" | "left_elbow" => Some("z bends the forearm (negative z ≈ fold up)."),
        "right_wrist" | "left_wrist" => Some("z tilts the hand; y rolls the wrist."),
        "head" => Some("y turns left/right; x nods up/down; z tilts ear to shoulder."),
        "neck" => Some("x/y/z for subtle head carriage — prefer `head` for big moves."),
        "spine" | "chest" | "upper_chest" => Some("x bends forward/back; y/z add torso twist."),
        "pelvis" => Some("y rotates the hips; x shifts weight forward/back."),
        "right_hip" | "left_hip" => Some("z swings the leg; x lifts knee forward."),
        "right_knee" | "left_knee" => Some("z bends the knee (negative z ≈ flex)."),
        _ if name.contains("_thumb_") || name.contains("_index_") => {
            Some("finger chains: z curls the digit; x/y fan the phalanges.")
        }
        _ => None,
    }
}
