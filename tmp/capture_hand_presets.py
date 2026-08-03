#!/usr/bin/env python3
"""Register hand-preset showcase poses and capture GET /screenshot PNGs."""
import json
import time
import urllib.request
from pathlib import Path

BASE = "http://127.0.0.1:7878"
OUT = Path(__file__).resolve().parent / "hand-shots"

# Author-tuned raised right hand (yaw 50°, pitch 4°, distance 1.2).
CAMERA_RAISED_HAND = {
    "yaw_deg": 50.0,
    "pitch_deg": 4.0,
    "distance": 1.20,
    "look_at": [0.33, 1.41, 0.16],
}

# Arm raised, hand toward camera (finger_emote fist keyframe).
ARM = {
    "right_shoulder": {"rotation_deg": {"y": -55.0, "z": -25.0}},
    "right_elbow": {"rotation_deg": {"z": 70.0}},
    "right_wrist": {"rotation_deg": {"z": -15.0}},
    "left_shoulder": {"rotation_deg": {"z": -70.0}},
    "left_elbow": {"rotation_deg": {"z": -12.0}},
}

# Idle right arm on hip (typical editor context).
ARM_HIP = {
    "right_shoulder": {"rotation_deg": {"z": 42.0}},
    "right_elbow": {"rotation_deg": {"z": 95.0}},
    "right_wrist": {"rotation_deg": {"z": -8.0}},
    "left_shoulder": {"rotation_deg": {"z": -78.0}},
    "left_elbow": {"rotation_deg": {"z": -10.0}},
    "left_wrist": {"rotation_deg": {"z": 5.0}},
}

CAMERA_HIP_CLOSE = {
    "yaw_deg": 18.0,
    "pitch_deg": 10.0,
    "distance": 1.05,
    "look_at": [0.26, 1.02, 0.14],
}
CAMERA_HIP_SIDE = {
    "yaw_deg": 38.0,
    "pitch_deg": 6.0,
    "distance": 1.0,
    "look_at": [0.28, 1.00, 0.12],
}


def rot(x=0.0, y=0.0, z=0.0):
    return {"rotation_deg": {"x": x, "y": y, "z": z}}


PRESETS = {
    "fist": {
        "right_thumb_metacarpal": rot(40, -30, 12),
        "right_thumb_proximal": rot(20, -15, 55),
        "right_thumb_distal": rot(0, 0, 50),
        "right_index_proximal": rot(0, 0, 80),
        "right_index_intermediate": rot(0, 0, 90),
        "right_index_distal": rot(0, 0, 70),
        "right_middle_proximal": rot(0, 0, 85),
        "right_middle_intermediate": rot(0, 0, 95),
        "right_middle_distal": rot(0, 0, 70),
        "right_ring_proximal": rot(0, 0, 85),
        "right_ring_intermediate": rot(0, 0, 95),
        "right_ring_distal": rot(0, 0, 70),
        "right_little_proximal": rot(0, 0, 80),
        "right_little_intermediate": rot(0, 0, 90),
        "right_little_distal": rot(0, 0, 70),
    },
    "relaxed": {
        "right_index_proximal": rot(0, 0, 12),
        "right_middle_proximal": rot(0, 0, 14),
        "right_ring_proximal": rot(0, 0, 14),
        "right_little_proximal": rot(0, 0, 10),
    },
    "open": {
        "right_thumb_metacarpal": rot(0, 0, 10),
        "right_thumb_proximal": rot(0, 0, 10),
        "right_index_proximal": rot(0, 0, 0),
        "right_index_intermediate": rot(0, 0, 0),
        "right_middle_proximal": rot(0, 0, 0),
        "right_ring_proximal": rot(0, 0, 0),
        "right_little_proximal": rot(0, 0, 0),
    },
    "point": {
        "right_thumb_metacarpal": rot(18, 0, 38),
        "right_thumb_proximal": rot(0, 0, 45),
        "right_thumb_distal": rot(0, 0, 35),
        "right_index_proximal": rot(0, 0, 5),
        "right_index_intermediate": rot(0, 0, 5),
        "right_index_distal": rot(0, 0, 5),
        "right_middle_proximal": rot(0, 0, 82),
        "right_middle_intermediate": rot(0, 0, 90),
        "right_middle_distal": rot(0, 0, 68),
        "right_ring_proximal": rot(0, 0, 85),
        "right_ring_intermediate": rot(0, 0, 92),
        "right_ring_distal": rot(0, 0, 70),
        "right_little_proximal": rot(0, 0, 80),
        "right_little_intermediate": rot(0, 0, 88),
        "right_little_distal": rot(0, 0, 65),
    },
    "high_five": {
        "right_thumb_metacarpal": rot(0, 0, -15),
        "right_thumb_proximal": rot(0, 0, 5),
        "right_index_proximal": rot(0, 0, 0),
        "right_index_intermediate": rot(0, 0, 0),
        "right_middle_proximal": rot(0, 0, 0),
        "right_middle_intermediate": rot(0, 0, 0),
        "right_ring_proximal": rot(0, 0, 0),
        "right_ring_intermediate": rot(0, 0, 0),
        "right_little_proximal": rot(0, 0, 0),
        "right_little_intermediate": rot(0, 0, 0),
    },
    "peace": {
        "right_thumb_metacarpal": rot(15, 0, 40),
        "right_thumb_proximal": rot(0, 0, 50),
        "right_thumb_distal": rot(0, 0, 40),
        "right_index_proximal": rot(0, 0, 5),
        "right_index_intermediate": rot(0, 0, 5),
        "right_index_distal": rot(0, 0, 5),
        "right_middle_proximal": rot(0, 0, 5),
        "right_middle_intermediate": rot(0, 0, 5),
        "right_middle_distal": rot(0, 0, 5),
        "right_ring_proximal": rot(0, 0, 85),
        "right_ring_intermediate": rot(0, 0, 95),
        "right_ring_distal": rot(0, 0, 70),
        "right_little_proximal": rot(0, 0, 85),
        "right_little_intermediate": rot(0, 0, 95),
        "right_little_distal": rot(0, 0, 70),
    },
}


def post_json(path, body, method="POST"):
    data = json.dumps(body).encode()
    req = urllib.request.Request(
        f"{BASE}{path}",
        data=data,
        headers={"Content-Type": "application/json"},
        method=method,
    )
    with urllib.request.urlopen(req, timeout=30) as r:
        return r.status, r.read()


def register_and_shot(name, joints, camera, out_name):
    pose = {"name": name, "joints": {**joints}, "camera": camera, "expressions": {}}
    status, _ = post_json("/poses", pose)
    print(f"register {name}: {status}")
    post_json("/pose", {"name": name})
    time.sleep(1.2)
    with urllib.request.urlopen(f"{BASE}/screenshot", timeout=15) as r:
        png = r.read()
    path = OUT / out_name
    path.write_bytes(png)
    print(f"  -> {path} ({len(png)} bytes)")


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    for preset, fingers in PRESETS.items():
        register_and_shot(
            f"_shot_{preset}_raised",
            {**ARM, **fingers},
            CAMERA_RAISED_HAND,
            f"{preset}_raised.png",
        )
    for ref in ("fist_pump_right", "hands_on_hips", "point"):
        post_json("/pose", {"name": ref})
        time.sleep(1.2)
        with urllib.request.urlopen(f"{BASE}/screenshot", timeout=15) as r:
            png = r.read()
        (OUT / f"ref_{ref}_fullbody.png").write_bytes(png)
        print(f"ref {ref} -> ref_{ref}_fullbody.png (pose default camera)")


if __name__ == "__main__":
    main()
