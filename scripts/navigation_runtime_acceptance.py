#!/usr/bin/env python3
"""Capture gameplay, delivery traffic, and Structure Clearing using the external control client.

Build the binary and example first with `cargo build --bin
 top-down-2d-rts-prototype-nano-swarm --example local_avoidance`.
Screenshots require visual inspection after this driver exits.
"""

import argparse
from contextlib import nullcontext
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


def run_scene(kind, output, runtime_dir=None):
    output.mkdir(parents=True, exist_ok=False)
    records = []
    runtime_context = (
        tempfile.TemporaryDirectory(prefix=f"ns-{kind}-")
        if runtime_dir is None else nullcontext(runtime_dir)
    )
    with runtime_context as runtime:
        socket = Path(runtime) / "nano-swarm/control.sock"
        if os.path.lexists(socket):
            raise RuntimeError(f"Control socket path already exists: {socket}")
        env = os.environ.copy()
        target = Path(env.get("CARGO_TARGET_DIR", ROOT / "target")).resolve()
        tuning = output / "balance.ron"
        shutil.copyfile(ROOT / "config/balance.ron", tuning)
        env.update(
            XDG_RUNTIME_DIR=str(runtime),
            XDG_CONFIG_HOME=env.get("XDG_CONFIG_HOME", str(output / "config")),
            NANO_SWARM_REPLAY_TUNING=str(tuning),
            NANO_SWARM_REPLAY_SCENE=kind,
            BEVY_ASSET_ROOT=str(ROOT),
            LD_LIBRARY_PATH=os.pathsep.join([
                str(target / "debug"),
                str(target / "debug/deps"),
                subprocess.check_output(["rustc", "--print", "target-libdir"], text=True).strip(),
                env.get("LD_LIBRARY_PATH", ""),
            ]),
        )
        binary = "examples/local_avoidance" if kind != "normal" else "top-down-2d-rts-prototype-nano-swarm"
        launch = [str(target / "debug" / binary)]
        if kind == "normal":
            launch += ["--headless", "--width", "1280", "--height", "720", "--tuning-file", str(tuning)]
        (output / "launch.json").write_text(json.dumps({"argv": launch, "runtime": str(runtime)}, indent=2))
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
                (output / f"{name}.json").write_text(json.dumps(call("state", "--details"), indent=2))

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
                    label, final_phase = ("CLEARING", "activated") if kind == "clearing" else ("TRAFFIC", "arrived")
                    for phase, x in [("passing", "-3"), (final_phase, "-2")]:
                        call("paint", "corridor", x, "-3")
                        for _ in range(80):
                            call("wait", "--frames", "30")
                            if f"{label} {phase}:" in (output / "process.log").read_text():
                                break
                        else:
                            raise RuntimeError(f"{kind} did not reach {phase}")
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
    parser.add_argument("--scene", action="append", choices=["normal", "bottleneck", "clearing"])
    parser.add_argument("--runtime-dir", type=Path, help="Existing private directory with space for the control socket")
    args = parser.parse_args()
    for kind in args.scene or ["normal", "bottleneck", "clearing"]:
        run_scene(kind, args.output.resolve() / kind, args.runtime_dir)


if __name__ == "__main__":
    main()
