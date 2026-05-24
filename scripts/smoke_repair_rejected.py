"""真实 LLM 跑 AI 自主修复，把存量 rejected chunks 推到 verified / needs_review。

用法：
    python scripts/smoke_repair_rejected.py            # 默认仅修 1 条做 smoke
    SMOKE_REPAIR_LIMIT=51 python scripts/smoke_repair_rejected.py  # 全量

每条 chunk：
  1) GET /chunks/:id/source            → 拿 chunk + 父 doc raw_content
  2) POST /chunks/:id/repair           → AI 第 1 轮提案
  3) （若有 followup）POST /chunks/:id/repair/answer → AI 第 2 轮（操作员答 "无更多信息"）
  4) Merge patch → 当前 chunk → PUT /chunks/:id（写库；后端 integrity 仍由 apply_chunk_integrity 决定）
  5) POST /chunks/:id/verify           → 走 sourceQuote → anchor 严格 gate

不绕；不 mock。失败会 stdout 打 ✗，不 raise，让批量继续跑。
"""

from __future__ import annotations

import json
import os
import sys
import time
import urllib.error
import urllib.request
from typing import Any

if hasattr(sys.stdout, "reconfigure"):
    try:
        sys.stdout.reconfigure(encoding="utf-8")  # type: ignore[attr-defined]
        sys.stderr.reconfigure(encoding="utf-8")  # type: ignore[attr-defined]
    except Exception:
        pass

API = os.environ.get("WECHATAGENT_API", "http://127.0.0.1:8080/api")
LIMIT = int(os.environ.get("SMOKE_REPAIR_LIMIT", "1"))


def _request(method: str, path: str, body: dict[str, Any] | None = None, timeout: int = 300) -> Any:
    url = f"{API}{path}"
    data = json.dumps(body, ensure_ascii=False).encode("utf-8") if body is not None else None
    req = urllib.request.Request(url, method=method, data=data, headers={"Content-Type": "application/json"})
    try:
        with urllib.request.urlopen(req, timeout=timeout) as r:
            txt = r.read().decode("utf-8")
            return json.loads(txt) if txt else {}
    except urllib.error.HTTPError as e:
        body_txt = e.read().decode("utf-8", errors="replace")
        raise RuntimeError(f"HTTP {e.code} {method} {path}: {body_txt[:300]}")
    except (TimeoutError, OSError) as e:
        raise RuntimeError(f"NET {type(e).__name__} {method} {path}: {e}")


def banner(t: str) -> None:
    print(f"\n{'=' * 78}\n{t}\n{'=' * 78}", flush=True)


def merge_patch_into_chunk(chunk: dict[str, Any], patch: dict[str, Any]) -> dict[str, Any]:
    """把 AI patch 合并进 chunk，转成 PUT 用的 OperationKnowledgeChunkRequest。"""
    out: dict[str, Any] = {
        "accountId": chunk.get("accountId"),
        "documentId": chunk.get("documentId"),
        "itemId": chunk.get("itemId"),
        "domain": chunk.get("domain") or "user_operations",
        "knowledgeType": chunk.get("knowledgeType"),
        "businessContext": chunk.get("businessContext"),
        "title": patch.get("title") or chunk.get("title") or "",
        "summary": patch.get("summary") or chunk.get("summary"),
        "body": patch.get("body") or chunk.get("body"),
        "routingCard": patch.get("routingCard") or chunk.get("routingCard"),
        "applicableScenes": patch.get("applicableScenes") or chunk.get("applicableScenes") or [],
        "notApplicableScenes": patch.get("notApplicableScenes") or chunk.get("notApplicableScenes") or [],
        "safeClaims": patch.get("safeClaims") or chunk.get("safeClaims") or [],
        "forbiddenClaims": patch.get("forbiddenClaims") or chunk.get("forbiddenClaims") or [],
        "evidenceItems": patch.get("evidenceItems") or chunk.get("evidenceItems") or [],
        "productTags": patch.get("productTags") or chunk.get("productTags") or [],
        "triggerKeywords": patch.get("triggerKeywords") or chunk.get("triggerKeywords") or [],
        "businessTopics": patch.get("businessTopics") or chunk.get("businessTopics") or [],
        "sourceQuote": patch.get("sourceQuote") or chunk.get("sourceQuote"),
        "sourceAnchors": chunk.get("sourceAnchors") or [],   # 让后端重算
        "integrityStatus": chunk.get("integrityStatus") or "needs_review",
        "confidenceScore": chunk.get("confidenceScore"),
        "distortionRisks": chunk.get("distortionRisks") or [],
        "unsupportedClaims": chunk.get("unsupportedClaims") or [],
        "verifiedClaims": chunk.get("verifiedClaims") or [],
        "status": chunk.get("status") or "review",
        "priority": chunk.get("priority") or 0,
    }
    return {k: v for k, v in out.items() if v is not None}


