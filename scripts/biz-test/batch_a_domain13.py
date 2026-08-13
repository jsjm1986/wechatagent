"""域⑬：知识自治（auto-verify 红线、completeness clamp、repair 提案、Vision 能力）。

每个核心断言创建自己的当前契约 fixture，并以精确 chunk id / revision / usage ledger 取证；
不依赖前序域残留。Vision 是可选环境能力，未配置或未提供图片 fixture 时写独立 BLOCKED
台账，不计作核心业务 PASS。
"""
import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
import _lib

DOMAIN = "⑬知识自治"


def _require(condition: bool, description: str, evidence: object, severity: str = "critical") -> None:
    _lib.expect(condition, DOMAIN, description, str(evidence), severity)


def main() -> None:
    account_id, _ = _lib.biztest_account()

    # 独立、最新的 citable draft；limit=1 令本轮只能处理这一行。
    auto_id = _lib.seed_citable_knowledge_chunk(
        "biztest_auto_verify_redline", account_id, "biztest 自动预审价格",
        "企业版年费 9800 元，含 10 个坐席。",
    )
    _require(bool(auto_id), "创建 auto-verify 独立 fixture", auto_id)
    av = _lib.api_bg(
        "POST", "/api/operation-knowledge/auto-verify",
        {"accountId": account_id, "confidenceThreshold": 7,
         "humanAuditSampleRate": 0.0, "limit": 1},
        admin=True, max_wait=600, tag="autoverify_exact",
    )
    auto_run_id = av.get("runId") if isinstance(av, dict) else None
    _require(_lib.is_api_error(av) is None and av.get("processed") == 1
             and av.get("failed") == 0
             and isinstance(auto_run_id, str) and bool(auto_run_id)
             and av.get("chunkIds") == [auto_id],
             "auto-verify 精确处理一条且返回冻结 run/chunk 身份", av)
    auto_item = _lib.get_knowledge_chunk(auto_id)
    usage = _lib.mongo_json(
        'db.knowledge_usage_logs.findOne('
        f'{{"route_result.kind":"knowledge_auto_verify",'
        f'"route_result.chunkId":{json.dumps(auto_id)}}},'
        '{route_result:1,knowledge_ids:1,run_id:1,_id:0},{sort:{created_at:-1}})'
    )
    revisions = _lib.mongo_json(
        'db.chunk_revisions.find('
        f'{{chunk_id:{json.dumps(auto_id)},op:"verify",source:"rule",'
        'reason:/^auto_verify:/},'
        '{revision_id:1,source:1,op:1,patch:1,_id:0})'
        '.sort({created_at:-1}).limit(1).toArray()'
    )
    revision = revisions[0] if isinstance(revisions, list) and revisions else None
    route = usage.get("route_result", {}) if isinstance(usage, dict) else {}
    final_status = route.get("finalStatus") if isinstance(route, dict) else None
    exact_audit = (
        isinstance(revision, dict) and bool(revision.get("revision_id"))
        and isinstance(route, dict)
        and route.get("chunkId") == auto_id
        and route.get("revisionId") == revision.get("revision_id")
        and usage.get("run_id") == auto_run_id
        and final_status == auto_item.get("integrityStatus")
    )
    _require(exact_audit, "auto-verify usage 与 rule revision 精确绑定目标 chunk",
             {"item": auto_item, "usage": usage, "revision": revision})
    _require(final_status != "verified" and auto_item.get("integrityStatus") != "verified",
             "AI 自动预审绝不把知识放行为 verified",
             {"response": av, "item": auto_item, "usage": usage})
    _lib.assert_llm_success_for_run(auto_run_id, "knowledge.auto_verify", DOMAIN)

    # Repair 使用另一条 draft，避免被上面的 auto-verify 改写；proposal 不得改 chunk。
    repair_id = _lib.seed_citable_knowledge_chunk(
        "biztest_repair_fixture", account_id, "biztest 待补全交付说明",
        "交付支持远程实施，具体周期尚待运营补充。",
    )
    _require(bool(repair_id), "创建 repair 独立 fixture", repair_id)
    before = _lib.get_knowledge_chunk(repair_id)

    # 有 needs_review fixture 时 completeness 必须看到草稿且不得宣称 fully supported。
    comp = _lib.api_bg(
        "POST", "/api/operation-knowledge/completeness?accountId=" + account_id,
        None, admin=True, max_wait=400, tag="completeness",
    )
    comp_item = comp.get("item") if isinstance(comp, dict) else None
    comp_item = comp_item if isinstance(comp_item, dict) else comp
    _require(_lib.is_api_error(comp) is None
             and isinstance(comp_item, dict)
             and isinstance(comp_item.get("needsReviewChunks"), int)
             and comp_item.get("needsReviewChunks") > 0,
             "completeness 识别 needs_review fixture", comp, "high")
    _require(comp_item.get("answeringMode") != "fully_supported",
             "有 needs_review 草稿时 answeringMode 不得 fully_supported", comp, "high")

    rp = _lib.api_bg(
        "POST", f"/api/operation-knowledge/chunks/{repair_id}/repair",
        None, admin=True, max_wait=600, tag="repair_exact",
    )
    after = _lib.get_knowledge_chunk(repair_id)
    proposal_ok = (
        _lib.is_api_error(rp) is None
        and rp.get("chunkId") == repair_id
        and isinstance(rp.get("sessionId"), str) and bool(rp.get("sessionId"))
        and isinstance(rp.get("runId"), str)
        and rp.get("runId", "").startswith(f"repair-chunk-{repair_id}-")
        and rp.get("promptKey") == "knowledge.chunk.repair.propose"
        and isinstance(rp.get("patch"), dict)
        and isinstance(rp.get("missingFields"), list)
    )
    _require(proposal_ok, "repair 返回绑定目标 chunk/run 的结构化提案", rp, "high")
    repair_usage = _lib.mongo_json(
        'db.knowledge_usage_logs.findOne('
        f'{{run_id:{json.dumps(rp.get("runId"))},"route_result.targetId":{json.dumps(repair_id)}}},'
        '{run_id:1,route_result:1,knowledge_ids:1,_id:0})'
    )
    _require(isinstance(repair_usage, dict) and repair_usage.get("run_id") == rp.get("runId"),
             "repair usage ledger 精确绑定目标 chunk/run", repair_usage, "high")
    immutable_fields = ("status", "integrityStatus", "body", "summary", "updatedAt")
    unchanged = all(before.get(key) == after.get(key) for key in immutable_fields)
    _require(unchanged, "repair propose 只生成提案、不修改 chunk", {"before": before, "after": after})
    _lib.assert_llm_success_for_run(
        rp.get("runId", ""), "knowledge.chunk.repair.propose", DOMAIN
    )

    # Vision 是独立可选能力。本矩阵没有上传真实图片，必须显式 BLOCKED 而不是假装通过。
    providers = _lib.api("GET", "/api/admin/llm-providers", admin=True)
    items = providers.get("items", providers) if isinstance(providers, dict) else providers
    items = items if isinstance(items, list) else []
    active_vision = next((item for item in items if item.get("isVisionActive")), None)
    if active_vision is None:
        _lib.record_blocked(DOMAIN, "vision_import", "no active vision provider",
                            "configure one active supportsVision provider and rerun image import")
    else:
        _lib.record_blocked(DOMAIN, "vision_import", f"provider={active_vision.get('id')}",
                            "supply an isolated biztest image fixture and assert import lineage")

    print(f"[{DOMAIN}] 核心完成：auto-verify/complete/repair 已精确取证；Vision 见 BLOCKED 台账")


if __name__ == "__main__":
    main()
