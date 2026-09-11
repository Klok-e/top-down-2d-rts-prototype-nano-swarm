#!/usr/bin/env python3
"""Run and report a reproducible four-game AI Battle smoke matrix."""

import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
from typing import NamedTuple


ROOT = Path(__file__).resolve().parents[1]
CONTROLLERS = ("timed", "adaptive")
PACING = ("baseline", "deliberate")
LAYOUTS = ("standard", "flanks", "narrows", "crossroads")
DEFAULT_SEEDS = (11,)
DEFAULT_LAYOUTS = ("standard", "flanks")
EXECUTION = {
    "headless": True,
    "agent_socket": True,
    "width": 640,
    "height": 360,
    "build_profile": "release",
}
GAMEPLAY_PACING = {
    "baseline": {
        "construction_work_ticks": 5,
        "attack_interval_ticks": 15,
        "charge_drain_per_tick": 0.00025,
    },
    "deliberate": {
        "construction_work_ticks": 90,
        "attack_interval_ticks": 30,
        "charge_drain_per_tick": 0.000125,
    },
}
SOURCE_IDENTITY_ENV = "NANO_SWARM_SOURCE_IDENTITY"


class ExperimentConfig(NamedTuple):
    controller: str
    opponent: str
    pacing: str
    seeds: tuple[int, ...]
    layouts: tuple[str, ...]
    trial_seconds: int


class Trial(NamedTuple):
    id: str
    subject: str
    opponent: str
    layout: str
    seed: int
    swap_sides: bool
    pacing: str
    trial_seconds: int


def canonical_json(value) -> str:
    return json.dumps(value, sort_keys=True, separators=(",", ":"))


def validate_config(config: ExperimentConfig) -> None:
    if config.controller not in CONTROLLERS:
        raise ValueError(f"controller must be from {','.join(CONTROLLERS)}")
    if config.opponent not in CONTROLLERS:
        raise ValueError(f"opponent must be from {','.join(CONTROLLERS)}")
    if config.pacing not in PACING:
        raise ValueError(f"pacing must be from {','.join(PACING)}")
    if not config.seeds or any(seed < 0 for seed in config.seeds):
        raise ValueError("seeds must be non-negative and non-empty")
    if not config.layouts or any(layout not in LAYOUTS for layout in config.layouts):
        raise ValueError(f"layouts must be non-empty values from {','.join(LAYOUTS)}")
    for name, values in (("seeds", config.seeds), ("layouts", config.layouts)):
        if len(values) != len(set(values)):
            raise ValueError(f"duplicate {name} would run the same raw match more than once")
    if config.trial_seconds <= 0:
        raise ValueError("trial seconds must be positive")


def build_matrix(config: ExperimentConfig) -> list[Trial]:
    validate_config(config)
    trials = []
    for layout in config.layouts:
        for seed in config.seeds:
            for swap_sides in (False, True):
                identity = {
                    "subject": config.controller,
                    "opponent": config.opponent,
                    "layout": layout,
                    "seed": seed,
                    "swap_sides": swap_sides,
                    "pacing": config.pacing,
                    "trial_seconds": config.trial_seconds,
                }
                digest = hashlib.sha256(canonical_json(identity).encode()).hexdigest()[:10]
                side = "swapped" if swap_sides else "direct"
                trial_id = (
                    f"{config.controller}-vs-{config.opponent}-{layout}-{seed}-{side}-{digest}"
                )
                trials.append(
                    Trial(
                        trial_id,
                        config.controller,
                        config.opponent,
                        layout,
                        seed,
                        swap_sides,
                        config.pacing,
                        config.trial_seconds,
                    )
                )
    return trials


def _git(*arguments: str) -> bytes:
    completed = subprocess.run(
        ["git", *arguments], cwd=ROOT, check=True, capture_output=True
    )
    return completed.stdout


