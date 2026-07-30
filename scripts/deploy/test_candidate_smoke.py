from __future__ import annotations

import argparse
import importlib.util
import os
from pathlib import Path
import tempfile
import unittest
from unittest import mock


MODULE_PATH = Path(__file__).with_name("candidate_smoke.py")
SPEC = importlib.util.spec_from_file_location("candidate_smoke", MODULE_PATH)
assert SPEC and SPEC.loader
candidate_smoke = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(candidate_smoke)


class CandidateSmokeTests(unittest.TestCase):
    def test_isolation_values_override_production_env_last(self) -> None:
        base = {
            "MONGODB_DATABASE": "wechatagent",
            "APP_HOST": "0.0.0.0",
            "APP_PORT": "3003",
            "MCP_BASE_URL": "https://real.example.invalid",
            "EVOLUTION_ENABLED": "true",
        }
        env = candidate_smoke.build_child_environment(
            base, "wechatagent_smoke_123", 39083, Path("/tmp/smoke-media")
        )
        self.assertEqual(env["MONGODB_DATABASE"], "wechatagent_smoke_123")
        self.assertEqual(env["APP_HOST"], "127.0.0.1")
        self.assertEqual(env["APP_PORT"], "39083")
        self.assertEqual(env["MCP_BASE_URL"], "http://127.0.0.1:9")
        self.assertEqual(env["EVOLUTION_ENABLED"], "false")

    def test_refuses_system_and_production_databases(self) -> None:
        for database in (
            "admin",
            "config",
            "local",
            "wechatagent",
            "WECHATAGENT",
            "custom_prod",
        ):
            with self.subTest(database=database), self.assertRaises(
                candidate_smoke.SmokeConfigError
            ):
                candidate_smoke.validate_database(database, "custom_prod")
        candidate_smoke.validate_database("wechatagent_smoke_123", "wechatagent")

    @mock.patch.object(candidate_smoke.subprocess, "run")
    def test_queue_preflight_passes_secrets_via_environment_only(self, run: mock.Mock) -> None:
        run.return_value = mock.Mock(returncode=0, stdout='{"agent_tasks":0}\n')
        candidate_smoke.assert_queues_empty(
            {"MONGODB_URI": "mongodb://secret@127.0.0.1:27017"},
            "wechatagent_smoke_123",
        )
        command = run.call_args.args[0]
        env = run.call_args.kwargs["env"]
        self.assertNotIn("secret", " ".join(command))
        self.assertEqual(env["CANDIDATE_SMOKE_DATABASE"], "wechatagent_smoke_123")
        self.assertIn("mongodb://secret", env["MONGODB_URI"])

    @mock.patch.object(candidate_smoke.subprocess, "run")
    def test_queue_preflight_rejects_nonempty_or_unreadable_queue(self, run: mock.Mock) -> None:
        run.return_value = mock.Mock(returncode=23, stdout='{"agent_tasks":1}\n')
        with self.assertRaises(candidate_smoke.SmokeConfigError):
            candidate_smoke.assert_queues_empty(
                {"MONGODB_URI": "mongodb://127.0.0.1:27017"},
                "wechatagent_smoke_123",
            )
        with self.assertRaises(candidate_smoke.SmokeConfigError):
            candidate_smoke.assert_queues_empty({}, "wechatagent_smoke_123")

    @mock.patch.object(candidate_smoke.subprocess, "run")
    def test_queue_preflight_requires_existing_migrated_database(self, run: mock.Mock) -> None:
        run.return_value = mock.Mock(returncode=0, stdout='{"queues":{}}\n')
        candidate_smoke.assert_queues_empty(
            {"MONGODB_URI": "mongodb://127.0.0.1:27017"},
            "wechatagent_smoke_123",
        )
        javascript = run.call_args.args[0][-1]
        self.assertIn("getDBNames", javascript)
        self.assertIn("Array.isArray(rawDatabaseNames)", javascript)
        self.assertIn("rawDatabaseNames.databases", javascript)
        self.assertIn("entry.name", javascript)
        self.assertIn("getCollectionNames", javascript)
        self.assertIn('getCollection("migrations").countDocuments', javascript)

    def test_systemd_command_enforces_network_sandbox_without_secrets(self) -> None:
        args = argparse.Namespace(
            unit="candidate-test.service",
            workdir="/opt/wechatagent",
            candidate="/opt/releases/candidate",
            database="wechatagent_smoke_123",
            env_file="/opt/wechatagent/.env",
            port=39083,
            media_dir="/tmp/smoke-media",
        )
        runner = Path("/run/wechatagent-candidate-smoke-test.py")
        command = candidate_smoke.systemd_run_command(args, runner)
        rendered = " ".join(command)
        self.assertIn("--property=IPAddressDeny=any", command)
        self.assertIn("--property=IPAddressAllow=localhost", command)
        self.assertNotIn("EnvironmentFile", rendered)
        self.assertNotIn("MCP_API_KEY", rendered)
        self.assertNotIn("OPENAI_API_KEY", rendered)
        self.assertIn(str(runner), command)

    def test_stage_runner_copies_content_with_owner_only_permissions(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory) / "source.py"
            source.write_text("print('isolated')\n", encoding="utf-8")
            with mock.patch.object(Path, "chmod", autospec=True) as chmod:
                staged = candidate_smoke.stage_runner(source, Path(directory) / "run")
            chmod.assert_called_once_with(staged, 0o700)
            try:
                self.assertEqual(staged.read_bytes(), source.read_bytes())
                self.assertTrue(staged.name.startswith("wechatagent-candidate-smoke-"))
                self.assertEqual(staged.suffix, ".py")
                if os.name == "posix":
                    staged.chmod(0o700)
                    self.assertEqual(staged.stat().st_mode & 0o077, 0)
            finally:
                staged.unlink(missing_ok=True)

    def test_static_bundle_is_served_byte_for_byte(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            workdir = Path(directory)
            dist = workdir / "frontend" / "dist"
            (dist / "assets").mkdir(parents=True)
            (dist / "index.html").write_bytes(b"<html>candidate</html>")
            (dist / "assets" / "app.js").write_bytes(b"console.log('candidate')")
            responses = {
                "/": b"<html>candidate</html>",
                "/index.html": b"<html>candidate</html>",
                "/assets/app.js": b"console.log('candidate')",
            }
            with mock.patch.object(
                candidate_smoke,
                "fetch_bytes",
                side_effect=lambda _port, path: responses[path],
            ):
                self.assertEqual(
                    candidate_smoke.assert_static_bundle_served(39083, workdir), 2
                )

            responses["/assets/app.js"] = b"stale"
            with mock.patch.object(
                candidate_smoke,
                "fetch_bytes",
                side_effect=lambda _port, path: responses[path],
            ), self.assertRaises(candidate_smoke.SmokeConfigError):
                candidate_smoke.assert_static_bundle_served(39083, workdir)

    def test_dotenv_parser_handles_export_quotes_and_comments(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / ".env"
            path.write_text(
                "export MONGODB_DATABASE=wechatagent\n"
                "TOKEN='not printed # literal'\n"
                "APP_PORT=3003 # comment\n",
                encoding="utf-8",
            )
            parsed = candidate_smoke.parse_dotenv(path)
        self.assertEqual(parsed["MONGODB_DATABASE"], "wechatagent")
        self.assertEqual(parsed["TOKEN"], "not printed # literal")
        self.assertEqual(parsed["APP_PORT"], "3003")


if __name__ == "__main__":
    unittest.main()
