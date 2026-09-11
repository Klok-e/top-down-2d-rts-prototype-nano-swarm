import argparse
import hashlib
import importlib.util
import io
import json
import signal
import tempfile
import unittest
from contextlib import redirect_stderr, redirect_stdout
from pathlib import Path
from unittest import mock


SCRIPT = Path(__file__).parents[1] / "scripts" / "ai_battle_experiment.py"
SPEC = importlib.util.spec_from_file_location("ai_battle_experiment", SCRIPT)
experiment = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(experiment)


def config(**changes):
    values = {
        "controller": "adaptive",
        "opponent": "timed",
        "pacing": "baseline",
        "seeds": (11,),
        "layouts": ("standard", "flanks"),
        "trial_seconds": 600,
    }
    values.update(changes)
    return experiment.ExperimentConfig(**values)


def counters(*, damage=0, gross=None, structure_damage=0):
    gross = damage if gross is None else gross
    return {
        "scored_damage_total": damage,
        "scored_damage_nanobots": damage - structure_damage,
        "scored_damage_structures": structure_damage,
        "effective_damage_total": gross,
        "effective_damage_nanobots": gross - structure_damage,
        "effective_damage_structures": structure_damage,
    }


def damage_sample(
    outcome=None,
    *,
    controller_damage=0,
    controller_gross=None,
    controller_structure_damage=0,
    opponent_damage=0,
    opponent_gross=None,
    opponent_structure_damage=0,
    seconds=600,
    swapped=False,
):
    subject_side = int(swapped)
    opponent_side = int(not swapped)
    swarms = {
        str(subject_side): counters(
            damage=controller_damage,
            gross=controller_gross,
            structure_damage=controller_structure_damage,
        ),
        str(opponent_side): counters(
            damage=opponent_damage,
            gross=opponent_gross,
            structure_damage=opponent_structure_damage,
        ),
    }
    return {
        "schema_version": 3,
        "status": "unresolved" if outcome is None else "completed",
        "outcome": outcome,
        "latest": {"simulation_seconds": seconds, "swarms": swarms},
    }


def recorded_summary(trial, source, **changes):
    outcome = changes.pop("outcome", None)
    seconds = changes.pop("seconds", trial.trial_seconds)
    summary = damage_sample(
        outcome,
        seconds=seconds,
        swapped=trial.swap_sides,
        **changes,
    )
    summary.update(
        {
            "seed": trial.seed,
            "code_revision": source["revision"],
            "dirty_worktree": source["dirty"],
            "gameplay_pacing": experiment.GAMEPLAY_PACING[trial.pacing],
            "timestep_seconds": 1 / 60,
            "experiment": {
                "controllers": [trial.subject, trial.opponent],
                "layout": trial.layout,
                "swap_sides": trial.swap_sides,
                "pacing": trial.pacing,
                "cutoff_seconds": trial.trial_seconds,
                "realtime": False,
            },
        }
    )
    if summary["status"] == "unresolved":
        summary["latest"]["fixed_tick"] = trial.trial_seconds * 60
    return summary


