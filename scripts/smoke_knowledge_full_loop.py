"""冒烟脚本：导入知识 → 预览 → 应用 → 列出 chunks → 选一条 needs_review 跑 AI 修复 → applied event。

跑法：python scripts/smoke_knowledge_full_loop.py
"""

from __future__ import annotations

import json
import os
import sys
import time
from pathlib import Path
from typing import Any

import urllib.request
import urllib.error

# Windows 默认 cp936 GBK 控制台无法打印中文 + ✓✗ 符号。强制 UTF-8。
if hasattr(sys.stdout, "reconfigure"):
    try:
        sys.stdout.reconfigure(encoding="utf-8")  # type: ignore[attr-defined]
        sys.stderr.reconfigure(encoding="utf-8")  # type: ignore[attr-defined]
    except Exception:
        pass

API = os.environ.get("WECHATAGENT_API", "http://127.0.0.1:8080/api")
ACCOUNT_ID = os.environ.get("SMOKE_ACCOUNT_ID", "1")
DOC_PATH = Path("docs/smoke/knowledge-smoke-doc.md")


def _request(method: str, path: str, body: dict[str, Any] | None = None, timeout: int = 180) -> Any:
    url = f"{API}{path}"
    data = None
    headers = {"Content-Type": "application/json"}
    if body is not None:
        data = json.dumps(body, ensure_ascii=False).encode("utf-8")
    req = urllib.request.Request(url, method=method, data=data, headers=headers)
    try:
        with urllib.request.urlopen(req, timeout=timeout) as r:
            txt = r.read().decode("utf-8")
            return json.loads(txt) if txt else {}
    except urllib.error.HTTPError as e:
        body_txt = e.read().decode("utf-8", errors="replace")
        raise SystemExit(f"HTTP {e.code} {method} {path}: {body_txt}")
    except urllib.error.URLError as e:
        raise SystemExit(f"URL error {method} {path}: {e}")


def banner(title: str) -> None:
    line = "=" * 78
    print(f"\n{line}\n{title}\n{line}", flush=True)


def main() -> None:
    if not DOC_PATH.exists():
        raise SystemExit(f"missing doc: {DOC_PATH}")

    content = DOC_PATH.read_text(encoding="utf-8")

    banner("[1/6] health")
    print(json.dumps(_request("GET", "/health"), ensure_ascii=False))

    banner("[2/6] import-preview")
    t0 = time.time()
    preview = _request(
        "POST",
        "/operation-knowledge/import-preview",
        {
            "accountId": ACCOUNT_ID,
            "sourceName": "OpsDesk 值班手册节选（冒烟）",
            "content": content,
        },
        timeout=300,
    )
    print(f"耗时 {time.time()-t0:.2f}s")
    doc_summary = preview.get("document", {})
    items = preview.get("items", [])
    chunks = preview.get("chunks", [])
    print(f"document.title = {doc_summary.get('title')!r}")
    print(f"document.routingMap = {doc_summary.get('routingMap')}")
    print(f"items 数量 = {len(items)}; titles = {[it.get('title') for it in items]}")
    print(f"chunks 数量 = {len(chunks)}; titles = {[c.get('title') for c in chunks]}")
    if not chunks:
        raise SystemExit("import-preview 没切出任何 chunk，停下排查")

    banner("[3/6] import-apply")
    t0 = time.time()
    applied = _request(
        "POST",
        "/operation-knowledge/import-apply",
        {
            "accountId": ACCOUNT_ID,
            "sourceName": "OpsDesk 值班手册节选（冒烟）",
            "document": doc_summary,
            "items": items,
            "chunks": chunks,
        },
        timeout=120,
    )
    print(f"耗时 {time.time()-t0:.2f}s")
    print(json.dumps(applied, ensure_ascii=False)[:600])
    document_id = applied.get("documentId") or applied.get("document", {}).get("id")
    print(f"documentId = {document_id}")

    banner("[4/6] list operation-knowledge / chunks")
    packs = _request("GET", "/operation-knowledge")
    pack_items = packs.get("items", [])
    print(f"知识包总数 = {len(pack_items)}; 最近 3 条标题 = {[p.get('title') for p in pack_items[:3]]}")

    chunks_resp = _request("GET", "/operation-knowledge/chunks")
    chunk_items = chunks_resp.get("items", [])
    print(f"切片总数 = {len(chunk_items)}")
    needs_review = [c for c in chunk_items if c.get("integrityStatus") != "verified"]
    print(f"需复核切片 = {len(needs_review)}; 选第一个跑 AI 修复")
    if not needs_review:
        print("没有 needs_review 切片可测，直接选第一个")
        needs_review = chunk_items
    if not needs_review:
        raise SystemExit("一条 chunk 都没有，停下")
    target_chunk = needs_review[0]
    chunk_id = target_chunk["id"]
    print(f"target chunk id = {chunk_id}; title = {target_chunk.get('title')!r}; integrity = {target_chunk.get('integrityStatus')}")

    banner("[5/6] AI 自主修复 chunk - propose")
    t0 = time.time()
    proposal = _request(
        "POST",
        f"/operation-knowledge/chunks/{chunk_id}/repair",
        timeout=240,
    )
    print(f"耗时 {time.time()-t0:.2f}s")
    print("interpretation = " + json.dumps(proposal.get("interpretation"), ensure_ascii=False))
    print(f"patch keys = {list((proposal.get('patch') or {}).keys())}")
    print(f"missingFields = {proposal.get('missingFields')}")
    print(f"followupQuestions = {proposal.get('followupQuestions')}")
    print(f"confidenceHint = {proposal.get('confidenceHint')}")

    # 如果有 followup，自动给一个简短回答继续测 answer 路径
    followups = proposal.get("followupQuestions") or []
    if followups:
        banner("[5b] AI 修复 - answer (turn 2)")
        answers = [
            {"id": q.get("id"), "field": q.get("field"), "text": "在内部值班场景下，按值班手册第 2 节执行；本切片需要在 P1 工单首响阶段打开。"}
            for q in followups
        ]
        proposal2 = _request(
            "POST",
            f"/operation-knowledge/chunks/{chunk_id}/repair/answer",
            {
                "sessionId": proposal.get("sessionId"),
                "previousPatch": proposal.get("patch"),
                "answers": answers,
                "turn": 2,
            },
            timeout=240,
        )
        print("answer turn 2 patch keys = " + str(list((proposal2.get("patch") or {}).keys())))
        print("answer stillMissing = " + str(proposal2.get("stillMissing")))
        proposal = proposal2  # 用最终 patch 走 applied

    banner("[6/6] AI 修复 - applied 事件上报")
    accepted = list((proposal.get("patch") or {}).keys())
    extras = (proposal.get("patch") or {}).get("extras")
    applied_evt = _request(
        "POST",
        "/operation-knowledge/repair/applied",
        {
            "targetKind": "chunk",
            "targetId": chunk_id,
            "sessionId": proposal.get("sessionId"),
            "turn": proposal.get("turn"),
            "acceptedFields": [k for k in accepted if k != "extras"],
            "skippedFields": [],
            "confidenceHint": proposal.get("confidenceHint"),
            "extras": extras,
            "thenVerify": False,
        },
        timeout=30,
    )
    print(json.dumps(applied_evt, ensure_ascii=False))

    banner("DONE")
    print("如需观察 mongo: db.agent_events.find({kind: /knowledge_repair/}).sort({_id:-1}).limit(5)")


if __name__ == "__main__":
    main()
