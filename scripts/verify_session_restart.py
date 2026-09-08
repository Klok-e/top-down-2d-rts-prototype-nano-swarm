#!/usr/bin/env python3
"""Verify menu restarts through the real offscreen process and control client."""

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


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--artifacts", type=Path, required=True)
    args = parser.parse_args()
    artifacts = args.artifacts.resolve()
    artifacts.mkdir(parents=True, exist_ok=False)
    root = Path(__file__).resolve().parents[1]
    binary = root / "target/debug/top-down-2d-rts-prototype-nano-swarm"
    client = root / "scripts/nano_swarm_control.py"
    with tempfile.TemporaryDirectory(prefix="nsr-") as runtime:
        env = dict(os.environ, XDG_RUNTIME_DIR=runtime, XDG_CONFIG_HOME=str(artifacts / "config"))
        env["BEVY_ASSET_ROOT"] = str(root)
        env["LD_LIBRARY_PATH"] = os.pathsep.join([
            str(binary.parent / "deps"),
            subprocess.check_output(["rustc", "--print", "target-libdir"], text=True).strip(),
            env.get("LD_LIBRARY_PATH", ""),
        ])
        env.pop("DISPLAY", None)
        env.pop("WAYLAND_DISPLAY", None)
        socket = Path(runtime) / "nano-swarm/control.sock"
        with (artifacts / "process.log").open("w") as log:
            process = subprocess.Popen([
                str(binary), "--headless", "--agent-socket", "--scenario", "standard",
                "--output-root", str(artifacts / "runs"), "--width", "1280", "--height", "720",
            ], env=env, stdout=log, stderr=subprocess.STDOUT)
            try:
                deadline = time.monotonic() + 120
                while not socket.exists():
                    if process.poll() is not None:
                        raise RuntimeError("process exited during startup; see process.log")
                    if time.monotonic() >= deadline:
                        raise TimeoutError("startup watchdog expired")
                    time.sleep(0.05)
                responses = []

                def command(*arguments):
                    result = subprocess.run([
                        sys.executable, str(client), "--socket", str(socket), "--timeout", "45", *arguments,
                    ], env=env, capture_output=True, text=True, timeout=55)
                    response = json.loads(result.stdout or result.stderr)
                    responses.append(response)
                    (artifacts / "responses.json").write_text(json.dumps(responses, indent=2) + "\n")
                    assert response["ok"], response
                    return response["result"]

                assert command("hello")["protocol_version"] == 4
                command("wait", "--frames", "3")
                for index, scenario in enumerate(("sandbox", "ai_battle", "standard", "standard")):
                    command("camera", "0", "0", "17")
                    command("menu")
                    command("button", f"menu.{scenario}")
                    command("wait", "--frames", "2")
                    if index == 0:
                        capture = command("screenshot", "--name", "restart-menu")
                        shutil.copy2(capture["path"], artifacts / "menu.png")
                    command("button", "menu.start")
                    command("wait", "--frames", "3")
                    state = command("state", "--cell-limit", "1")
                    assert state["scenario"]["current"] == scenario, state
                    assert not state["scenario"]["menu_open"], state
                    assert len(state["swarms"]) == (1 if scenario == "sandbox" else 2), state
                    assert state["match"]["outcome"] == "in_progress", state
                    assert state["camera"]["zoom"] == 2.0, state
                    capture = command("screenshot", "--name", f"restart-{index}-{scenario}")
                    shutil.copy2(capture["path"], artifacts / f"{index}-{scenario}.png")
                    assert process.poll() is None, "restart must preserve the running process"
                command("shutdown")
                assert process.wait(timeout=30) == 0
                assert not socket.exists(), "shutdown must remove the control socket"
                with socket.with_name("control.sock.lock").open("rb") as lock:
                    fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
                summaries = list((artifacts / "runs").glob("*/summary.json"))
                assert len(summaries) == 1, summaries
                assert json.loads(summaries[0].read_text())["status"] == "interrupted"
                print(f"Verified four same-process restarts; artifacts: {artifacts}")
            finally:
                if process.poll() is None:
                    process.terminate()
                    try:
                        process.wait(timeout=10)
                    except subprocess.TimeoutExpired:
                        process.kill()
                        process.wait()


if __name__ == "__main__":
    main()
