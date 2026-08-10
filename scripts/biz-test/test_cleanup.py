import importlib.util
import unittest
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

    def test_all_name_filters_require_the_biztest_prefix(self) -> None:
        self.assertNotIn("source_name:/biztest/", self.script)
        self.assertNotIn("title:/biztest/", self.script)
        self.assertTrue(self.script.endswith("printjson(r)"))


if __name__ == "__main__":
    unittest.main()
