from __future__ import annotations

import importlib.util
import json
import os
from pathlib import Path
import stat
import tempfile
import unittest
from unittest import mock


MODULE_PATH = Path(__file__).with_name("rotate_llm_credential.py")
SPEC = importlib.util.spec_from_file_location("rotate_llm_credential", MODULE_PATH)
assert SPEC and SPEC.loader
rotation = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(rotation)


class RotateLlmCredentialTests(unittest.TestCase):
    def assert_private_mode(self, path: Path, expected: int) -> None:
        if os.name == "posix":
            self.assertEqual(stat.S_IMODE(path.stat().st_mode), expected)

    def test_non_posix_hosts_are_rejected(self) -> None:
        with mock.patch.object(rotation.os, "name", "nt"), self.assertRaises(
            rotation.RotationError
        ):
            rotation.require_posix_host()

    def test_read_new_key_requires_private_regular_file(self) -> None:
        if os.name != "posix":
            self.skipTest("POSIX permission semantics are verified on Linux")
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "new.key"
            path.write_text("N" * 32 + "-synthetic", encoding="utf-8")
            path.chmod(0o600)
            self.assertEqual(rotation.read_new_key(path), "N" * 32 + "-synthetic")
            path.chmod(0o640)
            with self.assertRaises(rotation.RotationError):
                rotation.read_new_key(path)

    def test_replace_env_key_preserves_other_bytes_and_requires_one_assignment(self) -> None:
        original = b"APP_PORT=3003\nOPENAI_API_KEY=old-value\nOPENAI_MODEL=model-a\n"
        replaced = rotation.replace_env_key(original, "new-synthetic-value-1234567890")
        self.assertEqual(
            replaced,
            b"APP_PORT=3003\nOPENAI_API_KEY='new-synthetic-value-1234567890'\n"
            b"OPENAI_MODEL=model-a\n",
        )
        with self.assertRaises(rotation.RotationError):
            rotation.replace_env_key(b"APP_PORT=3003\n", "new-synthetic-value-1234567890")

    def test_replace_env_config_updates_exactly_the_candidate_triplet(self) -> None:
        original = (
            b"APP_PORT=3003\n"
            b"OPENAI_BASE_URL=https://old.invalid/v1\n"
            b"OPENAI_API_KEY=old-value\n"
            b"OPENAI_MODEL=old-model\n"
            b"UNCHANGED='same bytes'\n"
        )
        replaced = rotation.replace_env_config(
            original,
            "new-synthetic-value-1234567890",
            "https://new.invalid/v1",
            "new-model",
        )
        self.assertEqual(
            replaced,
            b"APP_PORT=3003\n"
            b"OPENAI_BASE_URL='https://new.invalid/v1'\n"
            b"OPENAI_API_KEY='new-synthetic-value-1234567890'\n"
            b"OPENAI_MODEL='new-model'\n"
            b"UNCHANGED='same bytes'\n",
        )

    def test_candidate_config_is_explicit_and_conservative(self) -> None:
        self.assertEqual(
            rotation.candidate_config(
                "https://gateway.invalid/v1/", "gpt-model", "openai"
            ),
            {
                "baseUrl": "https://gateway.invalid/v1",
                "model": "gpt-model",
                "format": "openai",
            },
        )
        for url in (
            "http://gateway.invalid/v1",
            "https://user:pass@gateway.invalid/v1",
            "https://gateway.invalid/v1?token=value",
        ):
            with self.assertRaises(rotation.RotationError):
                rotation.candidate_config(url, "gpt-model", "openai")

    def test_rotation_plan_covers_old_key_rows_and_unique_active(self) -> None:
        old = {
            "id": "1",
            "apiKey": "old",
            "format": "openai",
            "baseUrl": "https://old.invalid/v1",
            "model": "old-model",
            "isActive": False,
        }
        active = {
            "id": "2",
            "apiKey": "other",
            "format": "openai",
            "baseUrl": "https://active.invalid/v1",
            "model": "active-model",
            "isActive": True,
        }
        state = {"oldRows": [old], "activeRows": [active]}
        self.assertEqual(rotation.build_rotation_plan(state), [old, active])
        with self.assertRaises(rotation.RotationError):
            rotation.build_rotation_plan({"oldRows": [old], "activeRows": []})

    def test_provider_state_matching_is_field_exact(self) -> None:
        original = {
            "id": "1",
            "apiKey": "old",
            "format": "openai",
            "baseUrl": "https://old.invalid/v1",
            "model": "old-model",
            "isActive": True,
        }
        candidate = {
            "apiKey": "new",
            "format": "openai",
            "baseUrl": "https://new.invalid/v1",
            "model": "new-model",
        }
        migrated = {**original, **candidate}
        state = {"oldRows": [], "newRows": [migrated], "activeRows": [migrated]}
        self.assertTrue(
            rotation.provider_state_matches(
                state, [original], candidate, migrated=True
            )
        )
        migrated["model"] = "drifted"
        self.assertFalse(
            rotation.provider_state_matches(
                state, [original], candidate, migrated=True
            )
        )

    def test_atomic_write_preserves_private_mode(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / ".env"
            path.write_bytes(b"old")
            path.chmod(0o600)
            rotation.atomic_write(path, b"new")
            self.assertEqual(path.read_bytes(), b"new")
            self.assert_private_mode(path, 0o600)

    def test_probe_uses_protocol_specific_path_and_headers(self) -> None:
        response = mock.MagicMock()
        response.status = 200
        response.read.return_value = b"{}"
        response.__enter__.return_value = response
        with mock.patch.object(rotation.urllib.request, "urlopen", return_value=response) as open_url:
            self.assertEqual(
                rotation.probe("https://chat.invalid/v1", "m", "chat", "secret-value", 5),
                "accepted",
            )
            chat_request = open_url.call_args.args[0]
            self.assertEqual(chat_request.full_url, "https://chat.invalid/v1/chat/completions")
            self.assertIn("Bearer secret-value", chat_request.headers.values())

            self.assertEqual(
                rotation.probe("https://messages.invalid", "m", "messages", "secret-value", 5),
                "accepted",
            )
            message_request = open_url.call_args.args[0]
            self.assertEqual(message_request.full_url, "https://messages.invalid/v1/messages")
            self.assertEqual(message_request.headers["X-api-key"], "secret-value")
            self.assertEqual(message_request.headers["Anthropic-version"], "2023-06-01")

    def test_probe_output_never_contains_key_or_endpoint(self) -> None:
        with mock.patch.object(
            rotation.urllib.request,
            "urlopen",
            side_effect=OSError("secret-value https://private.invalid"),
        ):
            result = rotation.probe(
                "https://private.invalid", "m", "chat", "secret-value", 5
            )
        self.assertEqual(result, "transport_oserror")
        self.assertNotIn("secret", result)
        self.assertNotIn("private", result)

    def test_preflight_requires_real_success_and_active_success(self) -> None:
        with self.assertRaises(rotation.RotationError):
            rotation.require_usable_probe_results(
                [{"active": False, "result": "accepted_rate_limited"}]
            )
        with self.assertRaises(rotation.RotationError):
            rotation.require_usable_probe_results(
                [
                    {"active": False, "result": "accepted"},
                    {"active": True, "result": "http_530"},
                ]
            )
        rotation.require_usable_probe_results(
            [{"active": True, "result": "accepted"}]
        )

    def test_evidence_contains_only_typed_counts_and_statuses(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            target = Path(directory) / "evidence"
            result = {
                "oldProviderRefs": 2,
                "probeResults": ["accepted"],
                "rotationApplied": False,
            }
            rotation.write_evidence(target, result)
            content = (target / "result.json").read_text(encoding="utf-8")
            self.assertEqual(json.loads(content), result)
            self.assert_private_mode(target / "result.json", 0o600)

    @mock.patch.object(rotation, "run")
    def test_mongo_secrets_are_passed_via_environment_not_argv(self, run: mock.Mock) -> None:
        run.return_value = mock.Mock(stdout="{}\n")
        rotation.mongo_eval(
            {"MONGODB_URI": "mongodb://user:secret@db.invalid/app"},
            "print('{}')",
            {"HC001_NEW_KEY": "new-secret-value"},
        )
        command = " ".join(run.call_args.args[0])
        environment = run.call_args.kwargs["env"]
        self.assertNotIn("new-secret-value", command)
        self.assertNotIn("mongodb://user:secret", command)
        self.assertEqual(environment["HC001_NEW_KEY"], "new-secret-value")
        self.assertIn("mongodb://user:secret", environment["MONGODB_URI"])


if __name__ == "__main__":
    unittest.main()
