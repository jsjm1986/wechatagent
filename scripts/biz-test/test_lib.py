import hashlib
import hmac
import importlib.util
import unittest
from pathlib import Path
from unittest import mock

MODULE_PATH = Path(__file__).with_name("_lib.py")
SPEC = importlib.util.spec_from_file_location("biztest_lib", MODULE_PATH)
assert SPEC is not None and SPEC.loader is not None
lib = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(lib)


class WebhookSigningTests(unittest.TestCase):
    def test_signs_timestamp_dot_raw_body(self) -> None:
        body = b'{"appId":"app","content":"hello"}'
        expected = hmac.new(b"secret", b"1700000000000." + body, hashlib.sha256).hexdigest()
        self.assertEqual(lib.sign_webhook_body("secret", "1700000000000", body), expected)

    def test_rejects_non_test_sender_before_remote_io(self) -> None:
        with mock.patch.object(lib, "_webhook_secret") as secret:
            with self.assertRaisesRegex(ValueError, "biztest_"):
                lib.send_webhook("app", "real_user", "hello", "biztest_message")
            secret.assert_not_called()

    def test_mongo_failure_is_not_reported_as_cleanup_success(self) -> None:
        with mock.patch.object(lib, "remote_run", return_value=(2, "mongo unavailable")):
            with self.assertRaisesRegex(RuntimeError, "exit code 2"):
                lib.mongo("print(1)")

    def test_biztest_target_validation_rejects_unsafe_database_and_port(self) -> None:
        self.assertEqual(lib._validated_app_port("39085"), 39085)
        self.assertEqual(lib._validated_database("wechatagent_biztest_fixture"),
                         "wechatagent_biztest_fixture")
        for value in ("80", "70000", "not-a-port"):
            with self.assertRaises(ValueError):
                lib._validated_app_port(value)
        for value in ("admin", "config", "local", "bad/name", "db;drop"):
            with self.assertRaises(ValueError):
                lib._validated_database(value)

    def test_api_and_mongo_use_one_explicit_target_binding(self) -> None:
        self.assertEqual(lib.APP_BASE_URL, f"http://127.0.0.1:{lib.BIZTEST_APP_PORT}")
        with mock.patch.object(lib, "BIZTEST_DATABASE", "wechatagent_biztest_fixture"), \
             mock.patch.object(lib, "remote_run", return_value=(0, "null\n")) as remote:
            lib.mongo("print(null)")
        command = remote.call_args.args[0]
        self.assertIn("mongosh wechatagent_biztest_fixture ", command)
        self.assertNotIn("mongosh wechatagent --", command)

    def test_mongo_json_preserves_a_legitimate_json_null(self) -> None:
        with mock.patch.object(lib, "mongo", return_value="null\n"):
            self.assertIsNone(lib.mongo_json("db.collection.findOne({})"))

    def test_mongo_json_distinguishes_unparseable_output(self) -> None:
        with mock.patch.object(lib, "mongo", return_value="mongosh warning\n"):
            self.assertEqual(lib.mongo_json("db.collection.findOne({})"),
                             {"_raw": "mongosh warning"})

    def test_reply_window_waits_until_production_window_expires(self) -> None:
        with mock.patch.object(lib, "reply_window_remaining_ms", side_effect=[1500, 0]), \
             mock.patch.object(lib.time, "sleep") as sleep:
            self.assertTrue(lib.wait_contact_reply_window("biztest_contact"))
        sleep.assert_called_once()

    def test_reply_window_rejects_non_test_contact(self) -> None:
        with self.assertRaisesRegex(ValueError, "biztest_"):
            lib.reply_window_remaining_ms("real-contact")

    def test_send_wait_checks_idle_and_reply_window_before_injection(self) -> None:
        order = []
        def mark(name, value):
            def inner(*args, **kwargs):
                order.append(name)
                return value
            return inner
        with mock.patch.object(lib, "wait_contact_idle", side_effect=mark("idle", True)), \
             mock.patch.object(lib, "wait_contact_reply_window", side_effect=mark("window", True)), \
             mock.patch.object(lib, "run_log_count", side_effect=mark("count", 0)), \
             mock.patch.object(lib, "send_webhook", side_effect=mark("send", {})), \
             mock.patch.object(lib, "wait_run", side_effect=mark("wait", {"run_id": "run-1"})):
            result = lib.send_and_wait("app", "biztest_contact", "hello", "tag")
        self.assertEqual(result, {"run_id": "run-1"})
        self.assertEqual(order, ["idle", "window", "count", "send", "wait"])


    def test_blocked_capability_uses_separate_jsonl_ledger(self) -> None:
        import json
        import tempfile
        with tempfile.TemporaryDirectory() as directory:
            ledger = Path(directory) / "blocked.jsonl"
            with mock.patch.object(lib, "BLOCKED_LEDGER", ledger):
                lib.record_blocked("domain", "vision_import", "no provider", "configure provider")
            row = json.loads(ledger.read_text(encoding="utf-8").strip())
        self.assertEqual(row["capability"], "vision_import")
        self.assertEqual(row["domain"], "domain")

    def test_non_observational_expectation_fails_the_process(self) -> None:
        with mock.patch.object(lib, "record") as record:
            with self.assertRaises(lib.BizTestAssertionError):
                lib.expect(False, "domain", "redline", "evidence", "high")
        record.assert_called_once()

    def test_low_expectation_remains_observational(self) -> None:
        with mock.patch.object(lib, "record") as record:
            self.assertFalse(lib.expect(False, "domain", "observation", "evidence", "low"))
        record.assert_called_once()

    def test_manual_send_polling_stops_on_policy_terminal(self) -> None:
        self.assertFalse(
            lib.manual_send_requires_outbox_poll(
                {"gatewayStatus": "held_by_ai_policy", "reviewApproved": False}
            )
        )
        self.assertFalse(lib.manual_send_requires_outbox_poll({"unexpected": True}))

    def test_manual_send_polling_continues_only_after_durable_acceptance(self) -> None:
        self.assertTrue(
            lib.manual_send_requires_outbox_poll({"gatewayStatus": "outbox_enqueued"})
        )
        self.assertTrue(
            lib.manual_send_requires_outbox_poll({"gateway_status": "skipped_duplicate"})
        )

    def test_campaign_dispatch_body_requires_complete_frozen_identity(self) -> None:
        self.assertEqual(
            lib.campaign_dispatch_body({"specHash": "abc", "specVersion": 2}),
            {"specHash": "abc", "specVersion": 2},
        )
        for value in ({}, {"specHash": "abc"}, {"specHash": "", "specVersion": 1},
                      {"specHash": "abc", "specVersion": 0}):
            with self.assertRaises(ValueError):
                lib.campaign_dispatch_body(value)

    def test_guide_apply_binding_uses_complete_server_identity(self) -> None:
        preview = {"item": {
            "id": "preview-1", "accountId": "account-2", "contactId": "contact-1",
            "candidateHash": "hash-1", "requiresStrongConfirmation": True,
        }}
        self.assertEqual(
            lib.guide_apply_binding(preview),
            {
                "previewId": "preview-1", "expectedAccountId": "account-2",
                "expectedContactId": "contact-1", "candidateHash": "hash-1",
                "confirmGlobalImpact": True,
            },
        )
        with self.assertRaises(ValueError):
            lib.guide_apply_binding({"item": {"id": "preview-1"}})

    def test_domain_profile_identity_requires_exact_row_id(self) -> None:
        self.assertEqual(lib.domain_profile_identity({"id": "row-1", "version": 3}), ("row-1", 3))
        with self.assertRaises(ValueError):
            lib.domain_profile_identity({"version": 3})

    def test_management_binding_requires_server_plan_hash(self) -> None:
        self.assertEqual(
            lib.management_command_binding({"planHash": "frozen"}, "account-2"),
            {"accountId": "account-2", "planHash": "frozen"},
        )
        with self.assertRaises(ValueError):
            lib.management_command_binding({}, "account-2")
        with self.assertRaises(ValueError):
            lib.management_command_binding({"planHash": "frozen"}, "")

    def test_seed_citable_chunk_starts_unverified_and_uses_current_fields(self) -> None:
        with mock.patch.object(lib, "mongo"), mock.patch.object(
            lib, "mongo_json", return_value={"$oid": "64a1f2c3e4b5a697889a0001"}
        ) as mongo_json:
            chunk_id = lib.seed_citable_knowledge_chunk(
                "biztest_policy", "account-2", "营业时间", "周一至周五 09:00-18:00。"
            )
        self.assertEqual(chunk_id, "64a1f2c3e4b5a697889a0001")
        script = mongo_json.call_args.args[0]
        self.assertIn('"body":', script)
        self.assertNotIn('"content":', script)
        self.assertIn('source_anchors=[{sourceQuote:', script)
        self.assertIn('"integrity_status": "needs_review"', script)
        self.assertIn('"status": "draft"', script)

    def test_verify_chunk_binds_server_updated_at(self) -> None:
        with mock.patch.object(
            lib, "get_knowledge_chunk", return_value={"updatedAt": "2026-08-12T01:02:03Z"}
        ), mock.patch.object(lib, "api", return_value={"ok": True}) as api:
            self.assertEqual(lib.verify_knowledge_chunk("chunk-1"), {"ok": True})
        api.assert_called_once_with(
            "POST",
            "/api/operation-knowledge/chunks/chunk-1/verify",
            {"expectedUpdatedAt": "2026-08-12T01:02:03Z", "verifiedClaims": []},
            admin=True,
        )

    def test_patch_chunk_uses_public_request_envelope(self) -> None:
        with mock.patch.object(lib, "api", return_value={"ok": True}) as api:
            self.assertEqual(
                lib.patch_knowledge_chunk("chunk-1", {"summary": "new summary"}),
                {"ok": True},
            )
        api.assert_called_once_with(
            "POST",
            "/api/operation-knowledge/chunks/chunk-1/patch",
            {
                "patch": {"summary": "new summary"},
                "reason": "biz-test lifecycle verification",
            },
            admin=True,
        )

    def test_reset_contact_clears_only_test_stop_barrier(self) -> None:
        with mock.patch.object(lib, "mongo") as mongo:
            lib.reset_contact_conversation("account-2", "biztest_repeat")
        final_script = mongo.call_args_list[-1].args[0]
        self.assertIn("cooldown_until", final_script)
        self.assertIn("operation_policy.explicitStopRequested", final_script)
        with self.assertRaisesRegex(ValueError, "biztest_"):
            lib.reset_contact_conversation("account-2", "real-contact")

    def test_run_scoped_evidence_queries_bind_contact_and_run(self) -> None:
        with mock.patch.object(lib, "mongo_json", return_value=[]) as mongo_json:
            self.assertEqual(lib.outbox_for_run("biztest_contact", "run-1"), [])
            outbox_query = mongo_json.call_args.args[0]
            self.assertIn('contact_wxid:"biztest_contact"', outbox_query)
            self.assertIn('run_id:"run-1"', outbox_query)

            self.assertEqual(lib.ptier_events_for_run("biztest_contact", "run-2"), [])
            event_query = mongo_json.call_args.args[0]
            self.assertIn('contact_wxid:"biztest_contact"', event_query)
            self.assertIn('"details.run_id":"run-2"', event_query)

    def test_run_scoped_evidence_rejects_non_test_contact(self) -> None:
        for lookup in (lib.decision_review_for_run, lib.outbox_for_run, lib.ptier_events_for_run):
            with self.assertRaisesRegex(ValueError, "biztest_"):
                lookup("real-contact", "run-1")


if __name__ == "__main__":
    unittest.main()
