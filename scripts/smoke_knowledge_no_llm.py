"""绕 LLM 的冒烟脚本：手工构造 document/pack/chunk 落库 → 跑 chunks 列表 →
直接 POST apply event → 看 agent_events 是否落地。

为什么需要：本机当前 reqwest → DeepSeek connect 阶段失败（502 in 6-7s），
import-preview / propose chunk repair 等"靠 LLM"的端点跑不通。本脚本验证
**非 LLM 路径**（CRUD + apply audit event），同时把 LLM 失败这件事独立标记
为冒烟暴露的问题 #1。
"""
from __future__ import annotations

import json
import os
import sys
import time
import urllib.request
import urllib.error
from pathlib import Path
from typing import Any

# Windows 默认 cp936 GBK 控制台无法打印 ✓ ✗ 等符号。强制 UTF-8。
if hasattr(sys.stdout, "reconfigure"):
    try:
        sys.stdout.reconfigure(encoding="utf-8")  # type: ignore[attr-defined]
        sys.stderr.reconfigure(encoding="utf-8")  # type: ignore[attr-defined]
    except Exception:
        pass

API = os.environ.get("WECHATAGENT_API", "http://127.0.0.1:8080/api")
ACCOUNT_ID = os.environ.get("SMOKE_ACCOUNT_ID", "1")
DOC_PATH = Path("docs/smoke/knowledge-smoke-doc.md")


def _request(method: str, path: str, body: dict[str, Any] | None = None, timeout: int = 60) -> Any:
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


def banner(title: str) -> None:
    line = "=" * 78
    print(f"\n{line}\n{title}\n{line}", flush=True)


