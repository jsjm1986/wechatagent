"""Remove all data rooted in the ``biztest_`` namespace, idempotently.

The script snapshots test entity ids before deleting roots, then removes only rows linked to
those ids. It never uses a time window, so concurrent production traffic is not selected.
"""
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
import _lib


def build_cleanup_script() -> str:
    direct_contact_collections = [
        "contacts", "conversation_messages", "agent_run_logs",
        "agent_decision_reviews", "agent_send_outbox", "agent_events",
        "agent_principal_escalations", "operating_memories", "memory_candidates",
        "agent_tasks", "behavior_signals", "knowledge_usage_logs", "llm_call_logs",
    ]
    parts = [
        'var _contacts=db.contacts.find({wxid:/^biztest_/},{_id:1,wxid:1}).toArray()',
        'var _contact_ids=_contacts.map(x=>x._id.toString())',
        'var _runs=db.agent_run_logs.find({contact_wxid:/^biztest_/},{run_id:1}).toArray()',
        'var _run_ids=_runs.map(x=>x.run_id).filter(Boolean)',
        'var _escalations=db.agent_principal_escalations.find({contact_wxid:/^biztest_/},{_id:1}).toArray()',
        'var _principal_runs=_escalations.map(x=>new RegExp("^principal-card:"+x._id.toString()+":"))',
        'var _principal_outbox=db.agent_send_outbox.find({run_id:{$in:_principal_runs}},{_id:1}).toArray()',
        'var _principal_outbox_ids=_principal_outbox.map(x=>x._id)',
        'var _documents=db.operation_knowledge_documents.find({source_name:/^biztest_/},{_id:1}).toArray()',
        'var _docids=_documents.map(x=>x._id)',
        'var _chunks=db.operation_knowledge_chunks.find({$or:[{document_id:{$in:_docids}},{source_name:/^biztest_/}]},{_id:1}).toArray()',
        'var _chunk_ids=_chunks.map(x=>x._id.toString())',
        'var r={}',
        'r.principal_events=db.agent_events.deleteMany({\"details.outbox_id\":{$in:_principal_outbox_ids}}).deletedCount',
        'r.principal_outbox=db.agent_send_outbox.deleteMany({_id:{$in:_principal_outbox_ids}}).deletedCount',
    ]
    for collection in direct_contact_collections:
        parts.append(
            f'r.{collection}=db.{collection}.deleteMany({{$or:['
            '{contact_wxid:/^biztest_/},{from_wxid:/^biztest_/},{wxid:/^biztest_/}]}}).deletedCount'
        )
    parts.extend([
        'r.relationship_suggestions=db.relationship_type_suggestions.deleteMany({contact_id:{$in:_contact_ids}}).deletedCount',
        'r.suspected_deals=db.suspected_deal_signals.deleteMany({contact_id:{$in:_contact_ids}}).deletedCount',
        'r.projection_observations=db.projection_observations.deleteMany({$or:[{run_id:{$in:_run_ids}},{entity_id:{$in:_contact_ids}}]}).deletedCount',
        'r.taxonomy_candidates=db.taxonomy_candidates.deleteMany({source_run_ids:{$in:_run_ids}}).deletedCount',
        'r.chunk_revisions=db.chunk_revisions.deleteMany({chunk_id:{$in:_chunk_ids}}).deletedCount',
        'r.knowledge_gaps=db.knowledge_gap_signals.deleteMany({affected_chunk_ids:{$in:_chunk_ids}}).deletedCount',
        'r.catalog_jobs=db.catalog_rebuild_jobs.deleteMany({document_id:{$in:_docids}}).deletedCount',
        'r.mcp_logs=db.mcp_call_logs.deleteMany({"request.recipient":/^biztest_/}).deletedCount',
        'r.import_jobs=db.import_jobs.deleteMany({source_name:/^biztest_/}).deletedCount',
        'r.kchunks=db.operation_knowledge_chunks.deleteMany({_id:{$in:_chunks.map(x=>x._id)}}).deletedCount',
        'r.kdocs=db.operation_knowledge_documents.deleteMany({_id:{$in:_docids}}).deletedCount',
        'r.assets=db.content_assets.deleteMany({title:/^biztest_/}).deletedCount',
        'r.cards=db.referral_cards.deleteMany({display_name:/^biztest_/}).deletedCount',
        'r.profiles=db.domain_profiles.deleteMany({profile_id:/^biztest_/}).deletedCount',
        'r.campaigns=db.campaigns.deleteMany({title:/^biztest_/}).deletedCount',
        'r.campaign_sends=db.campaign_sends.deleteMany({contactWxid:/^biztest_/}).deletedCount',
        'r.guide_previews=db.user_operation_guide_previews.deleteMany({contact_wxid:/^biztest_/}).deletedCount',
        'printjson(r)',
    ])
    return "; ".join(parts)


def main() -> None:
    # Logout removes the test-created admin session identified by the cookie; failures are harmless
    # for preflight runs that never reached login.
    _lib.api("POST", "/api/auth/logout", {}, admin=True, timeout=30)
    print(_lib.mongo(build_cleanup_script()))


if __name__ == "__main__":
    main()