def capture_source(excluded: Path | None = None) -> tuple[dict, bytes, dict[str, bytes]]:
    revision = _git("rev-parse", "HEAD").decode().strip()
    tracked_diff = _git("diff", "--binary", "HEAD")
    untracked = _git("ls-files", "--others", "--exclude-standard", "-z").split(b"\0")
    excluded = excluded.resolve() if excluded else None
    digest = hashlib.sha256()
    digest.update(tracked_diff)
    retained_untracked = []
    untracked_files = {}
    for encoded in sorted(path for path in untracked if path):
        relative = Path(os.fsdecode(encoded))
        safe_relative_path(str(relative), "untracked source")
        source_path = ROOT / relative
        if source_path.is_symlink():
            raise RuntimeError(f"untracked source symlinks are not supported: {relative}")
        path = source_path.resolve()
        if excluded and (path == excluded or excluded in path.parents):
            continue
        retained_untracked.append(str(relative))
        contents = path.read_bytes()
        untracked_files[str(relative)] = contents
        digest.update(b"\0untracked\0")
        digest.update(encoded)
        digest.update(b"\0")
        digest.update(contents)
    dirty = bool(tracked_diff or retained_untracked)
    fingerprint = {
        "revision": revision,
        "dirty": dirty,
        "content_sha256": digest.hexdigest(),
        "untracked": retained_untracked,
    }
    return fingerprint, tracked_diff, untracked_files


def source_fingerprint(excluded: Path | None = None) -> dict:
    return capture_source(excluded)[0]


def source_identity(source: dict) -> dict:
    return {
        key: source.get(key)
        for key in ("revision", "dirty", "content_sha256", "untracked")
    }


def validate_active_output_location(output: Path) -> None:
    root = ROOT.resolve()
    resolved = output.resolve()
    try:
        relative = resolved.relative_to(root)
    except ValueError:
        return
    ignored = subprocess.run(
        ["git", "check-ignore", "--quiet", "--", os.fspath(relative)],
        cwd=ROOT,
        check=False,
    )
    if ignored.returncode != 0:
        raise RuntimeError(
            "an experiment output inside the repository must be Git-ignored; "
            "use target/... or a path outside the repository"
        )


def safe_relative_path(value, label: str) -> Path:
    if not isinstance(value, str):
        raise RuntimeError(f"source bundle {label} path must be a string")
    path = Path(value)
    if not value or path.is_absolute() or ".." in path.parts:
        raise RuntimeError(f"source bundle {label} contains an unsafe path: {value!r}")
    return path


def contained_bundle_path(root: Path, relative: Path, label: str) -> Path:
    resolved_root = root.resolve()
    resolved = (root / relative).resolve()
    if resolved != resolved_root and resolved_root not in resolved.parents:
        raise RuntimeError(f"source bundle {label} resolves outside its bundle root")
    return resolved


def write_source_bundle(
    output: Path, fingerprint: dict, tracked_diff: bytes, untracked_files: dict[str, bytes]
) -> dict:
    source_root = output / "source"
    source_root.mkdir()
    tracked_path = source_root / "tracked.patch"
    tracked_path.write_bytes(tracked_diff)
    untracked_root = source_root / "untracked"
    untracked_root.mkdir()
    for relative_name in fingerprint["untracked"]:
        relative = safe_relative_path(relative_name, "untracked")
        destination = untracked_root / relative
        destination.parent.mkdir(parents=True, exist_ok=True)
        destination.write_bytes(untracked_files[relative_name])
    return {
        "tracked_patch": "source/tracked.patch",
        "untracked_root": "source/untracked",
    }


