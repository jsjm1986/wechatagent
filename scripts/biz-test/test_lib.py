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

    def test_default_profile_restore_only_demotes_active_biztest_row(self) -> None:
        result = {"status": "restored_default", "modifiedCount": 1}
        with mock.patch.object(lib, "mongo_json", return_value=result) as mongo_json:
            self.assertEqual(lib.restore_default_domain_profile_fallback(), result)
        script = mongo_json.call_args.args[0]
        self.assertIn("active.profile_id.startsWith('biztest_')", script)
        self.assertIn("profile_id:/^biztest_/", script)
        self.assertIn("domain_profile\\u0000default", script)
        self.assertIn("remaining!==0", script)

    def test_default_profile_restore_rejects_unverified_result(self) -> None:
        with mock.patch.object(lib, "mongo_json", return_value={"status": "unknown"}):
            with self.assertRaisesRegex(RuntimeError, "failed to restore"):
                lib.restore_default_domain_profile_fallback()

    def test_industry_restore_marker_serializes_default_fallback_identity(self) -> None:
        module_path = Path(__file__).with_name("batch_b_industry.py")
        spec = importlib.util.spec_from_file_location("biztest_batch_b_industry", module_path)
        assert spec is not None and spec.loader is not None
        industry = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(industry)
        write_marker = getattr(industry, "_write_restore_marker", None)
        self.assertIsNotNone(write_marker)
        if write_marker is None:
            return
        with mock.patch.object(industry._lib, "mongo") as mongo:
            write_marker(None)
        self.assertEqual(
            mongo.call_args.args[0],
            'db.biztest_control.replaceOne('
            '{_id:"biztest_industry_profile_restore"},'
            '{_id:"biztest_industry_profile_restore",original_active_id:null,'
            'workspace_id:"default",created_at:new Date()}, {upsert:true})',
        )

    def test_reply_window_waits_until_production_window_expires(self) -> None:
        with mock.patch.object(lib, "reply_window_remaining_ms", side_effect=[1500, 0]), \
             mock.patch.object(lib.time, "sleep") as sleep:
            self.assertTrue(lib.wait_contact_reply_window("biztest_contact"))
        sleep.assert_called_once()

    def test_reply_window_rejects_non_test_contact(self) -> None:
        with self.assertRaisesRegex(ValueError, "biztest_"):
            lib.reply_window_remaining_ms("real-contact")

    def test_reply_window_queries_delivery_rate_limit_anchor(self) -> None:
        with mock.patch.object(lib, "_default_min_reply_interval_seconds", return_value=20), \
             mock.patch.object(lib, "mongo_json", return_value=0) as mongo_json:
            self.assertEqual(lib.reply_window_remaining_ms("biztest_contact"), 0)
        query = mongo_json.call_args.args[0]
        self.assertIn("last_agent_run_at", query)
        self.assertNotIn("last_outbound_at", query)

    def test_send_wait_checks_idle_and_reply_window_before_injection(self) -> None:
        order = []
        def mark(name, value):
            def inner(*args, **kwargs):
                order.append(name)
                return value
            return inner
        with mock.patch.object(lib, "wait_contact_idle", side_effect=mark("idle", True)), \
             mock.patch.object(lib, "wait_contact_reply_window", side_effect=mark("window", True)), \
             mock.patch.object(lib, "send_webhook", side_effect=mark("send", {})), \
             mock.patch.object(lib, "wait_run", side_effect=mark("wait", {"run_id": "run-1"})) as wait:
            result = lib.send_and_wait("app", "biztest_contact", "hello", "tag")
        self.assertEqual(result, {"run_id": "run-1"})
        self.assertEqual(order, ["idle", "window", "send", "wait"])
        source_event_id = wait.call_args.args[1]
        self.assertTrue(source_event_id.startswith("biztest_tag_"))

    def test_wait_run_binds_source_event_and_ignores_open_envelope(self) -> None:
        terminal = {
            "run_id": "run-1",
            "status": "outbox_enqueued",
            "lifecycle": "completed",
        }
        with mock.patch.object(lib, "mongo_json", side_effect=[[], [terminal]]) as mongo_json, \
             mock.patch.object(lib.time, "sleep") as sleep:
            self.assertEqual(
                lib.wait_run(
                    "biztest_contact", "biztest_msg_1",
                    max_wait=30, poll=1, diagnose_on_timeout=False,
                ),
                terminal,
            )
        self.assertEqual(mongo_json.call_count, 2)
        query = mongo_json.call_args.args[0]
        self.assertIn('source_event_id:"biztest_msg_1"', query)
        self.assertIn("$nin", query)
        self.assertIn("started", query)
        self.assertIn("running", query)
        sleep.assert_called_once_with(1)

    def test_contact_idle_counts_open_run_envelopes(self) -> None:
        with mock.patch.object(lib, "mongo_json", return_value=1) as mongo_json:
            self.assertEqual(lib.inflight_inbound("biztest_contact"), 1)
        query = mongo_json.call_args.args[0]
        self.assertIn("agent_run_logs.countDocuments", query)
        self.assertIn("$in", query)
        self.assertIn("started", query)
        self.assertIn("running", query)
        self.assertNotIn("conversation_messages", query)

    def test_send_wait_retries_transient_failed_run_envelope(self) -> None:
        transient = {
            "run_id": "run-failed",
            "status": "internal_error",
            "lifecycle": "failed_after_decision",
            "gateway_result": {
                "reason": "gateway_error: llm unavailable (network_error) after 0 retries",
            },
        }
        success = {
            "run_id": "run-success",
            "status": "outbox_enqueued",
            "lifecycle": "completed",
        }
        with mock.patch.object(lib, "wait_contact_idle", return_value=True), \
             mock.patch.object(lib, "wait_contact_reply_window", return_value=True), \
             mock.patch.object(lib, "send_webhook") as send_webhook, \
             mock.patch.object(lib, "wait_run", side_effect=[transient, success]), \
             mock.patch.object(lib.time, "sleep") as sleep:
            result = lib.send_and_wait(
                "app", "biztest_contact", "hello", "tag",
                endpoint_retries=1, retry_gap=25,
            )
        self.assertEqual(result, success)
        self.assertEqual(send_webhook.call_count, 2)
        sleep.assert_called_once_with(25)

    def test_knowledge_route_for_run_reads_exact_selected_chunk_ids(self) -> None:
        helper = getattr(lib, "knowledge_route_for_run", None)
        self.assertIsNotNone(helper)
        if helper is None:
            return
        row = {
            "run_id": "run-1",
            "status": "outbox_enqueued",
            "knowledge_route": {
                "selectedChunkIds": ["64a1f2c3e4b5a697889a0001"],
                "knowledgeCoverage": "enough",
            },
        }
        with mock.patch.object(lib, "mongo_json", return_value=[row]) as mongo_json:
            self.assertEqual(
                helper("biztest_contact", "run-1"),
                row,
            )
        query = mongo_json.call_args.args[0]
        self.assertIn("agent_run_logs.find", query)
        self.assertIn("contact_wxid", query)
        self.assertIn("run_id", query)
        self.assertIn("knowledge_route", query)

    def test_memory_candidates_for_runs_bind_contact_and_exact_runs(self) -> None:
        helper = getattr(lib, "memory_candidates_for_runs", None)
        self.assertIsNotNone(helper)
        if helper is None:
            return
        rows = [
            {
                "run_id": "run-2",
                "source": "projection",
                "status": "pending",
                "candidates": [{"text": "孩子8岁"}],
            },
        ]
        with mock.patch.object(lib, "mongo_json", return_value=rows) as mongo_json:
            self.assertEqual(
                helper("biztest_contact", ["run-1", "", "run-2"]),
                rows,
            )
        query = mongo_json.call_args.args[0]
        self.assertIn("memory_candidates.find", query)
        self.assertIn("contact_wxid", query)
        self.assertIn("$in", query)
        self.assertIn("run-1", query)
        self.assertIn("run-2", query)

    def test_memory_candidate_texts_only_extract_semantic_fields(self) -> None:
        rows = [{
            "candidates": [{
                "content": "孩子10岁",
                "evidence": "客户认真更正：不是8岁",
                "importance": 10,
                "confidence": 9,
            }],
        }]
        self.assertEqual(
            lib.memory_candidate_texts(rows),
            ["孩子10岁", "客户认真更正：不是8岁"],
        )

    def test_active_memory_task_binds_contact_account_and_active_states(self) -> None:
        helper = getattr(lib, "active_memory_consolidation_task", None)
        self.assertIsNotNone(helper)
        if helper is None:
            return
        row = {
            "_id": {"$oid": "64a1f2c3e4b5a697889a0001"},
            "status": "running",
            "kind": "memory_consolidation",
        }
        with mock.patch.object(lib, "mongo_json", return_value=[row]) as mongo_json:
            self.assertEqual(helper("biztest_contact", "account-2"), row)
        query = mongo_json.call_args.args[0]
        self.assertIn("memory_consolidation", query)
        self.assertIn("biztest_contact", query)
        self.assertIn("account-2", query)
        for state in ("pending", "running", "committing", "retry"):
            self.assertIn(state, query)
        self.assertNotIn("retry']}}},", query)

    def test_memory_task_terminal_waits_for_auto_consolidation(self) -> None:
        helper = getattr(lib, "wait_memory_task_terminal", None)
        self.assertIsNotNone(helper)
        if helper is None:
            return
        pending = {"status": "running"}
        sent = {"status": "sent", "gateway_status": "consolidated"}
        with mock.patch.object(
            lib, "memory_task_evidence", side_effect=[pending, sent],
        ) as evidence, mock.patch.object(lib.time, "sleep") as sleep:
            self.assertEqual(
                helper("64a1f2c3e4b5a697889a0001", max_wait=30, poll=1),
                sent,
            )
        self.assertEqual(evidence.call_count, 2)
        sleep.assert_called_once_with(1)

    def test_memory_task_evidence_normalizes_mongosh_long_generation(self) -> None:
        row = {
            "status": "sent",
            "gateway_status": "consolidated",
            "claim_generation": {"low": 1, "high": 0, "unsigned": False},
        }
        with mock.patch.object(lib, "mongo_json", return_value=[row]):
            task = lib.memory_task_evidence("64a1f2c3e4b5a697889a0001")
        self.assertEqual(task["claim_generation"], 1)

    def test_memory_commit_events_bind_normalized_task_generation(self) -> None:
        with mock.patch.object(lib, "mongo_json", return_value=[]) as mongo_json:
            self.assertEqual(
                lib.memory_commit_events("64a1f2c3e4b5a697889a0001", 2),
                [],
            )
        query = mongo_json.call_args.args[0]
        self.assertIn(
            "^memory_commit:64a1f2c3e4b5a697889a0001:2:",
            query,
        )

    def test_memory_discarded_texts_only_reads_completion_audit(self) -> None:
        events = [
            {
                "kind": "memory_conflict_resolved",
                "details": {"discarded": ["不应读取"]},
            },
            {
                "kind": "memory_consolidated",
                "details": {
                    "discarded": [
                        "客户的孩子今年8岁",
                        {"invalid": "not a text item"},
                    ],
                },
            },
        ]
        self.assertEqual(
            lib.memory_discarded_texts(events),
            ["客户的孩子今年8岁"],
        )

    def test_projection_contract_failure_is_distinct_from_transient_worker_failure(self) -> None:
        helper = getattr(lib, "projection_model_contract_failure", None)
        self.assertIsNotNone(helper)
        if helper is None:
            return
        self.assertTrue(helper({
            "post_decision_status": "failed_terminal",
            "post_decision_error_kind": "invalid_projection",
        }))
        self.assertFalse(helper({
            "post_decision_status": "failed_terminal",
            "post_decision_error_kind": "invalid_snapshot",
        }))
        self.assertFalse(helper({
            "post_decision_status": "retry",
            "post_decision_error_kind": "provider_error",
        }))

    def test_ptier_full_context_accepts_forced_or_full_escalation(self) -> None:
        helper = getattr(lib, "ptier_loaded_full_context", None)
        self.assertIsNotNone(helper)
        if helper is None:
            return
        self.assertTrue(helper([{"kind": "ptier_forced_full", "details": {}}]))
        self.assertTrue(helper([
            {"kind": "ptier_escalated", "details": {"target_tier": "Full"}},
        ]))
        self.assertFalse(helper([
            {"kind": "ptier_escalated", "details": {"target_tier": "Relational"}},
            {"kind": "ptier_run_tier", "details": {"forced_full": False}},
        ]))

    def test_ptier_clarification_requires_an_explicit_follow_up(self) -> None:
        helper = getattr(lib, "ptier_requested_clarification", None)
        self.assertIsNotNone(helper)
        if helper is None:
            return
        self.assertTrue(helper([
            {"kind": "ptier_clarify", "details": {"run_id": "run-1"}},
        ]))
        self.assertFalse(helper([
            {"kind": "ptier_escalated", "details": {"target_tier": "Full"}},
            {"kind": "ptier_run_tier", "details": {"sufficiency": "enough"}},
        ]))

    def test_exact_run_llm_assertion_waits_for_gateway_audit_flush(self) -> None:
        row = {
            "run_id": "run-1",
            "prompt_key": "user.reply.fast.task",
            "status": "success",
        }
        with mock.patch.object(lib, "mongo_json", side_effect=[[], [row]]) as mongo_json, \
             mock.patch.object(lib.time, "sleep") as sleep, \
             mock.patch.object(lib, "record"):
            self.assertTrue(
                lib.assert_llm_success_for_run(
                    "run-1", "user.reply.fast.task", "domain",
                )
            )
        self.assertEqual(mongo_json.call_count, 2)
        sleep.assert_called_once()


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

    def test_card_outbox_terminal_waits_for_same_run_and_card(self) -> None:
        pending = [
            {"referral_card_id": "card-1", "status": "pending"},
            {"status": "sent", "content": "text"},
        ]
        sent = [
            {"referral_card_id": "card-1", "status": "sent"},
            {"status": "sent", "content": "text"},
        ]
        with mock.patch.object(
            lib, "outbox_for_run", side_effect=[pending, sent],
        ) as outbox, mock.patch.object(lib.time, "sleep") as sleep:
            self.assertEqual(
                lib.wait_card_outbox_terminal(
                    "biztest_contact", "run-1", "card-1", max_wait=30, poll=1,
                ),
                sent[0],
            )
        self.assertEqual(outbox.call_count, 2)
        sleep.assert_called_once_with(1)

    def test_run_scoped_evidence_rejects_non_test_contact(self) -> None:
        for lookup in (lib.decision_review_for_run, lib.outbox_for_run, lib.ptier_events_for_run):
            with self.assertRaisesRegex(ValueError, "biztest_"):
                lookup("real-contact", "run-1")


if __name__ == "__main__":
    unittest.main()