class ExperimentSmokeTests(unittest.TestCase):
    def test_default_cli_describes_the_four_game_smoke_matrix(self):
        args = experiment.parser().parse_args(["--output", "smoke"])

        self.assertEqual(args.controller, "adaptive")
        self.assertEqual(args.opponent, "timed")
        self.assertEqual(args.seeds, (11,))
        self.assertEqual(args.layouts, ("standard", "flanks"))
        self.assertEqual(args.pacing, "baseline")
        self.assertFalse(hasattr(args, "phase"))

        matrix = experiment.build_matrix(
            experiment.ExperimentConfig(
                args.controller,
                args.opponent,
                args.pacing,
                args.seeds,
                args.layouts,
                args.trial_seconds,
            )
        )
        self.assertEqual(len(matrix), 4)
        self.assertEqual(
            [(trial.layout, trial.seed, trial.swap_sides) for trial in matrix],
            [
                ("standard", 11, False),
                ("standard", 11, True),
                ("flanks", 11, False),
                ("flanks", 11, True),
            ],
        )
        self.assertTrue(all(trial.subject == "adaptive" for trial in matrix))
        self.assertTrue(all(trial.opponent == "timed" for trial in matrix))

    def test_removed_research_cli_options_are_not_accepted_as_aliases(self):
        for removed in ("--phase", "--pool", "--candidate", "--incumbent"):
            with self.subTest(removed=removed), redirect_stderr(io.StringIO()):
                with self.assertRaises(SystemExit):
                    experiment.parser().parse_args(
                        ["--output", "smoke", removed, "obsolete"]
                    )

        with redirect_stderr(io.StringIO()), self.assertRaises(SystemExit):
            experiment.parser().parse_args(
                ["--output", "smoke", "--controller", "adaptive_v10"]
            )

    def test_explicit_settings_build_one_subject_against_one_opponent(self):
        chosen = config(
            controller="timed",
            opponent="adaptive",
            pacing="deliberate",
            seeds=(5, 8),
            layouts=("narrows",),
            trial_seconds=90,
        )

        matrix = experiment.build_matrix(chosen)

        self.assertEqual(len(matrix), 4)
        self.assertTrue(all(trial.subject == "timed" for trial in matrix))
        self.assertTrue(all(trial.opponent == "adaptive" for trial in matrix))
        self.assertTrue(all(trial.pacing == "deliberate" for trial in matrix))
        self.assertTrue(all(trial.trial_seconds == 90 for trial in matrix))

    def test_invalid_or_duplicate_matrix_inputs_are_rejected(self):
        cases = (
            (config(seeds=()), "seeds"),
            (config(seeds=(11, 11)), "duplicate seeds"),
            (config(layouts=("standard", "standard")), "duplicate layouts"),
            (config(layouts=("unknown",)), "layouts"),
            (config(trial_seconds=0), "trial seconds"),
        )
        for chosen, message in cases:
            with self.subTest(chosen=chosen), self.assertRaisesRegex(ValueError, message):
                experiment.build_matrix(chosen)

    def test_nonfinite_timeout_is_rejected_before_manifest_work(self):
        for value in ("nan", "inf", "-inf"):
            with self.subTest(value=value), redirect_stderr(io.StringIO()) as errors, \
                 mock.patch.object(experiment, "load_or_create_manifest") as load:
                exit_code = experiment.main(
                    ["--output", "/tmp/no-ai-battle-run", f"--timeout={value}"]
                )
            self.assertEqual(exit_code, 2)
            self.assertIn("finite", errors.getvalue())
            load.assert_not_called()

    def test_trial_launch_receives_manifest_identity_and_explicit_game_settings(self):
        trial = experiment.build_matrix(config(pacing="deliberate"))[0]
        source = {
            "revision": "abc",
            "dirty": True,
            "content_sha256": "digest",
            "untracked": ["notes.md"],
            "bundle": {"ignored": "for-build-identity"},
        }
        process = mock.Mock()
        process.wait.return_value = 1
        with tempfile.TemporaryDirectory() as directory, mock.patch.object(
            experiment.subprocess, "Popen", return_value=process
        ) as popen:
            with self.assertRaisesRegex(RuntimeError, "runtime failed"):
                experiment.run_trial(Path(directory), trial, source, 10)

        self.assertEqual(
            popen.call_args.kwargs["env"][experiment.SOURCE_IDENTITY_ENV],
            '{"content_sha256":"digest","dirty":true,"revision":"abc",'
            '"untracked":["notes.md"]}',
        )
        command = popen.call_args.args[0]
        self.assertIn("adaptive,timed", command)
        self.assertEqual(command[command.index("--pacing") + 1], "deliberate")
        self.assertEqual(command[command.index("--layout") + 1], "standard")
        self.assertEqual(command[command.index("--seed") + 1], "11")
        self.assertEqual(command[command.index("--trial-seconds") + 1], "600")

    def test_interrupt_terminates_owned_process_group_and_records_receipt(self):
        trial = experiment.build_matrix(config(layouts=("standard",)))[0]
        source = {
            "revision": "abc",
            "dirty": False,
            "content_sha256": "digest",
            "untracked": [],
        }
        process = mock.Mock(pid=1234, returncode=-signal.SIGTERM)
        process.wait.side_effect = [KeyboardInterrupt(), -signal.SIGTERM]
        with tempfile.TemporaryDirectory() as directory, mock.patch.object(
            experiment.subprocess, "Popen", return_value=process
        ), mock.patch.object(experiment.os, "killpg") as killpg:
            with self.assertRaises(KeyboardInterrupt):
                experiment.run_trial(Path(directory), trial, source, 10)
            receipt = json.loads(
                (
                    Path(directory)
                    / "trials"
                    / trial.id
                    / "process-receipt.json"
                ).read_text()
            )

        killpg.assert_called_once_with(1234, signal.SIGTERM)
        self.assertEqual(process.wait.call_args_list, [mock.call(timeout=10), mock.call(timeout=5)])
        self.assertEqual(
            receipt, {"status": "interrupted", "return_code": -signal.SIGTERM}
        )

    def test_active_manifest_rejects_unignored_repository_output(self):
        chosen = config(layouts=("standard",))
        trials = experiment.build_matrix(chosen)
        with tempfile.TemporaryDirectory(
            prefix="experiment-unignored-", dir=experiment.ROOT
        ) as directory:
            output = Path(directory)
            with self.assertRaisesRegex(RuntimeError, "must be Git-ignored"):
                experiment.load_or_create_manifest(output, chosen, trials)
            self.assertFalse((output / "manifest.json").exists())
            self.assertFalse((output / "source").exists())

    def test_active_manifest_allows_git_ignored_repository_output(self):
        experiment.validate_active_output_location(
            experiment.ROOT / "target" / "ai72" / "future-run"
        )

    def test_manifest_freezes_current_config_source_and_four_trials(self):
        chosen = config()
        trials = experiment.build_matrix(chosen)
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory)
            manifest = experiment.load_or_create_manifest(output, chosen, trials)

            self.assertEqual(manifest["schema_version"], 3)
            self.assertEqual(manifest["config"], experiment.config_dict(chosen))
            self.assertEqual(len(manifest["trials"]), 4)
            self.assertNotIn("phase", manifest["config"])
            self.assertNotIn("candidate", manifest["config"])
            self.assertNotIn("incumbent", manifest["config"])
            self.assertNotIn("pool", manifest["config"])
            self.assertNotIn("numerical_evaluation_criteria", manifest)
            experiment.validate_source_bundle(output, manifest["source"])

    def test_manifest_source_bundle_round_trips_exact_snapshot_bytes(self):
        chosen = config(layouts=("standard",))
        trials = experiment.build_matrix(chosen)
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory)
            manifest = experiment.load_or_create_manifest(output, chosen, trials)

            self.assertEqual(
                (output / "source" / "tracked.patch").read_bytes(),
                experiment._git("diff", "--binary", "HEAD"),
            )
            for relative in manifest["source"]["untracked"]:
                self.assertEqual(
                    (output / "source" / "untracked" / relative).read_bytes(),
                    (experiment.ROOT / relative).read_bytes(),
                )

    def test_source_bundle_corruption_and_path_traversal_are_rejected(self):
        chosen = config(layouts=("standard",))
        trials = experiment.build_matrix(chosen)
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory)
            manifest = experiment.load_or_create_manifest(output, chosen, trials)
            with (output / "source" / "tracked.patch").open("ab") as patch:
                patch.write(b"corruption")
            with self.assertRaisesRegex(RuntimeError, "hash does not match"):
                experiment.validate_source_bundle(output, manifest["source"])

        unsafe = {
            "revision": "abc",
            "dirty": False,
            "content_sha256": hashlib.sha256(b"").hexdigest(),
            "untracked": [],
            "bundle": {
                "tracked_patch": "../outside",
                "untracked_root": "source/untracked",
            },
        }
        with tempfile.TemporaryDirectory() as directory, self.assertRaisesRegex(
            RuntimeError, "unsafe path"
        ):
            experiment.validate_source_bundle(Path(directory), unsafe)

    def test_current_report_manifest_rejects_schema_or_config_mismatch(self):
        chosen = config(layouts=("standard",))
        trials = experiment.build_matrix(chosen)
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory)
            manifest = experiment.load_or_create_manifest(output, chosen, trials)
            for field, value in (("schema_version", 2), ("config", {})):
                with self.subTest(field=field):
                    changed = json.loads(json.dumps(manifest))
                    changed[field] = value
                    (output / "manifest.json").write_text(json.dumps(changed))
                    with self.assertRaisesRegex(RuntimeError, "does not match"):
                        experiment.load_manifest_for_report(output, chosen, trials)
            (output / "manifest.json").write_text(json.dumps(manifest))
            self.assertEqual(
                experiment.load_manifest_for_report(output, chosen, trials), manifest
            )

    def test_summary_validation_binds_identity_settings_cutoff_and_damage_schema(self):
        trial = experiment.build_matrix(config(layouts=("standard",)))[0]
        source = {"revision": "abc", "dirty": True}
        valid = recorded_summary(trial, source, controller_damage=100, opponent_damage=50)
        experiment.validate_summary(valid, trial, source)

        mutations = (
            ("status", lambda value: value.update(status="interrupted"), "invalid run status"),
            (
                "controllers",
                lambda value: value["experiment"].update(controllers=["timed", "adaptive"]),
                "does not match manifest",
            ),
            (
                "pacing",
                lambda value: value.update(gameplay_pacing={}),
                "numeric gameplay pacing",
            ),
            (
                "source",
                lambda value: value.update(code_revision="other"),
                "source revision",
            ),
            (
                "cutoff",
                lambda value: value["latest"].update(fixed_tick=35_999),
                "did not reach",
            ),
            (
                "damage",
                lambda value: value["latest"]["swarms"]["0"].pop(
                    "scored_damage_total"
                ),
                "scored_damage",
            ),
        )
        for label, mutate, message in mutations:
            with self.subTest(label=label):
                changed = json.loads(json.dumps(valid))
                mutate(changed)
                with self.assertRaisesRegex(RuntimeError, message):
                    experiment.validate_summary(changed, trial, source)

    def test_damage_validation_rejects_overcredit_bad_subtotals_and_nonfinite_time(self):
        trial = experiment.build_matrix(config(layouts=("standard",)))[0]
        for fault in ("overcredit", "bad_sum", "nonfinite"):
            with self.subTest(fault=fault):
                summary = damage_sample(controller_damage=100)
                subject = summary["latest"]["swarms"]["0"]
                if fault == "overcredit":
                    subject["scored_damage_total"] = 101
                    subject["scored_damage_nanobots"] = 101
                elif fault == "bad_sum":
                    subject["scored_damage_structures"] = 1
                else:
                    summary["latest"]["simulation_seconds"] = float("nan")
                with self.assertRaises(RuntimeError):
                    experiment.damage_observation(trial, summary)

    def test_report_uses_actual_swapped_sides_and_preserves_damage_diagnostics(self):
        chosen = config()
        trials = experiment.build_matrix(chosen)
        summaries = {}
        outcomes = {
            ("standard", False): "swarm_0_wins",
            ("standard", True): "swarm_0_wins",
            ("flanks", False): "draw",
            ("flanks", True): None,
        }
        for trial in trials:
            summaries[trial.id] = damage_sample(
                outcomes[(trial.layout, trial.swap_sides)],
                controller_damage=180,
                controller_gross=240,
                controller_structure_damage=20,
                opponent_damage=90,
                opponent_gross=100,
                opponent_structure_damage=10,
                seconds=(120 if outcomes[(trial.layout, trial.swap_sides)] else 600),
                swapped=trial.swap_sides,
            )

        report = experiment.build_report(chosen, {"revision": "abc"}, trials, summaries)

        controller = report["totals"]["controller"]
        self.assertEqual(controller["wins"], 1)
        self.assertEqual(controller["losses"], 1)
        self.assertEqual(controller["draws"], 1)
        self.assertEqual(controller["unresolved"], 1)
        self.assertEqual(controller["mean_damage_hp"], 180)
        self.assertEqual(controller["mean_gross_damage_hp"], 240)
        self.assertEqual(controller["mean_gross_nanobot_damage_hp"], 220)
        self.assertEqual(controller["mean_gross_structure_damage_hp"], 20)
        self.assertEqual(controller["mean_uncredited_repeat_damage_hp"], 60)
        self.assertEqual(report["totals"]["opponent"]["mean_damage_hp"], 90)
        self.assertEqual(report["by_layout"]["standard"]["controller"]["wins"], 1)
        self.assertEqual(report["by_start"]["swapped"]["controller"]["losses"], 1)
        self.assertEqual(len(report["trials"]), 4)
        self.assertIn("excluded_repeat_damage:controller", report["anti_farming_flags"])
        for obsolete in (
            "promotion_evaluated",
            "numerical_evaluation",
            "paired_comparison",
            "ladder",
            "damage_comparison",
        ):
            self.assertNotIn(obsolete, report)

    def test_report_flags_missing_finishes_by_layout_and_start(self):
        chosen = config()
        trials = experiment.build_matrix(chosen)
        summaries = {
            trial.id: damage_sample(
                "swarm_0_wins" if trial.layout == "standard" and not trial.swap_sides else None,
                controller_damage=100,
                seconds=(100 if trial.layout == "standard" and not trial.swap_sides else 600),
                swapped=trial.swap_sides,
            )
            for trial in trials
        }

        report = experiment.build_report(chosen, {}, trials, summaries)

        self.assertNotIn("no_finishing_evidence:overall", report["anti_farming_flags"])
        self.assertIn("no_finishing_evidence:layout:flanks", report["anti_farming_flags"])
        self.assertIn("no_finishing_evidence:start:swapped", report["anti_farming_flags"])

    def test_report_rejects_partial_duplicate_or_extra_trial_evidence(self):
        chosen = config()
        trials = experiment.build_matrix(chosen)
        summaries = {
            trial.id: damage_sample(swapped=trial.swap_sides) for trial in trials
        }
        with self.assertRaisesRegex(RuntimeError, "complete configured"):
            experiment.build_report(chosen, {}, trials[:-1], summaries)
        with self.assertRaisesRegex(RuntimeError, "complete configured"):
            experiment.build_report(chosen, {}, trials + trials[:1], summaries)
        with self.assertRaisesRegex(RuntimeError, "exactly one summary"):
            experiment.build_report(chosen, {}, trials, {**summaries, "extra": {}})

    def test_report_only_fails_when_manifest_trial_has_no_summary(self):
        with tempfile.TemporaryDirectory() as directory, redirect_stderr(
            io.StringIO()
        ) as errors:
            chosen = config()
            trials = experiment.build_matrix(chosen)
            experiment.load_or_create_manifest(Path(directory), chosen, trials)
            first = trials[0]
            trial_root = Path(directory) / "trials" / first.id
            trial_root.mkdir(parents=True)
            (trial_root / "process-receipt.json").write_text(
                json.dumps({"status": "succeeded", "return_code": 0})
            )
            exit_code = experiment.main(["--output", directory, "--report-only"])

        self.assertEqual(exit_code, 2)
        self.assertIn("expected exactly one battle summary", errors.getvalue())

    def test_report_only_rebuilds_report_from_complete_current_artifacts(self):
        chosen = config()
        trials = experiment.build_matrix(chosen)
        with tempfile.TemporaryDirectory() as directory, redirect_stdout(io.StringIO()):
            output = Path(directory)
            manifest = experiment.load_or_create_manifest(output, chosen, trials)
            for trial in trials:
                trial_root = output / "trials" / trial.id
                summary_root = trial_root / "battle-fixture"
                summary_root.mkdir(parents=True)
                (trial_root / "process-receipt.json").write_text(
                    json.dumps({"status": "succeeded", "return_code": 0})
                )
                (summary_root / "summary.json").write_text(
                    json.dumps(
                        recorded_summary(
                            trial,
                            manifest["source"],
                            controller_damage=100,
                            opponent_damage=50,
                        )
                    )
                )

            exit_code = experiment.main(["--output", directory, "--report-only"])
            report = json.loads((output / "report.json").read_text())

        self.assertEqual(exit_code, 0)
        self.assertEqual(report["totals"]["controller"]["valid_trials"], 4)
        self.assertEqual(report["totals"]["controller"]["mean_damage_hp"], 100)
        self.assertEqual(report["totals"]["opponent"]["mean_damage_hp"], 50)
        self.assertNotIn("numerical_evaluation", report)

    def test_resume_rejects_summary_without_successful_process_receipt(self):
        trial = experiment.build_matrix(config(layouts=("standard",)))[0]
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "process-receipt.json").write_text(
                json.dumps({"status": "failed", "return_code": 1})
            )
            with self.assertRaisesRegex(RuntimeError, "not a successful exit"):
                experiment.load_summary(root, trial, {"revision": "abc", "dirty": True})

    def test_source_change_aborts_before_next_trial_and_report(self):
        chosen = config()
        trials = experiment.build_matrix(chosen)
        source = {
            "revision": "abc",
            "dirty": True,
            "content_sha256": "same",
            "untracked": [],
        }
        manifest = {
            "schema_version": 3,
            "source": source,
            "config": experiment.config_dict(chosen),
            "execution": experiment.EXECUTION,
            "trials": [experiment.trial_dict(trial) for trial in trials],
        }
        observed_source = source

        def run_trial_then_change_source(*_arguments):
            nonlocal observed_source
            observed_source = {**source, "content_sha256": "changed"}
            return damage_sample()

        with tempfile.TemporaryDirectory() as directory, redirect_stdout(
            io.StringIO()
        ), redirect_stderr(io.StringIO()) as errors, mock.patch.object(
            experiment, "load_or_create_manifest", return_value=manifest
        ), mock.patch.object(
            experiment,
            "source_fingerprint",
            side_effect=lambda _output: observed_source,
        ), mock.patch.object(
            experiment, "run_trial", side_effect=run_trial_then_change_source
        ) as run_trial:
            exit_code = experiment.main(["--output", directory])
            report_exists = (Path(directory) / "report.json").exists()

        self.assertEqual(exit_code, 2)
        self.assertIn("source changed", errors.getvalue())
        self.assertEqual(run_trial.call_count, 1)
        self.assertFalse(report_exists)


if __name__ == "__main__":
    unittest.main()