def validate_source_bundle(output: Path, source: dict) -> None:
    bundle = source.get("bundle")
    if not isinstance(bundle, dict):
        raise RuntimeError("manifest source bundle metadata is missing")
    tracked_path = contained_bundle_path(
        output,
        safe_relative_path(bundle.get("tracked_patch"), "tracked patch"),
        "tracked patch",
    )
    untracked_root = contained_bundle_path(
        output,
        safe_relative_path(bundle.get("untracked_root"), "untracked root"),
        "untracked root",
    )
    untracked_names = source.get("untracked")
    if not isinstance(untracked_names, list) or len(untracked_names) != len(
        set(untracked_names)
    ):
        raise RuntimeError("manifest source bundle has an invalid untracked file list")
    try:
        tracked_diff = tracked_path.read_bytes()
        digest = hashlib.sha256(tracked_diff)
        for relative_name in sorted(untracked_names):
            relative = safe_relative_path(relative_name, "untracked")
            contents = contained_bundle_path(
                untracked_root, relative, "untracked file"
            ).read_bytes()
            digest.update(b"\0untracked\0")
            digest.update(os.fsencode(relative_name))
            digest.update(b"\0")
            digest.update(contents)
    except OSError as error:
        raise RuntimeError(f"cannot read source bundle: {error}") from error
    if digest.hexdigest() != source.get("content_sha256"):
        raise RuntimeError("source bundle content hash does not match manifest")
    if source.get("dirty") is not bool(tracked_diff or untracked_names):
        raise RuntimeError("source bundle dirty state does not match manifest")


def verify_source_fingerprint(expected: dict, output: Path) -> None:
    if source_identity(source_fingerprint(output)) != source_identity(expected):
        raise RuntimeError("source changed after the experiment manifest was created")


def config_dict(config: ExperimentConfig) -> dict:
    return {
        "controller": config.controller,
        "opponent": config.opponent,
        "pacing": config.pacing,
        "seeds": list(config.seeds),
        "layouts": list(config.layouts),
        "trial_seconds": config.trial_seconds,
    }


def trial_dict(trial: Trial) -> dict:
    return trial._asdict()


def write_json(path: Path, value) -> None:
    temporary = path.with_suffix(path.suffix + ".tmp")
    temporary.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n")
    temporary.replace(path)


def load_or_create_manifest(
    output: Path, config: ExperimentConfig, trials: list[Trial]
) -> dict:
    validate_active_output_location(output)
    output.mkdir(parents=True, exist_ok=True)
    fingerprint, tracked_diff, untracked_files = capture_source(output)
    fingerprint["bundle"] = {
        "tracked_patch": "source/tracked.patch",
        "untracked_root": "source/untracked",
    }
    expected = {
        "schema_version": 3,
        "source": fingerprint,
        "config": config_dict(config),
        "execution": EXECUTION,
        "trials": [trial_dict(trial) for trial in trials],
    }
    path = output / "manifest.json"
    if path.exists():
        actual = json.loads(path.read_text())
        if actual != expected:
            raise RuntimeError(
                "manifest does not match the current configuration and source fingerprint"
            )
        validate_source_bundle(output, actual["source"])
        return actual
    fingerprint["bundle"] = write_source_bundle(
        output, fingerprint, tracked_diff, untracked_files
    )
    validate_source_bundle(output, fingerprint)
    write_json(path, expected)
    return expected


def load_manifest_for_report(
    output: Path, config: ExperimentConfig, trials: list[Trial]
) -> dict:
    path = output / "manifest.json"
    if not path.is_file():
        raise RuntimeError("--report-only requires an existing manifest")
    manifest = json.loads(path.read_text())
    if (
        manifest.get("schema_version") != 3
        or manifest.get("config") != config_dict(config)
        or manifest.get("execution") != EXECUTION
        or manifest.get("trials") != [trial_dict(trial) for trial in trials]
        or not isinstance(manifest.get("source"), dict)
    ):
        raise RuntimeError("manifest does not match the requested report configuration")
    validate_source_bundle(output, manifest["source"])
    return manifest


def _summary_path(trial_root: Path) -> Path:
    paths = sorted(trial_root.glob("battle-*/summary.json"))
    if len(paths) != 1:
        raise RuntimeError(
            f"{trial_root}: expected exactly one battle summary, found {len(paths)}"
        )
    return paths[0]


