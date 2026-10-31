"""Summarize a Rust combat JSONL trace without treating captures as observations."""
import argparse
from collections import Counter, defaultdict
import json
import math
from pathlib import Path


def distribution(values):
    values = sorted(values)
    if not values:
        return {"count": 0}
    return {
        "count": len(values),
        "mean_ms": sum(values) / len(values) / 1000,
        "p50_ms": values[math.ceil(len(values) * 0.50) - 1] / 1000,
        "p95_ms": values[math.ceil(len(values) * 0.95) - 1] / 1000,
        "p99_ms": values[math.ceil(len(values) * 0.99) - 1] / 1000,
        "max_ms": values[-1] / 1000,
    }


def analyze(path):
    metrics = defaultdict(list)
    stage_calls = Counter()
    counts = Counter()
    deferred = Counter()
    loop_iterations = 0
    starts = defaultdict(list)
    session = footer = final_state = None
    invalid_lines = 0
    last_at = 0
    active_start = None
    active_us = 0
    segment = 0
    for text in path.open(encoding="utf-8"):
        try:
            event = json.loads(text)
        except json.JSONDecodeError:
            invalid_lines += 1
            continue
        kind, at, data = event["event"], event["at_us"], event["data"]
        last_at = max(last_at, at)
        counts[kind] += 1
        if kind == "session":
            session = data
        elif kind == "footer":
            footer = data
        elif kind == "loop_start":
            active_start = at
            segment += 1
        elif kind == "run_state":
            final_state = data
            if active_start is not None:
                active_us += at - active_start
                active_start = None
        elif kind == "loop_window":
            loop_iterations += data["iterations"]
            deferred.update(data["capture_deferred_iterations"])
        for key, value in data.items():
            if key.endswith("_us") and isinstance(value, (float, int)) and key != "started_us":
                metrics[f"{kind}.{key}"].append(value)
        if kind in ("capture", "recognition_start"):
            starts[kind].append((segment, data["frame_id"]))
        if kind == "capture":
            for key, value in data["timing"].items():
                metrics[f"capture.{key}"].append(value)
        if kind == "recognition_complete":
            for stage, timing in (data.get("stages") or {}).items():
                metrics[f"stage.{stage}"].append(timing["total_us"])
                stage_calls[stage] += timing["calls"]
        if kind == "recognition_consumed":
            counts["accepted_epoch" if data["accepted_epoch"] else "discarded_epoch"] += 1
    if active_start is not None:
        active_us += max(0, last_at - active_start)
    rates = {}
    for kind, times in starts.items():
        # Capture start is the common time boundary. Do not bridge pauses/resumes.
        times.sort()
        metrics[f"{kind}.capture_start_interval_us"] = [b[1] - a[1] for a, b in zip(times, times[1:]) if a[0] == b[0]]
        rates[f"{kind}_per_active_second"] = len(times) * 1_000_000 / active_us if active_us else None
    rates["accepted_observations_per_active_second"] = counts["accepted_epoch"] * 1_000_000 / active_us if active_us else None
    if final_state and active_us:
        rates["processed_observations_per_active_second"] = final_state["progress"]["frames"] * 1_000_000 / active_us
    loop_us = sum(metrics["loop_window.duration_us"])
    return {
        "file": str(path.resolve()), "session": session, "counts": dict(counts),
        "active_seconds_including_start_delay_and_audio_setup": active_us / 1_000_000,
        "rates": rates, "timings": {name: distribution(values) for name, values in sorted(metrics.items())},
        "stage_calls": dict(stage_calls), "final_state": final_state, "footer": footer,
        "loop_summary": {"iterations": loop_iterations, "sleep_fraction": sum(metrics["loop_window.sleep_us"]) / loop_us if loop_us else None, "capture_deferred_iterations": dict(deferred)},
        "complete": footer is not None and invalid_lines == 0 and footer.get("queue_dropped", 0) == 0 and footer.get("size_limit_omitted", 0) == 0,
        "invalid_json_lines": invalid_lines,
        "notes": [
            "Stage times are inclusive and overlap (agents/avatar/agent_state/template); do not sum them.",
            "Absent throttled stages did not run; stage distributions contain only frames on which the stage ran.",
            "latest_source_frame_id/source_age_us describe the latest consumed frame, not proven flash-to-input causality.",
            "Input duration includes target validation and native completion; asynchronous mode also includes controller queueing and completion polling. It is not game response latency.",
            "With asynchronous capture/input, capture_us and input duration_us are I/O lifetimes, not main-thread blocking time; capture main_thread_us measures polling/readback only.",
            "Capture-start interval distributions exclude gaps between loop_start segments (pause/resume).",
            "An incomplete trace cannot be treated as complete-run counts or throughput.",
        ],
    }


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("trace", type=Path)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    result = json.dumps(analyze(args.trace), ensure_ascii=False, indent=2) + "\n"
    if args.output:
        args.output.write_text(result, encoding="utf-8")
    else:
        print(result, end="")
