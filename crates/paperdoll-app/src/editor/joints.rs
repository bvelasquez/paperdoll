//! Joint name groupings for the pose editor tree.

pub const GROUPS: &[(&str, &[&str])] = &[
    (
        "Spine",
        &["pelvis", "spine", "chest", "upper_chest", "neck", "head", "jaw"],
    ),
    (
        "Arms",
        &[
            "left_shoulder",
            "left_elbow",
            "left_wrist",
            "right_shoulder",
            "right_elbow",
            "right_wrist",
        ],
    ),
    (
        "Legs",
        &[
            "left_hip",
            "left_knee",
            "left_ankle",
            "right_hip",
            "right_knee",
            "right_ankle",
        ],
    ),
    ("Face", &["left_eye", "right_eye", "left_pupil", "right_pupil"]),
    (
        "Left hand",
        &[
            "left_thumb_metacarpal",
            "left_thumb_proximal",
            "left_thumb_distal",
            "left_index_proximal",
            "left_index_intermediate",
            "left_index_distal",
            "left_middle_proximal",
            "left_middle_intermediate",
            "left_middle_distal",
            "left_ring_proximal",
            "left_ring_intermediate",
            "left_ring_distal",
            "left_little_proximal",
            "left_little_intermediate",
            "left_little_distal",
        ],
    ),
    (
        "Right hand",
        &[
            "right_thumb_metacarpal",
            "right_thumb_proximal",
            "right_thumb_distal",
            "right_index_proximal",
            "right_index_intermediate",
            "right_index_distal",
            "right_middle_proximal",
            "right_middle_intermediate",
            "right_middle_distal",
            "right_ring_proximal",
            "right_ring_intermediate",
            "right_ring_distal",
            "right_little_proximal",
            "right_little_intermediate",
            "right_little_distal",
        ],
    ),
];

pub fn joint_matches_filter(name: &str, filter: &str) -> bool {
    let f = filter.trim().to_ascii_lowercase();
    f.is_empty() || name.to_ascii_lowercase().contains(&f)
}
