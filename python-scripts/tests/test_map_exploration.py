from __future__ import annotations

import base64
import json
import subprocess
import sys
import unittest
from pathlib import Path

DIRECTORY = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(DIRECTORY))
from apps.map_exploration import ExplorationPolicy, Settings, guide_routes


def mini(*directions: int) -> bytes:
    rgb = bytearray([40, 40, 40] * 210 * 210)
    for direction in directions:
        for offset in (18, 28, 38, 48):
            for y in range(103, 107):
                for x in range(105 + offset * direction - 2, 105 + offset * direction + 2):
                    at = (y * 210 + x) * 3
                    rgb[at:at + 3] = bytes((80, 230, 30))
    return bytes(rgb)


def request(time_ms: int = 0, **sight: object) -> dict:
    return {"version": 1, "id": 1, "context": {"epoch": "target-1", "stage": "Follow", "target": "observation", "time_ms": time_ms}, "sight": {"fresh": True, "world_controls": True, "heading": 0.0, **sight}}


class ExplorationTests(unittest.TestCase):
    def test_green_branch_and_missing_line(self) -> None:
        self.assertEqual(len(guide_routes(mini(1), Settings())), 1)
        self.assertEqual(len(guide_routes(mini(-1, 1), Settings())), 2)
        self.assertEqual(guide_routes(mini(), Settings()), [])

    def test_bad_line_remains_ignored_when_one_branch_disappears(self) -> None:
        policy = ExplorationPolicy(Settings())
        frame = request(mini_rgb_b64=base64.b64encode(mini(-1, 1)).decode(), world_marker={"meters": 22.8, "point": [480, 450]})
        self.assertEqual(policy.decide(frame)["action"], "marker")
        frame["sight"]["mini_rgb_b64"] = base64.b64encode(mini(1)).decode()
        self.assertEqual(policy.decide(frame)["action"], "marker")
        frame["context"]["epoch"] = "target-2"
        self.assertEqual(policy.decide(frame)["action"], "guide")

    def test_progress_stall_and_tracking_recovery(self) -> None:
        policy = ExplorationPolicy(Settings())
        frame = request(world_marker={"meters": 22.8, "point": [960, 450]})
        self.assertEqual(policy.decide(frame)["action"], "marker")
        frame["context"]["time_ms"] = 3500
        frame["sight"]["world_marker"]["meters"] = 21.0
        self.assertEqual(policy.decide(frame)["action"], "marker")
        frame["context"]["time_ms"] = 7499
        self.assertEqual(policy.decide(frame)["action"], "marker")
        frame["context"]["time_ms"] = 7500
        self.assertEqual(policy.decide(frame)["action"], "recover")
        frame["context"]["epoch"] = "target-1-retracked"
        self.assertEqual(policy.decide(frame)["action"], "marker")

    def test_arrival_facing_and_interaction_require_separate_proof(self) -> None:
        policy = ExplorationPolicy(Settings())
        frame = request(arrival_point=[1800, 480])
        self.assertEqual(policy.decide(frame)["action"], "arrive")
        frame["context"]["stage"] = "ApproachTarget"
        self.assertEqual(policy.decide(frame)["action"], "face")
        frame["sight"]["interact"] = True
        self.assertEqual(policy.decide(frame)["action"], "interact")
        frame["sight"]["fresh"] = False
        self.assertEqual(policy.decide(frame)["action"], "wait")

    def test_road_needs_heading_and_cargo_does_not_use_observation_marker(self) -> None:
        policy = ExplorationPolicy(Settings())
        frame = request(road_route={"remaining": 60.0}, world_marker={"meters": 20, "point": [960, 450]})
        self.assertEqual(policy.decide(frame)["action"], "road")
        frame["sight"].pop("road_route")
        frame["context"]["target"] = "cargo"
        self.assertEqual(policy.decide(frame)["action"], "recover")
        frame["sight"]["road_route"] = {"remaining": 60}
        frame["sight"]["heading"] = None
        self.assertEqual(policy.decide(frame)["action"], "recover")

    def test_settings_fail_on_typo_nan_or_unbounded_limits(self) -> None:
        for values in ({"typo": 1}, {"green_min": True}, {"branch_angle": float("nan")}, {"no_progress_ms": 10_000}, {"marker_max_meters": 200}):
            with self.subTest(values=values), self.assertRaises(ValueError):
                Settings.load(values)

    def test_worker_uses_utf8_json_and_echoes_frame_id(self) -> None:
        frame = request(arrival_point=[960, 450])
        frame["id"] = 42
        result = subprocess.run([sys.executable, "-B", str(DIRECTORY / "run.py"), "worker"], input=json.dumps(frame) + "\n", capture_output=True, encoding="utf-8", check=True)
        hello, response = [json.loads(line) for line in result.stdout.splitlines()]
        self.assertTrue(hello["ready"])
        self.assertEqual(response["id"], 42)
        self.assertEqual(response["action"], "arrive")
        self.assertIn("到点", response["reason"])

    def test_live_requires_explicit_input_without_starting_engine(self) -> None:
        result = subprocess.run([sys.executable, "-B", str(DIRECTORY / "run.py"), "live", "--hwnd", "invalid"], capture_output=True, encoding="utf-8")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("--allow-input", result.stderr)

    def test_worker_error_contains_python_source_and_frame_id(self) -> None:
        frame=request(mini_rgb_b64="AAAA")
        result=subprocess.run([sys.executable,"-B",str(DIRECTORY/"run.py"),"worker"],input=json.dumps(frame)+"\n",capture_output=True,encoding="utf-8")
        self.assertNotEqual(result.returncode,0)
        response=json.loads(result.stdout.splitlines()[-1])
        self.assertEqual(response["id"],1)
        self.assertIn("map_exploration.py",response["error"])


if __name__ == "__main__":
    unittest.main()
