import importlib.util
import unittest
from unittest import mock
from pathlib import Path


MODULE_PATH = Path(__file__).with_name("cleanup.py")
SPEC = importlib.util.spec_from_file_location("biztest_cleanup", MODULE_PATH)
assert SPEC is not None and SPEC.loader is not None
cleanup = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(cleanup)


class BuildCleanupScriptTests(unittest.TestCase):
    def setUp(self) -> None:
        self.script = cleanup.build_cleanup_script()

    def test_cleans_decision_reviews_for_biztest_contacts(self) -> None:
        self.assertIn("db.agent_decision_reviews.deleteMany", self.script)
        self.assertIn("{contact_wxid:/^biztest_/}", self.script)
        self.assertIn("refusing to delete an active biztest profile", self.script)

    def test_contact_root_delete_filters_are_balanced(self) -> None:
        expected = (
            "deleteMany({$or:[{contact_wxid:/^biztest_/},"
            "{from_wxid:/^biztest_/},{wxid:/^biztest_/}]})"
        )
        self.assertIn(expected, self.script)
        self.assertNotIn("{wxid:/^biztest_/}]}})", self.script)

    def test_cleans_document_and_legacy_knowledge_shapes(self) -> None:
        document_query = (
            "db.operation_knowledge_documents.find({source_name:/^biztest_/},{_id:1})"
        )
        linked_chunk_delete = "deleteMany({document_id:{$in:_docids}})"
        legacy_chunk_delete = "deleteMany({source_name:/^biztest_/})"
        document_delete = "db.operation_knowledge_documents.deleteMany({_id:{$in:_docids}})"
        frozen_chunk_delete = (
            "db.operation_knowledge_chunks.deleteMany({_id:{$in:_chunks.map(x=>x._id)}})"
        )

        self.assertIn(document_query, self.script)
        self.assertIn(linked_chunk_delete, self.script)
        self.assertIn(legacy_chunk_delete, self.script)
        self.assertIn(frozen_chunk_delete, self.script)
        self.assertIn(document_delete, self.script)
        self.assertLess(self.script.index(frozen_chunk_delete), self.script.index(document_delete))

    def test_cleans_production_observed_derivative_shapes(self) -> None:
        for collection in (
            "agent_tasks",
            "behavior_signals",
            "knowledge_usage_logs",
            "import_jobs",
            "catalog_rebuild_jobs",
            "chunk_revisions",
            "knowledge_gap_signals",
            "projection_observations",
            "relationship_type_suggestions",
            "suspected_deal_signals",
            "mcp_call_logs",
        ):
            self.assertIn(f"db.{collection}", self.script)
        self.assertIn('new RegExp("^principal-card:"', self.script)
        self.assertIn('db.agent_events.deleteMany({"details.outbox_id":{$in:_principal_outbox_ids}})', self.script)
        self.assertIn("_projection_entity_ids", self.script)
        self.assertIn("entity_id:{$in:_projection_entity_ids}", self.script)

    def test_knowledge_runs_are_frozen_from_biztest_chunks_then_deleted_exactly(self) -> None:
        self.assertIn("db.knowledge_usage_logs.find({$or:", self.script)
        self.assertIn('"route_result.chunkId":{$in:_chunk_ids}', self.script)
        self.assertIn('"route_result.targetId":{$in:_chunk_ids}', self.script)
        self.assertIn('kind:"knowledge_run_started"', self.script)
        self.assertIn('"details.chunkIds":{$in:_chunk_ids}', self.script)
        self.assertIn(
            '_knowledge_usage.map(x=>x.run_id).concat(_knowledge_started.map',
            self.script,
        )
        self.assertIn("_knowledge_run_ids", self.script)
        self.assertIn(
            'db.agent_events.deleteMany({$or:[{_id:{$in:_knowledge_started_ids}},',
            self.script,
        )
        self.assertIn(
            "db.llm_call_logs.deleteMany({run_id:{$in:_knowledge_run_ids}})",
            self.script,
        )
        usage_delete = "db.knowledge_usage_logs.deleteMany({_id:{$in:_knowledge_usage_ids}})"
        chunk_delete = "db.operation_knowledge_chunks.deleteMany({_id:{$in:_chunks.map(x=>x._id)}})"
        self.assertLess(self.script.index(usage_delete), self.script.index(chunk_delete))

    def test_management_sessions_are_frozen_then_deleted_child_first(self) -> None:
        self.assertIn(
            "db.management_agent_sessions.find({title:/^biztest_/},{_id:1})",
            self.script,
        )
        tool_calls = "db.agent_tool_calls.deleteMany({command_run_id:{$in:_management_run_ids}})"
        runs = "db.agent_command_runs.deleteMany({_id:{$in:_management_run_ids}})"
        messages = "db.management_agent_messages.deleteMany({session_id:{$in:_management_session_ids}})"
        sessions = "db.management_agent_sessions.deleteMany({_id:{$in:_management_session_ids}})"
        for fragment in (tool_calls, runs, messages, sessions):
            self.assertIn(fragment, self.script)
        self.assertLess(self.script.index(tool_calls), self.script.index(runs))
        self.assertLess(self.script.index(runs), self.script.index(sessions))
        self.assertLess(self.script.index(messages), self.script.index(sessions))
        self.assertIn("_management_llm_run_ids=_management_session_ids.map(x=>x.toString())", self.script)
        self.assertIn(
            "db.llm_call_logs.deleteMany({run_id:{$in:_management_llm_run_ids}})",
            self.script,
        )

    def test_all_name_filters_require_the_biztest_prefix(self) -> None:
        self.assertNotIn("source_name:/biztest/", self.script)
        self.assertNotIn("title:/biztest/", self.script)
        self.assertTrue(self.script.endswith("printjson(r)"))

    def test_interrupted_profile_is_restored_before_marker_deletion(self) -> None:
        marker = {"original_active_id": "64a1f2c3e4b5a697889a0001"}
        reads = [
            marker,
            {"_id": {"$oid": marker["original_active_id"]},
             "release_status": "published", "current_version": False},
            {"_id": {"$oid": marker["original_active_id"]}},
        ]
        with mock.patch.object(cleanup._lib, "mongo_json", side_effect=reads), \
             mock.patch.object(cleanup._lib, "api", return_value={"ok": True}) as api, \
             mock.patch.object(cleanup._lib, "mongo") as mongo:
            cleanup.restore_interrupted_industry_profile()
        self.assertIn("/rollout", api.call_args_list[0].args[1])
        self.assertIn("/activate", api.call_args_list[1].args[1])
        self.assertIn("biztest_control.deleteOne", mongo.call_args.args[0])

    def test_active_test_profile_without_marker_fails_closed(self) -> None:
        with mock.patch.object(cleanup._lib, "mongo_json", side_effect=[None, 1]):
            with self.assertRaisesRegex(RuntimeError, "without rollback marker"):
                cleanup.restore_interrupted_industry_profile()


if __name__ == "__main__":
    unittest.main()
