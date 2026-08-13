"""Remove data rooted in ``biztest_`` after restoring global runtime pointers."""
import base64
import json
import os
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
import _lib

RESTORE_MARKER_ID = "biztest_industry_profile_restore"


def _ensure_admin_cookie() -> None:
    """Create a short-lived admin cookie so crash recovery can use production APIs."""
    credentials = {
        "username": os.environ.get("ADMIN_USER", "admin"),
        "password": os.environ.get("ADMIN_PASS", "admin"),
    }
    encoded = base64.b64encode(json.dumps(credentials).encode()).decode("ascii")
    command = (
        f"echo {encoded} | base64 -d > /tmp/biztest_cleanup_login.json && "
        "curl -s -c /tmp/biztest_cookie -o /tmp/biztest_cleanup_login.out "
        f"-w '%{{http_code}}' -X POST {_lib.APP_BASE_URL}/api/auth/login "
        "-H 'Content-Type: application/json' --data-binary @/tmp/biztest_cleanup_login.json"
    )
    code, output = _lib.remote_run(command)
    if code != 0 or output.strip().splitlines()[-1:] != ["200"]:
        raise RuntimeError(f"cleanup admin login failed: exit={code} http={output[-80:]}")


def restore_interrupted_industry_profile() -> None:
    """Recover an interrupted global profile switch through rollout/activate APIs."""
    marker = _lib.mongo_json(
        f'db.biztest_control.findOne({{_id:{json.dumps(RESTORE_MARKER_ID)}}})'
    )
    if marker is None:
        active_test = _lib.mongo_json(
            'db.domain_profiles.countDocuments({profile_id:/^biztest_/,is_active:true})'
        )
        if active_test:
            raise RuntimeError("active biztest profile exists without rollback marker")
        return
    if not isinstance(marker, dict):
        raise RuntimeError(f"unreadable industry rollback marker: {marker}")
    original_id = marker.get("original_active_id")
    if original_id is None:
        _lib.restore_default_domain_profile_fallback()
        _lib.mongo(f'db.biztest_control.deleteOne({{_id:{json.dumps(RESTORE_MARKER_ID)}}})')
        return
    if not isinstance(original_id, str) or len(original_id) != 24:
        raise RuntimeError(f"invalid industry rollback marker: {marker}")
    row = _lib.mongo_json(
        f'db.domain_profiles.findOne({{_id:ObjectId({json.dumps(original_id)})}},'
        '{release_status:1,current_version:1,is_active:1,_id:1})'
    )
    if not isinstance(row, dict) or row.get("release_status") != "published":
        raise RuntimeError(f"rollback target is missing or unpublished: {row}")
    if row.get("current_version") is not True:
        response = _lib.api(
            "POST", f"/api/admin/domain-profiles/{original_id}/rollout", {}, admin=True
        )
        if _lib.is_api_error(response):
            raise RuntimeError(f"rollback target rollout failed: {response}")
    response = _lib.api(
        "POST", f"/api/admin/domain-profiles/{original_id}/activate", {}, admin=True, timeout=180
    )
    if _lib.is_api_error(response):
        raise RuntimeError(f"rollback target activation failed: {response}")
    active = _lib.mongo_json('db.domain_profiles.findOne({is_active:true},{_id:1})')
    if _lib.bson_object_id(active.get("_id") if isinstance(active, dict) else None) != original_id:
        raise RuntimeError(f"runtime pointer did not recover to {original_id}: {active}")
    _lib.mongo(f'db.biztest_control.deleteOne({{_id:{json.dumps(RESTORE_MARKER_ID)}}})')