def _side_observation(trial: Trial, summary: dict, side: int) -> dict:
    if summary.get("schema_version") != 3:
        raise RuntimeError(f"{trial.id}: damage scoring requires summary schema 3")
    latest = summary.get("latest")
    if not isinstance(latest, dict):
        raise RuntimeError(f"{trial.id}: damage scoring requires a final sample")
    swarms = latest.get("swarms")
    if not isinstance(swarms, dict) or str(side) not in swarms:
        raise RuntimeError(f"{trial.id}: missing attacker damage sample")
    counters = swarms[str(side)]
    if not isinstance(counters, dict):
        raise RuntimeError(f"{trial.id}: invalid attacker damage sample")
    for prefix in ("effective_damage", "scored_damage"):
        values = [
            counters.get(f"{prefix}_{part}")
            for part in ("total", "nanobots", "structures")
        ]
        if any(type(value) is not int or value < 0 for value in values):
            raise RuntimeError(f"{trial.id}: missing or invalid {prefix} counters")
        if values[0] != values[1] + values[2]:
            raise RuntimeError(f"{trial.id}: inconsistent {prefix} subtotals")
    if any(
        counters[f"scored_damage_{part}"] > counters[f"effective_damage_{part}"]
        for part in ("total", "nanobots", "structures")
    ):
        raise RuntimeError(f"{trial.id}: scored damage exceeds effective damage")
    seconds = latest.get("simulation_seconds")
    if (
        type(seconds) not in (int, float)
        or not math.isfinite(seconds)
        or seconds <= 0
        or seconds > trial.trial_seconds + 0.001
    ):
        raise RuntimeError(f"{trial.id}: invalid damage scoring duration")
    status, outcome = summary.get("status"), summary.get("outcome")
    if (
        (
            status == "completed"
            and outcome not in ("swarm_0_wins", "swarm_1_wins", "draw")
        )
        or (status == "unresolved" and (outcome is not None or seconds < trial.trial_seconds))
        or status not in ("completed", "unresolved")
    ):
        raise RuntimeError(f"{trial.id}: invalid damage scoring outcome")
    won = status == "completed" and outcome == f"swarm_{side}_wins"
    lost = status == "completed" and outcome == f"swarm_{1 - side}_wins"
    return {
        "damage_hp": counters["scored_damage_total"],
        "nanobot_damage_hp": counters["scored_damage_nanobots"],
        "structure_damage_hp": counters["scored_damage_structures"],
        "gross_damage_hp": counters["effective_damage_total"],
        "gross_nanobot_damage_hp": counters["effective_damage_nanobots"],
        "gross_structure_damage_hp": counters["effective_damage_structures"],
        "uncredited_repeat_damage_hp": counters["effective_damage_total"]
        - counters["scored_damage_total"],
        "uncredited_repeat_nanobot_damage_hp": counters["effective_damage_nanobots"]
        - counters["scored_damage_nanobots"],
        "uncredited_repeat_structure_damage_hp": counters[
            "effective_damage_structures"
        ]
        - counters["scored_damage_structures"],
        "won": won,
        "lost": lost,
        "win_seconds": seconds if won else None,
        "capped_completion_cost_seconds": (
            min(seconds, trial.trial_seconds) if won else trial.trial_seconds
        ),
    }


def damage_observation(trial: Trial, summary: dict) -> dict:
    """Extract the configured controller's authoritative combat result."""
    return _side_observation(trial, summary, int(trial.swap_sides))


def opponent_damage_observation(trial: Trial, summary: dict) -> dict:
    """Extract the configured opponent's authoritative combat result."""
    return _side_observation(trial, summary, int(not trial.swap_sides))


