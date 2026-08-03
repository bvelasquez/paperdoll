#!/usr/bin/env python3
"""Try thumb variants for fist; save screenshots for visual pick."""
import json
import time
import urllib.request
from pathlib import Path

BASE = "http://127.0.0.1:7878"
OUT = Path(__file__).resolve().parent / "hand-shots" / "fist_thumb_tuning"

ARM = {
    "right_shoulder": {"rotation_deg": {"y": -55.0, "z": -25.0}},
    "right_elbow": {"rotation_deg": {"z": 70.0}},
    "right_wrist": {"rotation_deg": {"z": -15.0}},
    "left_shoulder": {"rotation_deg": {"z": -70.0}},
    "left_elbow": {"rotation_deg": {"z": -12.0}},
}
CAMERA_RAISED_CLOSE = {
    "yaw_deg": 14.0,
    "pitch_deg": 8.0,
    "distance": 0.98,
    "look_at": [0.30, 1.43, 0.14],
}
CAMERA_RAISED_SIDE = {
    "yaw_deg": 32.0,
    "pitch_deg": 4.0,
    "distance": 0.92,
    "look_at": [0.33, 1.41, 0.16],
}

FINGERS = {
    "right_index_proximal": {"rotation_deg": {"z": 80}},
    "right_index_intermediate": {"rotation_deg": {"z": 90}},
    "right_index_distal": {"rotation_deg": {"z": 70}},
    "right_middle_proximal": {"rotation_deg": {"z": 85}},
    "right_middle_intermediate": {"rotation_deg": {"z": 95}},
    "right_middle_distal": {"rotation_deg": {"z": 70}},
    "right_ring_proximal": {"rotation_deg": {"z": 85}},
    "right_ring_intermediate": {"rotation_deg": {"z": 95}},
    "right_ring_distal": {"rotation_deg": {"z": 70}},
    "right_little_proximal": {"rotation_deg": {"z": 80}},
    "right_little_intermediate": {"rotation_deg": {"z": 90}},
    "right_little_distal": {"rotation_deg": {"z": 70}},
}

# (label, metacarpal xyz, proximal xyz, distal xyz)
THUMBS = [
    ("current", (20, 0, 35), (0, 0, 40), (0, 0, 35)),
    ("idle_soft", (12, 0, 28), (0, 0, 30), (0, 0, 25)),
    ("wrap_x40_yneg", (40, -30, 12), (20, -15, 55), (0, 0, 50)),
    ("wrap_x45_ypos", (45, 25, 8), (18, 10, 58), (5, 0, 45)),
    ("wrap_meta_y", (22, 35, 18), (12, 20, 52), (0, 0, 48)),
    ("wrap_low_z", (30, -20, 5), (25, 0, 65), (10, 0, 55)),
    ("wrap_neg_z_meta", (28, 15, -8), (15, 25, 50), (0, 5, 42)),
    ("wrap_heavy_x", (55, -35, 20), (30, -20, 60), (8, 0, 52)),
    ("hips_style", (15, 0, 30), (0, 0, 32), (0, 0, 28)),
    ("curl_all_xyz", (35, -15, 22), (22, 18, 48), (12, 8, 40)),
]


def rot(x, y, z):
    return {"rotation_deg": {"x": x, "y": y, "z": z}}


def post(path, body):
    data = json.dumps(body).encode()
    req = urllib.request.Request(
        f"{BASE}{path}",
        data=data,
        headers={"Content-Type": "application/json"},
        method="POST",
    )
    with urllib.request.urlopen(req, timeout=30) as r:
        return r.status


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    for label, m, p, d in THUMBS:
        joints = {
            **ARM,
            **FINGERS,
            "right_thumb_metacarpal": rot(*m),
            "right_thumb_proximal": rot(*p),
            "right_thumb_distal": rot(*d),
        }
        name = f"_thumb_tune_{label}"
        time.sleep(1.0)
        for cam_label, cam in [
            ("close", CAMERA_RAISED_CLOSE),
            ("side", CAMERA_RAISED_SIDE),
        ]:
            post("/poses", {"name": f"{name}_{cam_label}", "joints": joints, "camera": cam, "expressions": {}})
            post("/pose", {"name": f"{name}_{cam_label}"})
            time.sleep(1.0)
            with urllib.request.urlopen(f"{BASE}/screenshot", timeout=15) as r:
                (OUT / f"{label}_{cam_label}.png").write_bytes(r.read())
        print(label)


if __name__ == "__main__":
    main()
