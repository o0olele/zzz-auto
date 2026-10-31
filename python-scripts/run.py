"""Python 开发入口：截图检查、决策回放、真实游戏测试与 Rust 工作进程。"""
from __future__ import annotations

import argparse
import json
import subprocess
import sys
import traceback
from pathlib import Path
from typing import Any

from apps.map_exploration import ExplorationPolicy, Settings

DIRECTORY = Path(__file__).resolve().parent
ROOT = DIRECTORY.parent
VERSION = 1
MAX_REQUEST = 512 * 1024


def load_policy(settings_path: Path) -> ExplorationPolicy:
    return ExplorationPolicy(Settings.load(json.loads(settings_path.read_text(encoding="utf-8"))))


def emit(value: dict[str, Any]) -> None:
    print(json.dumps(value, ensure_ascii=False, allow_nan=False), flush=True)


def worker(settings_path: Path) -> int:
    policy = load_policy(settings_path)
    emit({"version": VERSION, "ready": True, "app": "map_exploration"})
    while line := sys.stdin.buffer.readline(MAX_REQUEST + 1):
        if len(line) > MAX_REQUEST or not line.endswith(b"\n"):
            raise ValueError("脚本请求超长或不完整")
        request = json.loads(line)
        if request.get("version") != VERSION or not isinstance(request.get("id"), int):
            raise ValueError("脚本协议不匹配")
        try:
            emit({"version": VERSION, "id": request["id"], **policy.decide(request)})
        except Exception:
            emit({"version": VERSION, "id": request["id"], "error": traceback.format_exc(limit=8)})
            return 1
    return 0


def require_engine(path: Path) -> Path:
    executable = path.resolve()
    if not executable.is_file():
        raise ValueError("缺少开发版引擎；首次运行请执行 cargo build --locked -p zzz-engine。脚本运行不会自动调用 Cargo。")
    return executable


def inspect(args: argparse.Namespace) -> int:
    command = [str(require_engine(args.engine)), "--root", str(ROOT), "map-exploration-inspect", "--bundle", str(ROOT / "resources/generated/world-patrol"), "--image", str(args.image.resolve()), "--target", args.target, "--python-observation"]
    result = subprocess.run(command, cwd=ROOT, capture_output=True, encoding="utf-8", check=False)
    if result.returncode:
        raise ValueError(result.stderr.strip() or "底层截图识别失败")
    sight = json.loads(result.stdout)
    request = {"version": VERSION, "id": 1, "context": {"epoch": "inspect", "stage": "Follow", "target": args.target, "time_ms": 0}, "sight": sight}
    response = {"version": VERSION, "id": 1, **load_policy(args.settings).decide(request)}
    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(json.dumps({"request": request, "response": response}, ensure_ascii=False) + "\n", encoding="utf-8")
    emit({"image": str(args.image), "response": response, "native": {key: value for key, value in sight.items() if key != "mini_rgb_b64"}})
    return 0


def replay(args: argparse.Namespace) -> int:
    policy = load_policy(args.settings)
    count = matched = 0
    with args.trace.open(encoding="utf-8") as stream:
        for line in stream:
            frame = json.loads(line)
            request = frame["request"]
            response = policy.decide(request)
            expected = frame.get("response", {})
            equal = all(response.get(key) == expected.get(key) for key in ("action", "guide"))
            count += 1
            matched += int(equal)
            emit({"id": request["id"], "response": response, "matches_recording": equal})
    emit({"frames": count, "matched": matched})
    return int(args.check and matched != count)


def live(args: argparse.Namespace) -> int:
    if not args.allow_input:
        raise ValueError("游戏实测需要显式提供 --allow-input；只读检查使用 inspect 或 replay。")
    command = [str(require_engine(args.engine)), "--root", str(ROOT), "--data", str(args.data.resolve()), "map-exploration-dev", "--hwnd", args.hwnd, "--instance-id", str(args.instance_id), "--group-id", args.group_id, "--cancel-after-ms", str(args.duration_ms), "--allow-input"]
    # 继承控制台的 Ctrl+C，由 Rust 收到后停止任务并确认输入释放。
    process = subprocess.Popen(command, cwd=ROOT)
    while True:
        try:
            return process.wait()
        except KeyboardInterrupt:
            print("正在等待 Rust 停止任务并释放输入……", file=sys.stderr, flush=True)


def main() -> int:
    if hasattr(sys.stdout, "reconfigure"):
        sys.stdout.reconfigure(encoding="utf-8")
        sys.stderr.reconfigure(encoding="utf-8")
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    for name in ("worker", "inspect", "replay"):
        command = commands.add_parser(name)
        command.add_argument("--settings", type=Path, default=DIRECTORY / "configs/map_exploration.json")
        if name == "inspect":
            command.add_argument("--image", required=True, type=Path)
            command.add_argument("--target", choices=("observation", "cargo"), default="observation")
            command.add_argument("--engine", type=Path, default=ROOT / "target/debug/zzz-engine.exe")
            command.add_argument("--output", type=Path)
        elif name == "replay":
            command.add_argument("--trace", required=True, type=Path)
            command.add_argument("--check", action="store_true")
    command = commands.add_parser("live")
    command.add_argument("--engine", type=Path, default=ROOT / "target/debug/zzz-engine.exe")
    command.add_argument("--hwnd", required=True)
    command.add_argument("--data", type=Path, default=ROOT / "data")
    command.add_argument("--instance-id", type=int, default=1)
    command.add_argument("--group-id", default="one_dragon")
    command.add_argument("--duration-ms", type=int, default=60_000)
    command.add_argument("--allow-input", action="store_true")
    args = parser.parse_args()
    if args.command == "worker":
        return worker(args.settings.resolve())
    if args.command == "inspect":
        return inspect(args)
    if args.command == "replay":
        return replay(args)
    return live(args)


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (ValueError, OSError, KeyError, TypeError) as error:
        if "worker" in sys.argv:
            emit({"version": VERSION, "ready": False, "error": str(error)})
        print(f"Python 开发测试失败：{error}", file=sys.stderr)
        raise SystemExit(1)
