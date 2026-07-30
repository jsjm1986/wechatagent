from __future__ import annotations

import hashlib
import importlib.util
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest import mock


MODULE_PATH = Path(__file__).with_name("delete_hc001_actions_logs.py")
SPEC = importlib.util.spec_from_file_location("delete_hc001_actions_logs", MODULE_PATH)
assert SPEC and SPEC.loader
cleanup = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(cleanup)


class DeleteHc001ActionsLogsTests(unittest.TestCase):
    def write_checkpoint(self, directory: str, objects: dict[str, object]) -> Path:
        path = Path(directory) / "runs.json"
        path.write_text(
            json.dumps({"schema": 1, "kind": "runs", "objects": objects}),
            encoding="utf-8",
        )
        return path

    def expected_fixture(self) -> tuple[dict[str, object], list[int], int, str]:
        objects: dict[str, object] = {
            "101": {"status": "scanned", "matches": 2},
            "102": {"status": "scanned", "matches": 0},
            "103": {"status": "scanned", "matches": 3},
        }
        hits = [101, 103]
        identity = hashlib.sha256(b"101,103").hexdigest()
        return objects, hits, 5, identity

    def load_fixture(self, path: Path) -> list[int]:
        objects, hits, matches, identity = self.expected_fixture()
        with mock.patch.multiple(
            cleanup,
            EXPECTED_OBJECTS=len(objects),
            EXPECTED_HIT_RUNS=len(hits),
            EXPECTED_MATCHES=matches,
            EXPECTED_ID_SET_SHA256=identity,
        ):
            return cleanup.load_confirmed_run_ids(path)

    def test_checkpoint_must_match_complete_immutable_audit(self) -> None:
        objects, hits, _matches, _identity = self.expected_fixture()
        with tempfile.TemporaryDirectory() as directory:
            path = self.write_checkpoint(directory, objects)
            self.assertEqual(
                json.loads(path.read_text(encoding="utf-8"))["objects"], objects
            )
            self.assertEqual(self.load_fixture(path), hits)

            tampered = json.loads(path.read_text(encoding="utf-8"))
            tampered["objects"]["103"]["matches"] = 4
            path.write_text(json.dumps(tampered), encoding="utf-8")
            with self.assertRaises(cleanup.CleanupError):
                self.load_fixture(path)

            tampered["objects"]["103"] = {"status": "http_404", "matches": 0}
            path.write_text(json.dumps(tampered), encoding="utf-8")
            with self.assertRaises(cleanup.CleanupError):
                self.load_fixture(path)

    @mock.patch.object(cleanup, "run_gh")
    def test_preflight_is_read_only_and_verifies_each_run(self, run_gh: mock.Mock) -> None:
        run_gh.side_effect = [
            "",
            '{"nameWithOwner":"jsjm1986/wechatagent"}',
            '{"id":101,"name":"CI","status":"completed"}',
            '{"id":103,"name":"CI","status":"completed"}',
        ]
        self.assertEqual(cleanup.preflight([101, 103]), {"targetRuns": 2, "verifiedRuns": 2})
        rendered = [" ".join(call.args[0]) for call in run_gh.call_args_list]
        self.assertTrue(all("DELETE" not in command for command in rendered))
        self.assertTrue(all(not command.endswith("/logs") for command in rendered))

    @mock.patch.object(cleanup, "run_gh")
    def test_apply_requires_confirmation_before_any_remote_call(self, run_gh: mock.Mock) -> None:
        with self.assertRaises(cleanup.CleanupError):
            cleanup.delete_logs([101, 103], "wrong")
        run_gh.assert_not_called()

    @mock.patch.object(cleanup, "preflight")
    @mock.patch.object(cleanup, "run_gh")
    def test_apply_deletes_only_log_endpoints(
        self, run_gh: mock.Mock, preflight: mock.Mock
    ) -> None:
        preflight.return_value = {"targetRuns": 2, "verifiedRuns": 2}
        result = cleanup.delete_logs([101, 103], cleanup.CONFIRMATION)
        self.assertEqual(result["deletedLogs"], 2)
        commands = [call.args[0] for call in run_gh.call_args_list]
        self.assertEqual(
            commands,
            [
                [
                    "api",
                    "--method",
                    "DELETE",
                    "repos/jsjm1986/wechatagent/actions/runs/101/logs",
                ],
                [
                    "api",
                    "--method",
                    "DELETE",
                    "repos/jsjm1986/wechatagent/actions/runs/103/logs",
                ],
            ],
        )
        self.assertTrue(all(command[-1].endswith("/logs") for command in commands))

    @mock.patch.object(cleanup.subprocess, "run")
    def test_github_cli_output_and_parent_stdin_are_isolated(self, run: mock.Mock) -> None:
        run.return_value = subprocess.CompletedProcess([], 0, stdout="{}", stderr=None)
        cleanup.run_gh(["api", "repos/jsjm1986/wechatagent"], capture_output=True)
        options = run.call_args.kwargs
        self.assertIs(options["stdin"], subprocess.DEVNULL)
        self.assertIs(options["stdout"], subprocess.PIPE)
        self.assertIs(options["stderr"], subprocess.DEVNULL)


if __name__ == "__main__":
    unittest.main()
