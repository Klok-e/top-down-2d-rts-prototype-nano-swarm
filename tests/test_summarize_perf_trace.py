import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest


REPO = Path(__file__).resolve().parents[1]
SCRIPT = REPO / "scripts" / "summarize_perf_trace.py"


class TraceSummaryTests(unittest.TestCase):
    def run_summary(self, *options):
        spans = [
            ("perf_frame: frame=101", 0, 0, 10_000),
            ('system: name="gather"', 2, 1_000, 5_000),
            ('system: name="move"', 1, 2_000, 8_000),
            ("perf_frame: frame=102", 0, 12_000, 32_000),
            ('system: name="gather"', 1, 13_000, 17_000),
            ('system: name="move"', 1, 18_000, 19_000),
            ('system: name="crosses_frame_end"', 2, 31_000, 51_000),
            ("perf_frame: frame=103", 0, 35_000, 65_000),
            ('system: name="gather"', 1, 36_000, 60_000),
            ('system: name="build"', 1, 60_000, 64_000),
        ]
        events = [
            dict(name=name, pid=1, tid=thread, ph=phase, ts=timestamp)
            for name, thread, start, end in spans
            for phase, timestamp in [("B", start), ("E", end)]
        ]
        events.sort(key=lambda event: event["ts"])

        scratch = REPO / ".tmp"
        scratch.mkdir(exist_ok=True)
        with tempfile.TemporaryDirectory(prefix="ns-trace-", dir=scratch) as directory:
            trace = Path(directory) / "trace.json"
            trace.write_text(json.dumps(events), encoding="utf-8")
            run = subprocess.run(
                [sys.executable, str(SCRIPT), str(trace), *options],
                capture_output=True, text=True, timeout=5,
            )
            self.assertEqual(run.returncode, 0, run.stderr)
            return run.stdout.strip().split("\n\n")

    def test_slowest_frames_rank_by_duration_with_their_recorder_numbers(self):
        frames, = self.run_summary("--limit", "2")
        self.assertEqual(frames.splitlines()[1:], ["103\t30.000", "102\t20.000"])

    def test_inclusive_range_ranks_worker_systems_by_total_and_peak(self):
        _, selected, by_total, by_peak = self.run_summary("--frames", "101", "102")
        self.assertEqual(selected, "Frames 101-102 (2 of 2 captured)")
        self.assertEqual(by_total.splitlines()[1:], ["8.000\tgather", "7.000\tmove"])
        self.assertEqual(by_peak.splitlines()[1:], ["6.000\tmove", "4.000\tgather"])


if __name__ == "__main__":
    unittest.main()
