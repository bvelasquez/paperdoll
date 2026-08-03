#!/usr/bin/env python3
"""
Apply a hand-preset pose (no authored camera), wait for you to frame the viewport,
then save GET /screenshot and GET /state camera to hand-shots/.

Usage:
  1. Close editor (F2) OR use editor: pose the arm, apply hand preset, orbit camera.
  2. python3 capture_live_camera.py fist
  3. Frame the hand in the paperdoll window, press Enter in the terminal.
"""
import json
import sys
import time
import urllib.request
from pathlib import Path

BASE = "http://127.0.0.1:7878"
OUT = Path(__file__).resolve().parent / "hand-shots"
CAMERA_JSON = Path(__file__).resolve().parent / "hand_camera.json"

# Import preset joint blocks from capture_hand_presets
sys.path.insert(0, str(Path(__file__).resolve().parent))
from capture_hand_presets import ARM, ARM_HIP, PRESETS  # noqa: E402


def get_json(path: str):
    with urllib.request.urlopen(f"{BASE}{path}", timeout=15) as r:
        return json.loads(r.read().decode())


def post(path: str, body: dict):
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
    preset = sys.argv[1] if len(sys.argv) > 1 else "fist"
    context = sys.argv[2] if len(sys.argv) > 2 else "raised"
    if preset not in PRESETS:
        print(f"unknown preset {preset!r}; choose from: {', '.join(PRESETS)}")
        sys.exit(1)
    arm = ARM if context == "raised" else ARM_HIP
    name = f"_live_{preset}_{context}"
    joints = {**arm, **PRESETS[preset]}
    # No `camera` block — viewport stays wherever you put it.
    post("/poses", {"name": name, "joints": joints, "expressions": {}})
    post("/pose", {"name": name})
    print(f"Applied {name}. Frame the hand in the paperdoll window, then press Enter…")
    input()
    state = get_json("/state")
    cam = state.get("camera") or state.get("orbit")
    with urllib.request.urlopen(f"{BASE}/screenshot", timeout=15) as r:
        png = r.read()
    OUT.mkdir(parents=True, exist_ok=True)
    out_png = OUT / f"{preset}_{context}_your_camera.png"
    out_png.write_bytes(png)
    if cam:
        CAMERA_JSON.write_text(json.dumps(cam, indent=2) + "\n")
        print(f"Saved camera → {CAMERA_JSON}")
    print(f"Saved screenshot → {out_png}")


if __name__ == "__main__":
    main()
