from __future__ import annotations

import importlib.util
import io
from pathlib import Path
import subprocess
import unittest
from unittest import mock


MODULE_PATH = Path(__file__).with_name("sync_github_secret.py")
SPEC = importlib.util.spec_from_file_location("sync_github_secret", MODULE_PATH)
assert SPEC and SPEC.loader
sync = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(sync)


class SyncGithubSecretTests(unittest.TestCase):
    def test_target_validation_is_explicit_and_conservative(self) -> None:
        sync.validate_target("jsjm1986/wechatagent", "RSXERMU_KEY")
        for repository in ("wechatagent", "https://github.com/a/b", "a/b/c"):
            with self.subTest(repository=repository), self.assertRaises(sync.SyncError):
                sync.validate_target(repository, "RSXERMU_KEY")
        with self.assertRaises(sync.SyncError):
            sync.validate_target("jsjm1986/wechatagent", "bad-secret")

    def test_stdin_accepts_one_value_and_removes_one_line_ending(self) -> None:
        value = "new-synthetic-secret-value-1234567890"
        with mock.patch.object(sync.sys, "stdin", io.StringIO(value + "\r\n")):
            self.assertEqual(sync.read_secret_from_stdin(), value)
        for invalid in ("short", value + "\nextra", " " + value, value + " "):
            with self.subTest(invalid=invalid), mock.patch.object(
                sync.sys, "stdin", io.StringIO(invalid)
            ), self.assertRaises(sync.SyncError):
                sync.read_secret_from_stdin()

    @mock.patch.object(sync.subprocess, "run")
    def test_secret_is_only_sent_over_stdin_and_child_output_is_discarded(
        self, run: mock.Mock
    ) -> None:
        secret = "new-synthetic-secret-value-1234567890"
        run.return_value = subprocess.CompletedProcess([], 0, stdout=None, stderr=None)
        sync.run_gh(
            ["secret", "set", "RSXERMU_KEY", "--repo", "jsjm1986/wechatagent"],
            input_value=secret,
        )
        command = run.call_args.args[0]
        options = run.call_args.kwargs
        self.assertNotIn(secret, " ".join(command))
        self.assertNotIn(secret, repr(options.get("env")))
        self.assertEqual(options["input"], secret)
        self.assertIs(options["stdout"], subprocess.DEVNULL)
        self.assertIs(options["stderr"], subprocess.DEVNULL)

    @mock.patch.object(sync.subprocess, "run")
    def test_non_secret_calls_cannot_consume_parent_stdin(self, run: mock.Mock) -> None:
        run.return_value = subprocess.CompletedProcess([], 0, stdout=None, stderr=None)
        sync.run_gh(["auth", "status", "--hostname", "github.com"])
        options = run.call_args.kwargs
        self.assertIs(options["stdin"], subprocess.DEVNULL)
        self.assertNotIn("input", options)

    @mock.patch.object(sync, "run_gh")
    def test_apply_requires_confirmation_and_verifies_secret_metadata(
        self, run_gh: mock.Mock
    ) -> None:
        secret = "new-synthetic-secret-value-1234567890"
        with self.assertRaises(sync.SyncError):
            sync.apply("jsjm1986/wechatagent", "RSXERMU_KEY", "wrong")
        run_gh.assert_not_called()

        run_gh.side_effect = [
            mock.Mock(),
            mock.Mock(),
            mock.Mock(),
            mock.Mock(stdout='[{"name":"RSXERMU_KEY"}]'),
        ]
        with mock.patch.object(sync.sys, "stdin", io.StringIO(secret)):
            sync.apply(
                "jsjm1986/wechatagent",
                "RSXERMU_KEY",
                sync.CONFIRMATION,
            )
        set_call = run_gh.call_args_list[2]
        self.assertEqual(
            set_call.args[0],
            [
                "secret",
                "set",
                "RSXERMU_KEY",
                "--repo",
                "jsjm1986/wechatagent",
                "--app",
                "actions",
            ],
        )
        self.assertEqual(set_call.kwargs["input_value"], secret)
        self.assertNotIn(secret, " ".join(set_call.args[0]))

    @mock.patch.object(sync.subprocess, "run")
    def test_cli_failure_does_not_expose_child_output(self, run: mock.Mock) -> None:
        secret = "new-synthetic-secret-value-1234567890"
        run.return_value = subprocess.CompletedProcess(
            [], 1, stdout="remote echoed " + secret, stderr="remote echoed " + secret
        )
        with self.assertRaisesRegex(sync.SyncError, "GitHub CLI refused") as raised:
            sync.run_gh(["secret", "set", "RSXERMU_KEY"], input_value=secret)
        self.assertNotIn(secret, str(raised.exception))


if __name__ == "__main__":
    unittest.main()
