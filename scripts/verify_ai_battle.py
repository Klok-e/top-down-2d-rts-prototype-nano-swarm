#!/usr/bin/env python3
"""Verify an actual offscreen AI Battle process and retain its artifacts."""

import argparse
import csv
import fcntl
import json
import os
from pathlib import Path
import shutil
import signal
import subprocess
import sys
import tempfile
import time


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=Path("target/debug/top-down-2d-rts-prototype-nano-swarm"))
    parser.add_argument("--artifacts", type=Path, required=True)
    args = parser.parse_args()
    artifacts = args.artifacts.resolve()
    artifacts.mkdir(parents=True, exist_ok=False)
    client = Path(__file__).with_name("nano_swarm_control.py")
    with tempfile.TemporaryDirectory(prefix="nsb-") as runtime:
        env = dict(os.environ, XDG_RUNTIME_DIR=runtime, XDG_CONFIG_HOME=str(artifacts / "config"))
        env["BEVY_ASSET_ROOT"] = str(Path(__file__).resolve().parents[1])
        env["LD_LIBRARY_PATH"] = os.pathsep.join([
            str(args.binary.resolve().parent / "deps"),
            subprocess.check_output(["rustc", "--print", "target-libdir"], text=True).strip(),
            env.get("LD_LIBRARY_PATH", ""),
        ])
        env.pop("DISPLAY", None)
        env.pop("WAYLAND_DISPLAY", None)
        socket = Path(runtime) / "nano-swarm/control.sock"
        with (artifacts / "process.log").open("w") as log:
            process = subprocess.Popen([
                str(args.binary.resolve()), "--headless", "--agent-socket", "--scenario", "ai-battle",
                "--seed", "42", "--output-root", str(artifacts / "runs"),
                "--width", "1280", "--height", "720",
            ], env=env, stdout=log, stderr=subprocess.STDOUT)
            try:
                deadline = time.monotonic() + 120
                while not socket.exists():
                    if process.poll() is not None:
                        raise RuntimeError(f"process exited during startup: {process.returncode}; see process.log")
                    if time.monotonic() >= deadline:
                        raise TimeoutError("verification startup watchdog expired")
                    time.sleep(0.05)

                def command(name, *arguments, success=True):
                    result = subprocess.run([
                        sys.executable, str(client), "--socket", str(socket), "--timeout", "320", name, *arguments,
                    ], env=env, capture_output=True, text=True, timeout=330)
                    # The client reports protocol errors on stderr with a nonzero status.
                    response = json.loads(result.stdout or result.stderr)
                    if response["ok"] != success:
                        raise AssertionError(response)
                    (artifacts / f"{name}.json").write_text(json.dumps(response, indent=2) + "\n")
                    return response

                command("hello")
                command("wait", "--fixed-ticks", "120")
                state = command("state")
                assert state["result"]["scenario"]["current"] == "ai_battle", state
                command("paint", "defend", "2", "0", success=False)
                command("camera", "6400", "6400", "20")
                capture = command("screenshot", "--name", "ai-battle")
                shutil.copy2(capture["result"]["path"], artifacts / "ai-battle.png")
                process.send_signal(signal.SIGINT)
                assert process.wait(timeout=120) == 130, "Ctrl+C must use its conventional exit status"
                summaries = list((artifacts / "runs").glob("*/summary.json"))
                assert len(summaries) == 1, summaries
                summary = json.loads(summaries[0].read_text())
                assert summary["status"] == "interrupted", summary
                assert summary["seed"] == 42, summary
                assert summary["outcome"] is None, summary
                assert summary["latest"]["simulation_seconds"] >= 2.0, summary
                with summaries[0].with_name("samples.csv").open(newline="") as source:
                    samples = list(csv.DictReader(source))
                assert len(samples) >= 2, samples
                assert not socket.exists(), "graceful exit must remove the control socket"
                with socket.with_name("control.sock.lock").open("rb") as lock:
                    fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
                print(json.dumps({"summary": str(summaries[0]), "sample_rows": len(samples), "screenshot": str(artifacts / "ai-battle.png")}, indent=2))
            finally:
                if process.poll() is None:
                    process.send_signal(signal.SIGINT)
                    try:
                        process.wait(timeout=30)
                    except subprocess.TimeoutExpired:
                        process.kill()
                        process.wait()


if __name__ == "__main__":
    main()