def main() -> None:
    if not DOC_PATH.exists():
        raise SystemExit(f"missing doc: {DOC_PATH}")
    content = DOC_PATH.read_text(encoding="utf-8")

    banner("[A1] health & accounts")
    print("health =", _request("GET", "/health"))

    banner("[A2] 手工创建 document（绕 LLM）")
    # 使用 import-apply 但 chunks 段我们手填（以避免任何 LLM 触发）。
    # 先调 create_operation_knowledge_document
    doc_body = {
        "accountId": ACCOUNT_ID,
        "domain": "user_operations",
        "sourceType": "manual",
        "sourceName": "OpsDesk 值班手册节选（冒烟·绕 LLM）",
        "title": "OpsDesk 值班手册节选",
        "summary": "内部工单平台值班 SRE 的处置 playbook，覆盖 P1 首响、DB 主备切换、升级路径。",
        "catalogSummary": "内部工单平台 OpsDesk 值班 SOP，遇到 P1 工单首响、数据库主备切换、严重度升降级时打开。",
        "routingMap": ["P1 工单首响 30 分钟内动作", "MySQL 主备切换前置检查", "工单严重度升降级路径"],
        "riskNotes": ["binlog 延迟 ≥30s 时强切会丢交易"],
        "productTags": ["OpsDesk"],
        "triggerKeywords": ["p1", "工单", "主备切换", "binlog", "升级"],
        "businessTopics": ["值班 SOP", "数据库切换", "工单升级"],
        "rawContent": content,
        "status": "active",
    }
    doc_resp = _request("POST", "/operation-knowledge/documents", doc_body)
    document_id = doc_resp.get("id") or doc_resp.get("documentId")
    print(f"document_id = {document_id}")
    if not document_id:
        raise SystemExit(f"no document id in response: {doc_resp}")

    banner("[A3] 手工创建 pack")
    pack_body = {
        "accountId": ACCOUNT_ID,
        "domain": "user_operations",
        "category": "运维 SOP",
        "businessType": "内部值班",
        "knowledgeType": "应急处置",
        "businessContext": "OpsDesk 值班 SRE 在工单生命周期内的处置规则",
        "title": "OpsDesk 值班 SOP",
        "summary": "P1 首响 30 分钟、DB 主备切换前置、严重度升降级路径",
        "body": "见 chunks",
        "routingCard": "当工单 severity≥P2 或涉及 MySQL 主备切换时打开",
        "applicableScenes": ["P1 工单首响阶段", "MySQL 主备切换前置检查", "严重度升降级判断"],
        "notApplicableScenes": ["纯外部客户问题", "P3/P4 例行处理"],
        "operationStates": ["triaged", "acknowledged", "mitigating"],
        "intentLevels": ["紧急", "常规"],
        "safeClaims": ["P1 必须 30 分钟首响", "binlog 延迟 < 30 秒才能主备切换"],
        "forbiddenClaims": ["P1 可以下班再处理", "可以跳过 manager 自行下调严重度"],
        "commonQuestions": ["P1 来了第一步做什么？", "DB 主备切换前要看什么？"],
        "commonObjections": ["这次先扛过去明天再说", "binlog 延迟一下没事"],
        "evidenceItems": ["值班手册第 2.1 节", "值班手册第 2.2 节"],
        "productTags": ["OpsDesk", "MySQL"],
        "triggerKeywords": ["p1", "工单", "主备切换", "binlog", "升级"],
        "businessTopics": ["值班 SOP", "数据库切换"],
        "status": "active",
        "priority": 80,
    }
    pack_resp = _request("POST", "/operation-knowledge", pack_body)
    pack_id = pack_resp.get("id")
    print(f"pack_id = {pack_id}")

    banner("[A4] 手工创建 1 条 chunk（故意缺 sourceQuote 测 verify gate）")
    chunk_body = {
        "accountId": ACCOUNT_ID,
        "documentId": document_id,
        "itemId": pack_id,
        "domain": "user_operations",
        "knowledgeType": "应急处置",
        "businessContext": "P1 工单首响 SOP",
        "title": "P1 工单首响 30 分钟内必须做的事",
        "summary": "签收 + 作战频道 + 首条状态更新 + 升级判断",
        "body": "P1 出现时值班机器人会 @ SRE。30 分钟内必须签收、`/incident open` 创建作战频道、发布首条状态更新（含怀疑方向 / 下一步动作 / 下次更新时间）、若 5 分钟内无法定位影响面则升 P0。",
        "routingCard": "工单 severity=P1 且 status=triaged 时打开",
        "applicableScenes": ["P1 工单刚到值班 SRE 手上"],
        "notApplicableScenes": ["P3/P4 例行工单"],
        "safeClaims": ["未签收 P1 在 25 分钟时触发二次告警", "首条状态更新必须含 4 段"],
        "forbiddenClaims": ["可以只发\"在看\"", "可以晚点再签收"],
        "evidenceItems": ["值班手册第 2.1 节"],
        # 故意不传 sourceQuote / sourceAnchors，期望 verify endpoint 拒绝
        "integrityStatus": "needs_review",
        "confidenceScore": 60,
        "status": "active",
        "priority": 50,
        "productTags": ["OpsDesk"],
        "triggerKeywords": ["p1", "工单", "签收", "首响"],
        "businessTopics": ["值班 SOP"],
    }
    chunk_resp = _request("POST", "/operation-knowledge/chunks", chunk_body)
    chunk_id = chunk_resp.get("id")
    print(f"chunk_id = {chunk_id}; integrity = {chunk_resp.get('integrityStatus')}")

    banner("[A5] 列出 chunks 看刚刚那条在不在")
    chunks_resp = _request("GET", "/operation-knowledge/chunks")
    items = chunks_resp.get("items", [])
    print(f"chunks 总数 = {len(items)}")
    found = next((c for c in items if c.get("id") == chunk_id), None)
    print(f"刚创建的 chunk 命中 = {bool(found)}; integrityStatus = {found.get('integrityStatus') if found else None}")

    banner("[A6] 直接 POST verify → 应该被 chunk_verify_gate 拒绝（缺 sourceQuote）")
    try:
        _request("POST", f"/operation-knowledge/chunks/{chunk_id}/verify", {})
        print("✗ FAIL：verify 居然 200 了，gate 没生效！")
    except SystemExit as e:
        msg = str(e)
        if "sourceQuote" in msg or "source_anchors" in msg:
            print(f"✓ verify gate 正确拒绝：{msg}")
        else:
            print(f"⚠ verify 拒绝了，但理由不是 gate 期望的：{msg}")

    banner("[A7] 跑 apply event 上报（验证 #318 收口的端点真的能写）")
    apply_resp = _request(
        "POST",
        "/operation-knowledge/repair/applied",
        {
            "targetKind": "chunk",
            "targetId": chunk_id,
            "sessionId": "smoke-session-A7",
            "turn": 1,
            "acceptedFields": ["routingCard", "summary"],
            "skippedFields": ["sourceQuote"],
            "confidenceHint": 72,
            "extras": {"compliance_band": "internal_sre", "domain_hint": "ops_runbook"},
            "thenVerify": False,
        },
    )
    print(json.dumps(apply_resp, ensure_ascii=False))

    banner("[A8] 跑 apply event 上报 - extras=null")
    apply_resp2 = _request(
        "POST",
        "/operation-knowledge/repair/applied",
        {
            "targetKind": "pack",
            "targetId": pack_id,
            "acceptedFields": ["routingCard"],
            "skippedFields": [],
            "thenVerify": True,
        },
    )
    print(json.dumps(apply_resp2, ensure_ascii=False))

    banner("[A9] 跑 apply event - 错误的 targetKind 必须 400")
    try:
        _request("POST", "/operation-knowledge/repair/applied", {"targetKind": "garbage", "targetId": "x"})
        print("✗ FAIL：未知 targetKind 居然通过")
    except SystemExit as e:
        if "400" in str(e):
            print(f"✓ 400 正确拒绝：{e}")
        else:
            print(f"⚠ 拒绝了，但 status code 不是 400：{e}")

    banner("[A10] 列出 events 看 knowledge_repair_applied 是否落地")
    evt = _request(
        "GET",
        f"/events?kind=knowledge_repair_applied&accountId={ACCOUNT_ID}&limit=20",
    )
    print(f"events 总数 = {len(evt.get('items', []))}")
    for it in evt.get("items", [])[:3]:
        print("  -", it.get("kind"), "|", it.get("summary"))

    banner("DONE")
    print(f"\n用于 mongosh 检查的 ID:")
    print(f"  document_id = {document_id}")
    print(f"  pack_id     = {pack_id}")
    print(f"  chunk_id    = {chunk_id}")


if __name__ == "__main__":
    main()