def repair_one(chunk_id: str) -> dict[str, Any]:
    banner(f"[修复] chunk:{chunk_id}")
    src = _request("GET", f"/operation-knowledge/chunks/{chunk_id}/source")
    chunk = src.get("chunk") or {}
    print(f"  before: integrityStatus={chunk.get('integrityStatus')!r} sourceQuote={(chunk.get('sourceQuote') or '')[:50]!r}")

    # 第 1 轮 propose
    t0 = time.time()
    try:
        proposal = _request("POST", f"/operation-knowledge/chunks/{chunk_id}/repair", {})
    except RuntimeError as e:
        print(f"  ✗ propose: {e}")
        return {"error": "propose"}
    print(f"  propose 耗时 {time.time()-t0:.1f}s patch={list((proposal.get('patch') or {}).keys())}")
    print(f"  missingFields={proposal.get('missingFields')} followup={len(proposal.get('followupQuestions') or [])}")

    patch = proposal.get("patch") or {}
    followup = proposal.get("followupQuestions") or []

    # 第 2 轮：自动答 "暂无更多信息"，让 AI 用 patch 推到尽
    if followup:
        answers = [{"id": q.get("id") or f"q{i}", "field": q.get("field"), "text": "暂无更多信息，请基于已有原文尽力补全；找不到的字段保持原值即可。"} for i, q in enumerate(followup)]
        t0 = time.time()
        try:
            answered = _request("POST", f"/operation-knowledge/chunks/{chunk_id}/repair/answer", {
                "sessionId": proposal.get("sessionId"),
                "previousPatch": patch,
                "answers": answers,
                "turn": 2,
            })
        except RuntimeError as e:
            print(f"  ✗ answer: {e}")
            answered = None
        if answered:
            print(f"  answer 耗时 {time.time()-t0:.1f}s patch={list((answered.get('patch') or {}).keys())}")
            patch = answered.get("patch") or patch

    # 应用 patch
    new_chunk = merge_patch_into_chunk(chunk, patch)
    try:
        _request("PUT", f"/operation-knowledge/chunks/{chunk_id}", new_chunk)
        print(f"  ✓ PUT applied")
    except RuntimeError as e:
        print(f"  ✗ PUT: {e}")
        return {"error": "put"}

    # verify
    try:
        v = _request("POST", f"/operation-knowledge/chunks/{chunk_id}/verify", {})
        print(f"  ✓ verify ok")
    except RuntimeError as e:
        print(f"  ⚠ verify 拒绝（符合预期，gate 由 sourceQuote→anchor 决定）: {str(e)[:160]}")

    # 看最终状态
    after = _request("GET", f"/operation-knowledge/chunks/{chunk_id}/source")
    a = after.get("chunk") or {}
    print(f"  after: integrityStatus={a.get('integrityStatus')!r} sourceAnchors={len(a.get('sourceAnchors') or [])}")
    return {"before": chunk.get("integrityStatus"), "after": a.get("integrityStatus")}


def main() -> None:
    banner("[开始] 列出所有 rejected chunks")
    listing = _request("GET", "/operation-knowledge/chunks?limit=300")
    items = listing.get("items") or []
    rejected = [it for it in items if it.get("integrityStatus") == "rejected"]
    print(f"  total chunks={len(items)} rejected={len(rejected)}")
    targets = rejected[:LIMIT]
    print(f"  本轮处理 {len(targets)} 条（SMOKE_REPAIR_LIMIT={LIMIT}）")

    summary = []
    for i, it in enumerate(targets, 1):
        cid = it.get("id") or it.get("_id")
        if not cid:
            continue
        print(f"\n>>> [{i}/{len(targets)}] {it.get('title','')[:40]!r}  id={cid}")
        try:
            r = repair_one(cid)
        except Exception as e:
            r = {"error": f"unhandled: {e}"}
        summary.append((cid, r))

    banner("汇总")
    transitioned = [s for _, s in summary if s.get("before") == "rejected" and s.get("after") in ("verified", "needs_review")]
    failed = [s for _, s in summary if s.get("error")]
    print(f"  rejected → verified/needs_review: {len(transitioned)} / {len(summary)}")
    print(f"  失败: {len(failed)}")
    for cid, r in summary:
        print(f"    {cid}: before={r.get('before')} after={r.get('after')} err={r.get('error')}")


if __name__ == "__main__":
    main()