def validate_summary(summary: dict, trial: Trial, source: dict) -> None:
    status = summary.get("status")
    outcome = summary.get("outcome")
    if status not in ("completed", "unresolved"):
        raise RuntimeError(f"{trial.id}: invalid run status {status!r}")
    if status == "completed" and outcome not in (
        "swarm_0_wins",
        "swarm_1_wins",
        "draw",
    ):
        raise RuntimeError(f"{trial.id}: completed run has invalid outcome {outcome!r}")
    if status == "unresolved" and outcome is not None:
        raise RuntimeError(f"{trial.id}: unresolved run invented outcome {outcome!r}")
    if status == "unresolved":
        latest = summary.get("latest", {})
        expected_ticks = trial.trial_seconds * 60
        simulation_seconds = latest.get("simulation_seconds")
        timestep = summary.get("timestep_seconds")
        if (
            latest.get("fixed_tick") != expected_ticks
            or not isinstance(simulation_seconds, (int, float))
            or not isinstance(timestep, (int, float))
            or simulation_seconds < trial.trial_seconds
            or abs(simulation_seconds - expected_ticks * timestep) > 1e-6
        ):
            raise RuntimeError(f"{trial.id}: unresolved run did not reach its configured cutoff")
    expected_experiment = {
        "controllers": [trial.subject, trial.opponent],
        "layout": trial.layout,
        "swap_sides": trial.swap_sides,
        "pacing": trial.pacing,
        "cutoff_seconds": trial.trial_seconds,
        "realtime": False,
    }
    if summary.get("experiment") != expected_experiment:
        raise RuntimeError(
            f"{trial.id}: recorded experiment configuration does not match manifest"
        )
    if summary.get("gameplay_pacing") != GAMEPLAY_PACING[trial.pacing]:
        raise RuntimeError(
            f"{trial.id}: recorded numeric gameplay pacing does not match manifest"
        )
    if summary.get("seed") != trial.seed:
        raise RuntimeError(f"{trial.id}: recorded seed does not match manifest")
    if summary.get("code_revision") != source["revision"]:
        raise RuntimeError(f"{trial.id}: recorded source revision does not match manifest")
    if summary.get("dirty_worktree") is not source["dirty"]:
        raise RuntimeError(
            f"{trial.id}: recorded dirty-worktree state does not match manifest"
        )
    damage_observation(trial, summary)
    opponent_damage_observation(trial, summary)


def load_summary(trial_root: Path, trial: Trial, source: dict) -> dict:
    try:
        receipt = json.loads((trial_root / "process-receipt.json").read_text())
        if receipt != {"status": "succeeded", "return_code": 0}:
            raise RuntimeError(f"{trial.id}: process receipt is not a successful exit")
        summary = json.loads(_summary_path(trial_root).read_text())
    except (OSError, json.JSONDecodeError) as error:
        raise RuntimeError(f"{trial.id}: cannot read battle summary: {error}") from error
    validate_summary(summary, trial, source)
    return summary


