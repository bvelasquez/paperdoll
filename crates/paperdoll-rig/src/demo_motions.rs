//! Curated free `.vrma` demos (VRoid / vrm-viewer samples) for fetch + import smoke tests.

/// Remote base for demo files (tk256ailab/vrm-viewer, MIT-friendly samples).
pub const DEMO_VRMA_BASE_URL: &str =
    "https://raw.githubusercontent.com/tk256ailab/vrm-viewer/main/VRMA/";

#[derive(Debug, Clone, Copy)]
pub struct DemoMotion {
    pub file_name: &'static str,
    pub animation_name: &'static str,
    pub sample_interval_ms: u32,
}

pub const DEMO_MOTIONS: &[DemoMotion] = &[
    DemoMotion {
        file_name: "Clapping.vrma",
        animation_name: "vrma_clapping",
        sample_interval_ms: 120,
    },
    DemoMotion {
        file_name: "Jump.vrma",
        animation_name: "vrma_jump",
        sample_interval_ms: 100,
    },
    DemoMotion {
        file_name: "Goodbye.vrma",
        animation_name: "vrma_goodbye",
        sample_interval_ms: 120,
    },
];