def build_cleanup_script() -> str:
    direct_contact_collections = [
        "contacts", "conversation_messages", "agent_run_logs",
        "agent_decision_reviews", "agent_send_outbox", "agent_events",
        "agent_principal_escalations", "operating_memories", "memory_candidates",
        "agent_tasks", "behavior_signals", "knowledge_usage_logs", "llm_call_logs",
    ]
    parts = [
        'if(db.domain_profiles.countDocuments({profile_id:/^biztest_/,is_active:true})>0)'
        '{throw new Error("refusing to delete an active biztest profile")}',
        'var _contacts=db.contacts.find({wxid:/^biztest_/},{_id:1,wxid:1}).toArray()',
        'var _contact_ids=_contacts.map(x=>x._id.toString())',
        'var _runs=db.agent_run_logs.find({contact_wxid:/^biztest_/},{run_id:1}).toArray()',
        'var _run_ids=_runs.map(x=>x.run_id).filter(Boolean)',
        'var _relationship=db.relationship_type_suggestions.find({contact_id:{$in:_contact_ids}},{_id:1}).toArray()',
        'var _relationship_ids=_relationship.map(x=>x._id.toString())',
        'var _deals=db.suspected_deal_signals.find({contact_id:{$in:_contact_ids}},{_id:1}).toArray()',
        'var _deal_ids=_deals.map(x=>x._id.toString())',
        'var _projection_entity_ids=_relationship_ids.concat(_deal_ids)',
        'var _escalations=db.agent_principal_escalations.find({contact_wxid:/^biztest_/},{_id:1}).toArray()',
        'var _principal_runs=_escalations.map(x=>new RegExp("^principal-card:"+x._id.toString()+":"))',
        'var _principal_outbox=db.agent_send_outbox.find({run_id:{$in:_principal_runs}},{_id:1}).toArray()',
        'var _principal_outbox_ids=_principal_outbox.map(x=>x._id)',
        'var _documents=db.operation_knowledge_documents.find({source_name:/^biztest_/},{_id:1}).toArray()',
        'var _docids=_documents.map(x=>x._id)',
        'var _chunks=db.operation_knowledge_chunks.find({$or:[{document_id:{$in:_docids}},{source_name:/^biztest_/}]},{_id:1}).toArray()',
        'var _chunk_ids=_chunks.map(x=>x._id.toString())',
        'var _chunk_object_ids=_chunks.map(x=>x._id)',
        'var _knowledge_usage=db.knowledge_usage_logs.find({$or:[{knowledge_ids:{$in:_chunk_object_ids}},{\"route_result.chunkId\":{$in:_chunk_ids}},{\"route_result.targetId\":{$in:_chunk_ids}}]},{_id:1,run_id:1}).toArray()',
        'var _knowledge_usage_ids=_knowledge_usage.map(x=>x._id)',
        'var _knowledge_started=db.agent_events.find({kind:"knowledge_run_started","details.chunkIds":{$in:_chunk_ids}},{_id:1,"details.runId":1}).toArray()',
        'var _knowledge_started_ids=_knowledge_started.map(x=>x._id)',
        'var _knowledge_run_ids=Array.from(new Set(_knowledge_usage.map(x=>x.run_id).concat(_knowledge_started.map(x=>x.details&&x.details.runId)).filter(Boolean)))',
        'var _management_sessions=db.management_agent_sessions.find({title:/^biztest_/},{_id:1}).toArray()',
        'var _management_session_ids=_management_sessions.map(x=>x._id)',
        'var _management_llm_run_ids=_management_session_ids.map(x=>x.toString())',
        'var _management_runs=db.agent_command_runs.find({session_id:{$in:_management_session_ids}},{_id:1}).toArray()',
        'var _management_run_ids=_management_runs.map(x=>x._id)',
        'var r={}',
        'r.principal_events=db.agent_events.deleteMany({"details.outbox_id":{$in:_principal_outbox_ids}}).deletedCount',
        'r.principal_outbox=db.agent_send_outbox.deleteMany({_id:{$in:_principal_outbox_ids}}).deletedCount',
        'r.projection_observations=db.projection_observations.deleteMany({$or:[{run_id:{$in:_run_ids}},{entity_id:{$in:_projection_entity_ids}}]}).deletedCount',
        'r.relationship_suggestions=db.relationship_type_suggestions.deleteMany({_id:{$in:_relationship.map(x=>x._id)}}).deletedCount',
        'r.suspected_deals=db.suspected_deal_signals.deleteMany({_id:{$in:_deals.map(x=>x._id)}}).deletedCount',
    ]
    for collection in direct_contact_collections:
        parts.append(
            f'r.{collection}=db.{collection}.deleteMany({{$or:['
            '{contact_wxid:/^biztest_/},{from_wxid:/^biztest_/},{wxid:/^biztest_/}]}).deletedCount'
        )
    parts.extend([
        'r.taxonomy_candidates=db.taxonomy_candidates.deleteMany({source_run_ids:{$in:_run_ids}}).deletedCount',
        'r.chunk_revisions=db.chunk_revisions.deleteMany({chunk_id:{$in:_chunk_ids}}).deletedCount',
        'r.knowledge_gaps=db.knowledge_gap_signals.deleteMany({affected_chunk_ids:{$in:_chunk_ids}}).deletedCount',
        'r.catalog_jobs=db.catalog_rebuild_jobs.deleteMany({document_id:{$in:_docids}}).deletedCount',
        'r.mcp_logs=db.mcp_call_logs.deleteMany({"request.recipient":/^biztest_/}).deletedCount',
        'r.import_jobs=db.import_jobs.deleteMany({source_name:/^biztest_/}).deletedCount',
        'r.knowledge_events=db.agent_events.deleteMany({$or:[{_id:{$in:_knowledge_started_ids}},{\"details.runId\":{$in:_knowledge_run_ids}},{\"details.chunkId\":{$in:_chunk_ids}},{\"details.chunkIds\":{$in:_chunk_ids}}]}).deletedCount',
        'r.knowledge_llm_logs=db.llm_call_logs.deleteMany({run_id:{$in:_knowledge_run_ids}}).deletedCount',
        'r.knowledge_usage=db.knowledge_usage_logs.deleteMany({_id:{$in:_knowledge_usage_ids}}).deletedCount',
        'r.management_llm_logs=db.llm_call_logs.deleteMany({run_id:{$in:_management_llm_run_ids}}).deletedCount',
        'r.management_tool_calls=db.agent_tool_calls.deleteMany({command_run_id:{$in:_management_run_ids}}).deletedCount',
        'r.management_runs=db.agent_command_runs.deleteMany({_id:{$in:_management_run_ids}}).deletedCount',
        'r.management_messages=db.management_agent_messages.deleteMany({session_id:{$in:_management_session_ids}}).deletedCount',
        'r.management_sessions=db.management_agent_sessions.deleteMany({_id:{$in:_management_session_ids}}).deletedCount',
        'r.kchunks=db.operation_knowledge_chunks.deleteMany({_id:{$in:_chunks.map(x=>x._id)}}).deletedCount',
        'r.kdocs=db.operation_knowledge_documents.deleteMany({_id:{$in:_docids}}).deletedCount',
        'r.assets=db.content_assets.deleteMany({title:/^biztest_/}).deletedCount',
        'r.cards=db.referral_cards.deleteMany({display_name:/^biztest_/}).deletedCount',
        'r.profiles=db.domain_profiles.deleteMany({profile_id:/^biztest_/,is_active:false}).deletedCount',
        'r.campaigns=db.campaigns.deleteMany({title:/^biztest_/}).deletedCount',
        'r.campaign_sends=db.campaign_sends.deleteMany({contactWxid:/^biztest_/}).deletedCount',
        'r.guide_previews=db.user_operation_guide_previews.deleteMany({contact_wxid:/^biztest_/}).deletedCount',
        'r.control=db.biztest_control.deleteMany({_id:/^biztest_/}).deletedCount',
        'printjson(r)',
    ])
    return "; ".join(parts)


def main() -> None:
    _ensure_admin_cookie()
    restore_interrupted_industry_profile()
    print(_lib.mongo(build_cleanup_script()))
    _lib.api("POST", "/api/auth/logout", {}, admin=True, timeout=30)


if __name__ == "__main__":
    main()
