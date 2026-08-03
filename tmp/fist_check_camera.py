#!/usr/bin/env python3
"""Quick fist screenshot with hand-focused cameras."""
import json, time, urllib.request
from pathlib import Path

BASE = "http://127.0.0.1:7878"
OUT = Path(__file__).resolve().parent / "hand-shots"
from capture_hand_presets import ARM, PRESETS, CAMERA_RAISED_CLOSE, CAMERA_RAISED_SIDE, post_json

def main():
    joints = {**ARM, **PRESETS["fist"]}
    for label, cam in [("close", CAMERA_RAISED_CLOSE), ("side", CAMERA_RAISED_SIDE)]:
        name = f"_fist_check_{label}"
        post_json("/poses", {"name": name, "joints": joints, "camera": cam, "expressions": {}})
        post_json("/pose", {"name": name})
        time.sleep(1.1)
        with urllib.request.urlopen(f"{BASE}/screenshot", timeout=15) as r:
            (OUT / f"fist_check_{label}.png").write_bytes(r.read())
        print(label)

if __name__ == "__main__":
    main()