def terminate_process_group(process: subprocess.Popen) -> None:
    """Stop and reap the exact process group created for one trial."""
    try:
        os.killpg(process.pid, signal.SIGTERM)
    except ProcessLookupError:
        pass
    try:
        process.wait(timeout=5)
    except subprocess.TimeoutExpired:
        try:
            os.killpg(process.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        process.wait()


def run_trial(output: Path, trial: Trial, source: dict, timeout: float) -> dict:
    trial_root = output / "trials" / trial.id
    if trial_root.exists():
        return load_summary(trial_root, trial, source)
    trial_root.mkdir(parents=True)
    command = [
        "cargo",
        "run",
        "--release",
        "--quiet",
        "--",
        "--headless",
        "--agent-socket",
        "--width",
        str(EXECUTION["width"]),
        "--height",
        str(EXECUTION["height"]),
        "--scenario",
        "ai-battle",
        "--experiment",
        "--controllers",
        f"{trial.subject},{trial.opponent}",
        "--layout",
        trial.layout,
        "--seed",
        str(trial.seed),
        "--pacing",
        trial.pacing,
        "--trial-seconds",
        str(trial.trial_seconds),
        "--output-root",
        str(trial_root),
    ]
    if trial.swap_sides:
        command.append("--swap-sides")
    with tempfile.TemporaryDirectory(prefix="nano-swarm-experiment-") as runtime_dir:
        os.chmod(runtime_dir, 0o700)
        environment = os.environ.copy()
        environment["XDG_RUNTIME_DIR"] = runtime_dir
        environment[SOURCE_IDENTITY_ENV] = canonical_json(source_identity(source))
        with (trial_root / "process.log").open("wb") as log:
            process = subprocess.Popen(
                command,
                cwd=ROOT,
                env=environment,
                stdout=log,
                stderr=subprocess.STDOUT,
                start_new_session=True,
            )
            try:
                return_code = process.wait(timeout=timeout)
            except subprocess.TimeoutExpired as error:
                terminate_process_group(process)
                write_json(
                    trial_root / "process-receipt.json",
                    {"status": "timeout", "return_code": process.returncode},
                )
                raise RuntimeError(
                    f"{trial.id}: technical timeout after {timeout:g} wall seconds"
                ) from error
            except KeyboardInterrupt:
                terminate_process_group(process)
                write_json(
                    trial_root / "process-receipt.json",
                    {"status": "interrupted", "return_code": process.returncode},
                )
                raise
    if return_code != 0:
        write_json(
            trial_root / "process-receipt.json",
            {"status": "failed", "return_code": return_code},
        )
        raise RuntimeError(f"{trial.id}: runtime failed with exit code {return_code}")
    write_json(
        trial_root / "process-receipt.json",
        {"status": "succeeded", "return_code": 0},
    )
    return load_summary(trial_root, trial, source)


def _aggregate(observations: list[dict], statuses: list[tuple[str, str | None]]) -> dict:
    if not observations:
        raise RuntimeError("cannot aggregate an empty trial set")
    totals = {
        "wins": sum(observation["won"] for observation in observations),
        "losses": sum(observation["lost"] for observation in observations),
        "draws": sum(outcome == "draw" for _, outcome in statuses),
        "unresolved": sum(status == "unresolved" for status, _ in statuses),
        "valid_trials": len(observations),
    }
    totals["win_rate"] = totals["wins"] / totals["valid_trials"]
    for field in (
        "damage_hp",
        "nanobot_damage_hp",
        "structure_damage_hp",
        "gross_damage_hp",
        "gross_nanobot_damage_hp",
        "gross_structure_damage_hp",
        "uncredited_repeat_damage_hp",
        "uncredited_repeat_nanobot_damage_hp",
        "uncredited_repeat_structure_damage_hp",
        "capped_completion_cost_seconds",
    ):
        totals[f"mean_{field}"] = sum(value[field] for value in observations) / len(
            observations
        )
    wins = [value["win_seconds"] for value in observations if value["won"]]
    totals["mean_win_seconds"] = sum(wins) / len(wins) if wins else None
    return totals


def _role_tally(
    trials: list[Trial], summaries: dict[str, dict], *, opponent: bool = False
) -> dict:
    read = opponent_damage_observation if opponent else damage_observation
    return _aggregate(
        [read(trial, summaries[trial.id]) for trial in trials],
        [
            (summaries[trial.id]["status"], summaries[trial.id]["outcome"])
            for trial in trials
        ],
    )


def _slice_report(trials: list[Trial], summaries: dict[str, dict]) -> dict:
    return {
        "controller": _role_tally(trials, summaries),
        "opponent": _role_tally(trials, summaries, opponent=True),
    }


def build_report(
    config: ExperimentConfig,
    source: dict,
    trials: list[Trial],
    summaries: dict[str, dict],
) -> dict:
    expected = build_matrix(config)
    if len(trials) != len(expected) or set(trials) != set(expected):
        raise RuntimeError("report requires the complete configured smoke matrix")
    expected_ids = {trial.id for trial in expected}
    if set(summaries) != expected_ids:
        raise RuntimeError("report requires exactly one summary for every configured trial")

    overall = _slice_report(trials, summaries)
    by_layout = {
        layout: _slice_report(
            [trial for trial in trials if trial.layout == layout], summaries
        )
        for layout in config.layouts
    }
    by_start = {
        label: _slice_report(
            [trial for trial in trials if trial.swap_sides is swapped], summaries
        )
        for label, swapped in (("direct", False), ("swapped", True))
    }
    results = []
    for trial in trials:
        summary = summaries[trial.id]
        results.append(
            {
                "trial": trial_dict(trial),
                "status": summary["status"],
                "outcome": summary["outcome"],
                "simulation_seconds": summary["latest"]["simulation_seconds"],
                "controller": damage_observation(trial, summary),
                "opponent": opponent_damage_observation(trial, summary),
            }
        )

    flags = []
    if overall["controller"]["mean_uncredited_repeat_damage_hp"] > 0:
        flags.append("excluded_repeat_damage:controller")
    if overall["controller"]["wins"] == 0:
        flags.append("no_finishing_evidence:overall")
    for layout, report in by_layout.items():
        if report["controller"]["wins"] == 0:
            flags.append(f"no_finishing_evidence:layout:{layout}")
    for start, report in by_start.items():
        if report["controller"]["wins"] == 0:
            flags.append(f"no_finishing_evidence:start:{start}")

    return {
        "source": source,
        "config": config_dict(config),
        "primary_metric": "scored_damage_total",
        "totals": overall,
        "by_layout": by_layout,
        "by_start": by_start,
        "trials": results,
        "anti_farming_flags": flags,
    }


def comma_values(value: str) -> tuple[str, ...]:
    values = tuple(item.strip() for item in value.split(",") if item.strip())
    if not values:
        raise argparse.ArgumentTypeError("expected at least one comma-separated value")
    return values


def comma_integers(value: str) -> tuple[int, ...]:
    try:
        return tuple(int(item) for item in comma_values(value))
    except ValueError as error:
        raise argparse.ArgumentTypeError("seeds must be comma-separated integers") from error


def parser() -> argparse.ArgumentParser:
    result = argparse.ArgumentParser(description=__doc__)
    result.add_argument("--output", type=Path, required=True)
    result.add_argument("--controller", choices=CONTROLLERS, default="adaptive")
    result.add_argument("--opponent", choices=CONTROLLERS, default="timed")
    result.add_argument("--pacing", choices=PACING, default="baseline")
    result.add_argument("--seeds", type=comma_integers, default=DEFAULT_SEEDS)
    result.add_argument("--layouts", type=comma_values, default=DEFAULT_LAYOUTS)
    result.add_argument("--trial-seconds", type=int, default=600)
    result.add_argument(
        "--timeout",
        type=float,
        default=1800.0,
        help="wall-clock safety timeout per trial",
    )
    result.add_argument("--report-only", action="store_true")
    return result


def main(argv=None) -> int:
    args = parser().parse_args(argv)
    args.output = args.output.resolve()
    config = ExperimentConfig(
        args.controller,
        args.opponent,
        args.pacing,
        args.seeds,
        args.layouts,
        args.trial_seconds,
    )
    try:
        if not math.isfinite(args.timeout) or args.timeout <= 0:
            raise ValueError("timeout seconds must be finite and positive")
        trials = build_matrix(config)
        manifest = (
            load_manifest_for_report(args.output, config, trials)
            if args.report_only
            else load_or_create_manifest(args.output, config, trials)
        )
        source = manifest["source"]
        summaries = {}
        for index, trial in enumerate(trials, 1):
            if not args.report_only:
                verify_source_fingerprint(source, args.output)
            trial_root = args.output / "trials" / trial.id
            if args.report_only:
                summaries[trial.id] = load_summary(trial_root, trial, source)
            else:
                print(f"[{index}/{len(trials)}] {trial.id}", flush=True)
                summaries[trial.id] = run_trial(
                    args.output, trial, source, args.timeout
                )
            if not args.report_only:
                verify_source_fingerprint(source, args.output)
        report = build_report(config, source, trials, summaries)
        write_json(args.output / "report.json", report)
        print(json.dumps(report, indent=2, sort_keys=True))
        return 0
    except (OSError, RuntimeError, ValueError, subprocess.SubprocessError) as error:
        print(f"error: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
