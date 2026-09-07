#!/usr/bin/env python3

import argparse
import json
import os
from pathlib import Path
import socket
import sys
import time


MAX_RESPONSE_BYTES = 16 * 1024 * 1024
MAX_STATE_CELL_LIMIT = 10_000
INTENTS = ("gather", "build", "defend", "corridor")


def default_socket_path() -> Path:
    runtime_dir = os.environ.get("XDG_RUNTIME_DIR")
    if not runtime_dir:
        raise RuntimeError("XDG_RUNTIME_DIR is not set")
    return Path(runtime_dir) / "nano-swarm" / "control.sock"


def request(socket_path: Path, timeout: float, method: str, params: dict | None) -> dict:
    request_id = f"{os.getpid()}-{time.time_ns()}"
    payload = {"id": request_id, "method": method}
    if params:
        payload["params"] = params

    with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as connection:
        connection.settimeout(timeout)
        connection.connect(str(socket_path))
        connection.sendall(json.dumps(payload, separators=(",", ":")).encode() + b"\n")

        response = bytearray()
        while b"\n" not in response:
            chunk = connection.recv(65536)
            if not chunk:
                raise RuntimeError("control socket closed before returning a response")
            response.extend(chunk)
            if len(response) > MAX_RESPONSE_BYTES:
                raise RuntimeError("control response exceeds 16 MiB limit")

    line, _, _remainder = response.partition(b"\n")
    decoded = json.loads(line)
    if decoded.get("id") != request_id:
        raise RuntimeError("control response request ID does not match")
    return decoded


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description="Control a running nano-swarm process")
    parser.add_argument("--socket", type=Path, help="override the Unix control socket path")
    parser.add_argument("--timeout", type=float, default=320.0, help="response timeout in seconds")
    commands = parser.add_subparsers(dest="command", required=True)

    commands.add_parser("hello")
    state = commands.add_parser("state")
    state.add_argument("--cell-offset", type=int, default=0)
    state.add_argument("--cell-limit", type=int, default=MAX_STATE_CELL_LIMIT)
    state.add_argument("--map-revision", type=int)

    button = commands.add_parser("button")
    button.add_argument(
        "button",
        choices=tuple(f"intent.{intent}" for intent in INTENTS) + ("menu.open", "menu.resume", "menu.standard", "menu.sandbox", "menu.quit"),
    )

    select = commands.add_parser("select")
    select.add_argument("intent", choices=INTENTS)

    for command in ("paint", "erase"):
        edit = commands.add_parser(command)
        edit.add_argument("intent", choices=INTENTS)
        edit.add_argument("x", type=int)
        edit.add_argument("y", type=int)

    camera = commands.add_parser("camera")
    camera.add_argument("x", type=float)
    camera.add_argument("y", type=float)
    camera.add_argument("zoom", type=float, nargs="?")

    pan = commands.add_parser("pan")
    pan.add_argument("dx", type=float)
    pan.add_argument("dy", type=float)

    wait = commands.add_parser("wait")
    wait.add_argument("--frames", type=int, default=0)
    wait.add_argument("--fixed-ticks", type=int, default=0)

    screenshot = commands.add_parser("screenshot")
    screenshot.add_argument("--name")

    commands.add_parser("shutdown")
    return parser


def command_request(args: argparse.Namespace) -> tuple[str, dict | None]:
    if args.command == "hello":
        return "session.hello", None
    if args.command == "state":
        if args.cell_offset < 0:
            raise ValueError("state cell offset cannot be negative")
        if args.map_revision is not None and args.map_revision < 0:
            raise ValueError("state map revision cannot be negative")
        if args.cell_offset > 0 and args.map_revision is None:
            raise ValueError("state pages after offset zero require --map-revision")
        if not 1 <= args.cell_limit <= MAX_STATE_CELL_LIMIT:
            raise ValueError(
                f"state cell limit must be between 1 and {MAX_STATE_CELL_LIMIT}"
            )
        params = {
            "cell_offset": args.cell_offset,
            "cell_limit": args.cell_limit,
        }
        if args.map_revision is not None:
            params["map_revision"] = args.map_revision
        return "state.get", params
    if args.command == "button":
        return "button.press", {"button": args.button}
    if args.command == "select":
        return "intent.select", {"intent": args.intent}
    if args.command in ("paint", "erase"):
        return "map.apply", {
            "action": args.command,
            "intent": args.intent,
            "x": args.x,
            "y": args.y,
        }
    if args.command == "camera":
        params = {"x": args.x, "y": args.y}
        if args.zoom is not None:
            params["zoom"] = args.zoom
        return "camera.set", params
    if args.command == "pan":
        return "camera.pan", {"dx": args.dx, "dy": args.dy}
    if args.command == "wait":
        if args.frames < 0 or args.fixed_ticks < 0:
            raise ValueError("wait values cannot be negative")
        if args.frames == 0 and args.fixed_ticks == 0:
            raise ValueError("wait requires --frames or --fixed-ticks greater than zero")
        return "frame.wait", {
            "frames": args.frames,
            "fixed_ticks": args.fixed_ticks,
        }
    if args.command == "screenshot":
        return "screenshot.capture", {"name": args.name} if args.name else None
    if args.command == "shutdown":
        return "process.shutdown", None
    raise ValueError(f"unsupported command: {args.command}")


def main() -> int:
    parser = build_parser()
    args = parser.parse_args()
    try:
        method, params = command_request(args)
        socket_path = args.socket or default_socket_path()
        response = request(socket_path, args.timeout, method, params)
    except (OSError, RuntimeError, ValueError, json.JSONDecodeError) as error:
        print(f"error: {error}", file=sys.stderr)
        return 2

    print(json.dumps(response, indent=2, sort_keys=True))
    return 0 if response.get("ok") else 1


if __name__ == "__main__":
    raise SystemExit(main())
