"""域⑬：知识库自治 LLM 群（auto_verify 红线 / completeness clamp / repair 忠实 / vision）。

实测确认的真实形态：
- auto-verify：POST /operation-knowledge/auto-verify，body camelCase
  {accountId,confidenceThreshold,humanAuditSampleRate,limit}；响应
  {processed,verified,needsReview,rejected,needsHumanAudit}。红线(verify.rs:392):
  product_fact 类**一律强制 needsHumanAudit**,不被 LLM 自评放行成 verified。
  prompt_key=knowledge.auto_verify(走 generate_agent_json,写 log)。
- completeness：GET/POST /operation-knowledge/completeness?accountId=。**直调 state.llm.generate_json
  (catalog.rs:711),绕过 generate_agent_json→不写 llm_call_logs,故不能 assert_llm_success**。
  verified==0 时走 fallback 早退不调 LLM。clamp 红线:有 needs_review 草稿绝不宣称 fully_supported。
  响应 {totalChunks,verifiedChunks,needsReviewChunks,answeringMode,gaps}。
- repair：POST /operation-knowledge/chunks/:id/repair(无 body)；响应
  {patch,missingFields,followupQuestions,sessionId}；prompt_key=knowledge.chunk.repair.propose。
- vision：需 active vision provider,无则标 BLOCKED 不假绿。

跑法：export DEPLOY_PASS=...; python scripts/biz-test/batch_a_domain13.py
"""
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
import _lib

DOMAIN = "⑬知识自治"


def main() -> None:
    account_id, _app_id = _lib.biztest_account()

    # ── auto-verify：批量校验（复用域①/域②落的 biztest needs_review chunks）──
    print(f"[{DOMAIN}] auto-verify 批量校验（真模型，后台轮询）...")
    av = _lib.api_bg(
        "POST", "/api/operation-knowledge/auto-verify",
        {"accountId": account_id, "confidenceThreshold": 7,
         "humanAuditSampleRate": 0.1, "limit": 20},
        admin=True, max_wait=600, tag="autoverify",
    )
    print(f"  av={str(av)[:300]}")
    processed = av.get("processed") if isinstance(av, dict) else None
    _lib.expect(processed is not None, DOMAIN, "auto-verify 真跑返回统计(processed)",
                f"av={str(av)[:300]}", "high", "auto-verify 无 processed→端点错或无 chunk 可校")
    if processed is not None and processed > 0:
        _lib.assert_llm_success(600, "knowledge.auto_verify", DOMAIN)

    # 红线：种一条 product_fact 类切片，验 auto-verify 不把它放行成 verified（强制 needsHumanAudit）
    _lib.mongo('db.operation_knowledge_chunks.deleteMany({source_name:"biztest_product_fact"})')
    _lib.mongo(
        'db.operation_knowledge_chunks.insertOne({'
        'source_name:"biztest_product_fact",title:"biztest 价格",'
        'workspace_id:"default",domain:"user_operations",priority:0,'
        'content:"企业版年费 9800 元，含 10 个坐席。",'
        'source_quote:"企业版年费 9800 元，含 10 个坐席。",'
        'source_anchors:["企业版年费 9800 元"],'
        'knowledge_type:"product_fact",integrity_status:"needs_review",status:"draft",'
        f'account_id:"{account_id}",created_at:new Date(),updated_at:new Date()'
        '}})'
    )
    av2 = _lib.api_bg(
        "POST", "/api/operation-knowledge/auto-verify",
        {"accountId": account_id, "confidenceThreshold": 7,
         "humanAuditSampleRate": 0.0, "limit": 20},
        admin=True, max_wait=600, tag="autoverify2",
    )
    pf = _lib.mongo_json(
        'db.operation_knowledge_chunks.findOne({source_name:"biztest_product_fact"},'
        '{integrity_status:1,_id:0})'
    )
    pf_status = pf.get("integrity_status") if isinstance(pf, dict) else None
    _lib.expect(pf_status != "verified", DOMAIN,
                "红线:product_fact 类 auto-verify 不自评放行成 verified(强制人工把关)",
                f"product_fact integrity_status={pf_status} av2={str(av2)[:200]}", "critical",
                "product_fact 被 LLM 自评直接 verified=AI永不自动verify红线破")

    # ── completeness：clamp（有 needs_review 草稿不宣称 fully_supported）──
    # 注意：completeness 直调 llm.generate_json 不写 log，不能 assert_llm_success。
    print(f"[{DOMAIN}] completeness 审计...")
    comp = _lib.api_bg(
        "POST", "/api/operation-knowledge/completeness?accountId=" + account_id,
        None, admin=True, max_wait=400, tag="completeness",
    )
    mode = str(comp.get("answeringMode", "")) if isinstance(comp, dict) else ""
    nr = comp.get("needsReviewChunks", 0) if isinstance(comp, dict) else 0
    print(f"  answeringMode={mode} needsReviewChunks={nr}")
    if isinstance(nr, int) and nr > 0:
        _lib.expect(mode != "fully_supported", DOMAIN,
                    "completeness clamp:有 needs_review 草稿绝不宣称 fully_supported",
                    f"mode={mode} needsReview={nr}", "high",
                    "有草稿仍判 fully_supported=认知状态闸 clamp 失效")
    else:
        _lib.expect(isinstance(comp, dict) and "answeringMode" in comp, DOMAIN,
                    "completeness 返回 answeringMode(无草稿则 clamp 不触发,仅验链路通)",
                    f"comp={str(comp)[:200]}", "low")

    # ── repair：对一条 needs_review chunk 跑修复，验产 patch ──
    print(f"[{DOMAIN}] chunk repair...")
    ch = _lib.mongo_json(
        'db.operation_knowledge_chunks.findOne('
        '{source_name:/biztest/,integrity_status:"needs_review"},{_id:1})'
    )
    if isinstance(ch, dict) and ch.get("_id"):
        oid = ch["_id"]
        chid = str(oid.get("$oid", "")) if isinstance(oid, dict) else str(oid)
        rp = _lib.api_bg(
            "POST", f"/api/operation-knowledge/chunks/{chid}/repair",
            None, admin=True, max_wait=600, tag="repair",
        )
        has_patch = isinstance(rp, dict) and ("patch" in rp or "missingFields" in rp)
        _lib.expect(has_patch, DOMAIN, "repair 真产 patch/missingFields",
                    f"rp={str(rp)[:300]}", "medium", "repair 无 patch→修复链路未产出")
        if has_patch:
            _lib.assert_llm_success(600, "knowledge.chunk.repair.propose", DOMAIN)
    else:
        _lib.record(DOMAIN, "无 biztest needs_review chunk 可跑 repair(先跑域①②)",
                    "findOne 返回空", "low", "依赖前序域落库,非bug")

    # ── vision：需 active vision provider，否则 BLOCKED 不假绿 ──
    provs = _lib.api("GET", "/api/admin/llm-providers", admin=True)
    items = provs.get("items", provs) if isinstance(provs, dict) else provs
    items = items if isinstance(items, list) else []
    has_vision = any(p.get("isVisionActive") or p.get("supportsVision")
                     or p.get("visionActive") for p in items)
    if not has_vision:
        _lib.record(DOMAIN, "vision 多模态子项 BLOCKED(无 active vision provider)",
                    "GET /llm-providers 无 vision active", "low",
                    "需配 llama-3.2-90b-vision 等后单独测,非bug(rsxermu claude 不一定开 vision)")
    else:
        print(f"  vision provider 在,vision import 子项需图片 base64,本版留人工补")

    print(f"[{DOMAIN}] 完成")


if __name__ == "__main__":
    main()
