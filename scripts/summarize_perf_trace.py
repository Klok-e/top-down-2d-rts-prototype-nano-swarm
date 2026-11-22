#!/usr/bin/env python3
"""Summarize frame and system times in a nano-swarm Chrome trace."""

import argparse
from bisect import bisect_right
from collections import defaultdict
import json
from pathlib import Path


FRAME_PREFIX = "perf_frame: frame="
SYSTEM_PREFIX = "system: name="


def completed_spans(events):
    stacks = defaultdict(list)
    for event in events:
        phase = event["ph"]
        if phase not in ("B", "E"):
            continue
        stack = stacks[event["pid"], event["tid"]]
        if phase == "B":
            stack.append((event["name"], event["ts"]))
        elif stack:
            name, start = stack.pop()
            yield name, start, event["ts"]


def summarize(events, frame_range):
    frames = []
    systems = []
    for name, start, end in completed_spans(events):
        if name.startswith(FRAME_PREFIX):
            frames.append((int(name.removeprefix(FRAME_PREFIX)), start, end))
        elif frame_range and name.startswith(SYSTEM_PREFIX):
            systems.append((name.removeprefix(SYSTEM_PREFIX).strip('"'), start, end))
    frames.sort(key=lambda frame: frame[1])
    return frames, systems


def system_timings(frames, systems):
    starts = [start for _, start, _ in frames]
    timings = {}
    for name, start, end in systems:
        index = bisect_right(starts, start) - 1
        if index < 0 or end > frames[index][2]:
            continue
        duration = end - start
        total, peak = timings.get(name, (0.0, 0.0))
        timings[name] = (total + duration, max(peak, duration))
    return timings


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("trace", type=Path)
    parser.add_argument(
        "--frames", type=int, nargs=2, metavar=("START", "END"),
        help="rank systems for this inclusive recorder frame range",
    )
    parser.add_argument("--limit", type=int, default=10, help="maximum rows per ranking")
    args = parser.parse_args()
    if args.limit < 1:
        parser.error("--limit must be positive")
    if args.frames and not 1 <= args.frames[0] <= args.frames[1]:
        parser.error("--frames requires 1 <= START <= END")

    try:
        with args.trace.open(encoding="utf-8") as trace:
            frames, systems = summarize(json.load(trace), args.frames)
    except (OSError, ValueError) as error:
        parser.error(str(error))
    if not frames:
        parser.error("no complete perf_frame spans found; capture with RUST_LOG=info")

    print("Slowest main-world frames (ms)")
    slowest = sorted(frames, key=lambda frame: (frame[1] - frame[2], frame[0]))
    for frame, start, end in slowest[:args.limit]:
        print(f"{frame}\t{(end - start) / 1000:.3f}")
    if args.frames:
        first, last = args.frames
        selected = [frame for frame in frames if first <= frame[0] <= last]
        print(f"\nFrames {first}-{last} ({len(selected)} of {last - first + 1} captured)")
        timings = system_timings(selected, systems)
        for column, metric in enumerate(("total", "peak")):
            print(f"\nSystems by {metric} time (ms)")
            rank = sorted(timings, key=lambda name: (-timings[name][column], name))
            for name in rank[:args.limit]:
                print(f"{timings[name][column] / 1000:.3f}\t{name}")


if __name__ == "__main__":
    main()
