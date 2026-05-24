"""把 docs/ 里几份正式文档用"全新通用知识库流程"重新导入，并在每份导入后
打印 documents/packs/chunks 的累计变化和填充率。

每份文档独立 import-preview + import-apply，避免单条 LLM 长生成串到极限。
跑法：python scripts/smoke_reimport_docs.py
真实 LLM 驱动；不绕。
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

if hasattr(sys.stdout, "reconfigure"):
    try:
        sys.stdout.reconfigure(encoding="utf-8")  # type: ignore[attr-defined]
        sys.stderr.reconfigure(encoding="utf-8")  # type: ignore[attr-defined]
    except Exception:
        pass

API = os.environ.get("WECHATAGENT_API", "http://127.0.0.1:8080/api")
ACCOUNT_ID = os.environ.get("SMOKE_ACCOUNT_ID", "1")

DOCS = [
    ("docs/wechatagent-project-knowledge.md", "WechatAgent 项目内部知识"),
    ("docs/sales-positioning-knowledge.md", "WechatAgent 销售口径与边界"),
    ("docs/product-modules.md", "WechatAgent 产品模块说明"),
    ("docs/agent-policy.md", "WechatAgent Agent 策略与守门"),
]


def _request(method: str, path: str, body: dict[str, Any] | None = None, timeout: int = 300) -> Any:
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


def import_one(path: str, name: str) -> dict[str, Any]:
    p = Path(path)
    if not p.exists():
        raise SystemExit(f"missing: {path}")
    content = p.read_text(encoding="utf-8")
    banner(f"[预览] {name} ({path}, {len(content)} chars)")
    t0 = time.time()
    preview = _request(
        "POST",
        "/operation-knowledge/import-preview",
        {"accountId": ACCOUNT_ID, "sourceName": name, "content": content},
        timeout=600,
    )
    print(f"  耗时 {time.time()-t0:.2f}s")
    doc_summary = preview.get("document", {})
    items = preview.get("items", [])
    chunks = preview.get("chunks", [])
    print(f"  document.title = {doc_summary.get('title')!r}")
    rmap = doc_summary.get("routingMap") or []
    print(f"  document.routingMap (n={len(rmap)}) = {rmap[:3]}")
    print(f"  items = {len(items)} | chunks = {len(chunks)}")
    if not chunks:
        print("  ⚠ 没切出 chunk，跳过 apply")
        return {"document": None, "items": 0, "chunks": 0}

    banner(f"[落库] {name}")
    t0 = time.time()
    applied = _request(
        "POST",
        "/operation-knowledge/import-apply",
        {
            "accountId": ACCOUNT_ID,
            "sourceName": name,
            "document": doc_summary,
            "items": items,
            "chunks": chunks,
        },
        timeout=180,
    )
    print(f"  耗时 {time.time()-t0:.2f}s")
    document_id = applied.get("documentId") or applied.get("document", {}).get("id")
    print(f"  documentId = {document_id}")
    return {"document": document_id, "items": len(items), "chunks": len(chunks)}


def main() -> None:
    banner("[起始] mongo 现状")
    print(json.dumps(_request("GET", "/health"), ensure_ascii=False))

    summary = []
    for path, name in DOCS:
        try:
            r = import_one(path, name)
            summary.append((name, r))
        except SystemExit as e:
            print(f"⚠ {name} 失败：{e}")
            summary.append((name, {"error": str(e)}))

    banner("[全部完成]")
    for name, r in summary:
        if "error" in r:
            print(f"  ✗ {name}: {r['error']}")
        else:
            print(f"  ✓ {name}: doc={r.get('document')} items={r.get('items')} chunks={r.get('chunks')}")

    banner("[列表] packs / chunks 数量")
    packs = _request("GET", "/operation-knowledge")
    chunks_resp = _request("GET", "/operation-knowledge/chunks")
    print(f"  packs 总数 = {len(packs.get('items', []))}")
    print(f"  chunks 总数 = {len(chunks_resp.get('items', []))}")

    banner("DONE")


if __name__ == "__main__":
    main()
