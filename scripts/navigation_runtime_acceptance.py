#!/usr/bin/env python3
"""Capture normal gameplay and paused traffic using the external control client.

Build the binary and example first with `cargo build --bin
 top-down-2d-rts-prototype-nano-swarm --example local_avoidance`.
Screenshots require visual inspection after this driver exits.
"""

import argparse
import fcntl
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import time


ROOT = Path(__file__).resolve().parents[1]


def run_scene(kind, output):
    output.mkdir(parents=True, exist_ok=False)
    records = []
    with tempfile.TemporaryDirectory(prefix=f"ns66-{kind}-") as runtime:
        socket = Path(runtime) / "nano-swarm/control.sock"
        env = os.environ.copy()
        env.update(
            XDG_RUNTIME_DIR=runtime,
            BEVY_ASSET_ROOT=str(ROOT),
            LD_LIBRARY_PATH=os.pathsep.join([
                str(ROOT / "target/debug/deps"),
                subprocess.check_output(["rustc", "--print", "target-libdir"], text=True).strip(),
                env.get("LD_LIBRARY_PATH", ""),
            ]),
        )
        binary = "examples/local_avoidance" if kind == "bottleneck" else "top-down-2d-rts-prototype-nano-swarm"
        launch = [str(ROOT / "target/debug" / binary)]
        if kind == "normal":
            launch += ["--headless", "--agent-socket", "--width", "1280", "--height", "720"]
        (output / "launch.json").write_text(json.dumps({"argv": launch, "runtime": runtime}, indent=2))
        with (output / "process.log").open("w") as log:
            process = subprocess.Popen(launch, cwd=ROOT, env=env, stdout=log, stderr=subprocess.STDOUT)
            def call(*args):
                command = [sys.executable, str(ROOT / "scripts/nano_swarm_control.py"), "--socket", str(socket), *args]
                completed = subprocess.run(command, cwd=ROOT, capture_output=True, text=True, timeout=340)
                records.append({"command": command, "returncode": completed.returncode, "stdout": completed.stdout, "stderr": completed.stderr})
                (output / "commands.json").write_text(json.dumps(records, indent=2))
                completed.check_returncode()
                return json.loads(completed.stdout)

            def capture(name):
                call("wait", "--frames", "2")
                response = call("screenshot", "--name", name)
                shutil.copyfile(response["result"]["path"], output / f"{name}.png")
                (output / f"{name}.json").write_text(json.dumps(call("state"), indent=2))

            try:
                deadline = time.monotonic() + 60
                while not socket.exists():
                    if process.poll() is not None or time.monotonic() >= deadline:
                        raise RuntimeError(f"{kind} failed to become ready; see process.log")
                    time.sleep(0.05)
                call("hello")
                if kind == "normal":
                    call("camera", "256", "256", "1.5")
                    call("button", "intent.corridor")
                    for x, y in [(-1, 0), (0, 0), (0, 1)]:
                        call("paint", "corridor", str(x), str(y))
                    call("paint", "build", "-1", "1")
                    capture("start")
                    for name in ["working", "delivery", "later"]:
                        call("wait", "--fixed-ticks", "600")
                        capture(name)
                else:
                    capture("before")
                    for phase, x in [("retreat", "-3"), ("arrived", "-2")]:
                        call("paint", "corridor", x, "-3")
                        for _ in range(80):
                            call("wait", "--frames", "30")
                            if f"TRAFFIC {phase}:" in (output / "process.log").read_text():
                                break
                        else:
                            raise RuntimeError(f"traffic did not reach {phase}")
                        capture(phase)
                call("shutdown")
                exit_code = process.wait(timeout=30)
                assert exit_code == 0, exit_code
                assert not socket.exists(), "socket remains after shutdown"
                with Path(str(socket) + ".lock").open("r+") as lock:
                    fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
                    fcntl.flock(lock, fcntl.LOCK_UN)
                (output / "cleanup.json").write_text(json.dumps({"exit_code": exit_code, "socket_removed": True, "lock_released": True, "runtime_mode": oct(Path(runtime).stat().st_mode & 0o777)}, indent=2))
            finally:
                if process.poll() is None:
                    try:
                        call("shutdown")
                        process.wait(timeout=30)
                    finally:
                        if process.poll() is None:
                            process.terminate()
                            process.wait(timeout=10)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=ROOT / "target/issue-66/runtime/replay")
    args = parser.parse_args()
    for kind in ["normal", "bottleneck"]:
        run_scene(kind, args.output.resolve() / kind)


if __name__ == "__main__":
    main()
