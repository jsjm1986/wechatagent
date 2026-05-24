"""把 docs/ 下大文档按 H2 (## ) 边界切成 ≤2000 字符段，逐段走真实 LLM 导入。

每段独立 import-preview + import-apply，避免 LLM 长生成触发 chunked
body_decode_error。每段标题 = "{父文档标题} · {H2 标题}"，方便落库后查看。

跑法：python scripts/smoke_reimport_split.py
真实 LLM 驱动；不绕。
"""

from __future__ import annotations

import json
import os
import re
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path
from typing import Any, Iterator

if hasattr(sys.stdout, "reconfigure"):
    try:
        sys.stdout.reconfigure(encoding="utf-8")  # type: ignore[attr-defined]
        sys.stderr.reconfigure(encoding="utf-8")  # type: ignore[attr-defined]
    except Exception:
        pass

API = os.environ.get("WECHATAGENT_API", "http://127.0.0.1:8080/api")
ACCOUNT_ID = os.environ.get("SMOKE_ACCOUNT_ID", "1")
MAX_CHARS = int(os.environ.get("SMOKE_SEG_MAX_CHARS", "2000"))

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
        raise RuntimeError(f"HTTP {e.code} {method} {path}: {body_txt[:300]}")
    except (TimeoutError, OSError) as e:
        raise RuntimeError(f"NET {type(e).__name__} {method} {path}: {e}")


def _request_with_retry(method: str, path: str, body: dict[str, Any] | None = None, timeout: int = 300, retries: int = 2) -> Any:
    last: Exception | None = None
    for i in range(retries + 1):
        try:
            return _request(method, path, body, timeout)
        except RuntimeError as e:
            last = e
            print(f"  ↻ retry {i+1}/{retries+1} after {e}")
            time.sleep(2)
    assert last is not None
    raise last


def banner(t: str) -> None:
    print(f"\n{'=' * 78}\n{t}\n{'=' * 78}", flush=True)


def split_by_h2(content: str) -> list[tuple[str, str]]:
    """切到 (heading, body) 列表；body 已含 heading 行。第一段没 heading 的算前言。"""
    lines = content.splitlines(keepends=True)
    parts: list[tuple[str, list[str]]] = []
    current_head = ""
    current_body: list[str] = []
    for ln in lines:
        if re.match(r"^##\s+", ln):
            if current_body:
                parts.append((current_head, current_body))
            current_head = ln.strip().lstrip("#").strip()
            current_body = [ln]
        else:
            current_body.append(ln)
    if current_body:
        parts.append((current_head, current_body))
    return [(h, "".join(b)) for h, b in parts]


def chunk_segments(segments: list[tuple[str, str]], max_chars: int) -> Iterator[tuple[str, str]]:
    """合并相邻短段、切超长段。"""
    buf_head: list[str] = []
    buf_body: list[str] = []
    for head, body in segments:
        if len(body) > max_chars:
            # 已经累的 buf 先吐
            if buf_body:
                yield " / ".join([h for h in buf_head if h]) or "前言", "".join(buf_body)
                buf_head, buf_body = [], []
            # 大段按段落切
            paras = re.split(r"\n\n+", body)
            cur: list[str] = []
            cur_len = 0
            for p in paras:
                if cur_len + len(p) > max_chars and cur:
                    yield head, "\n\n".join(cur)
                    cur, cur_len = [], 0
                cur.append(p)
                cur_len += len(p) + 2
            if cur:
                yield head, "\n\n".join(cur)
            continue
        if sum(len(x) for x in buf_body) + len(body) > max_chars and buf_body:
            yield " / ".join([h for h in buf_head if h]) or "前言", "".join(buf_body)
            buf_head, buf_body = [], []
        buf_head.append(head)
        buf_body.append(body)
    if buf_body:
        yield " / ".join([h for h in buf_head if h]) or "前言", "".join(buf_body)


def import_one(name: str, sub_title: str, content: str, existing_sources: set[str]) -> dict[str, Any]:
    full_source = f"{name} · {sub_title}"
    if full_source in existing_sources:
        banner(f"[跳过] {full_source}（已存在）")
        return {"document": "skipped", "items": 0, "chunks": 0}
    banner(f"[预览] {full_source} ({len(content)} chars)")
    t0 = time.time()
    try:
        preview = _request_with_retry(
            "POST",
            "/operation-knowledge/import-preview",
            {"accountId": ACCOUNT_ID, "sourceName": full_source, "content": content},
            timeout=300,
            retries=2,
        )
    except RuntimeError as e:
        print(f"  ✗ preview 失败: {e}")
        return {"error": str(e)}
    print(f"  耗时 {time.time()-t0:.2f}s")
    doc = preview.get("document", {})
    items = preview.get("items", [])
    chunks = preview.get("chunks", [])
    rmap = doc.get("routingMap") or []
    print(
        f"  document.title={doc.get('title')!r} routingMap={len(rmap)} items={len(items)} chunks={len(chunks)}"
    )
    if not chunks:
        return {"document": None, "items": 0, "chunks": 0, "warn": "no_chunks"}

    banner(f"[落库] {full_source}")
    t0 = time.time()
    try:
        applied = _request_with_retry(
            "POST",
            "/operation-knowledge/import-apply",
            {
                "accountId": ACCOUNT_ID,
                "sourceName": full_source,
                "document": doc,
                "items": items,
                "chunks": chunks,
            },
            timeout=180,
            retries=1,
        )
    except RuntimeError as e:
        print(f"  ✗ apply 失败: {e}")
        return {"error": str(e)}
    print(f"  耗时 {time.time()-t0:.2f}s")
    document_id = applied.get("documentId") or applied.get("document", {}).get("id")
    print(f"  documentId={document_id}")
    return {"document": document_id, "items": len(items), "chunks": len(chunks)}


def main() -> None:
    banner("[起始] 健康检查")
    print(json.dumps(_request("GET", "/health"), ensure_ascii=False))

    # 加载现存的 sourceName 集合，跳过已导入的段
    existing_sources: set[str] = set()
    try:
        listed = _request("GET", "/operation-knowledge")
        for it in listed.get("items", []) or []:
            sn = it.get("sourceName")
            if sn:
                existing_sources.add(sn)
        # 文档级 sourceName 也参考一下
        listed_docs = _request("GET", "/operation-knowledge/documents")
        for it in listed_docs.get("items", []) or []:
            sn = it.get("sourceName")
            if sn:
                existing_sources.add(sn)
    except Exception as e:
        print(f"  ⚠ 加载现存 sourceName 失败: {e}")
    print(f"  已存在 sourceName: {len(existing_sources)} 条")

    summary: list[tuple[str, dict[str, Any]]] = []
    for path, name in DOCS:
        p = Path(path)
        if not p.exists():
            print(f"⚠ missing: {path}")
            continue
        full = p.read_text(encoding="utf-8")
        segs = split_by_h2(full)
        merged = list(chunk_segments(segs, MAX_CHARS))
        banner(f"[切片] {name} → {len(merged)} 段（每段 ≤ {MAX_CHARS} chars）")
        for h, b in merged:
            print(f"  - {h[:40]!r}: {len(b)} chars")
        for sub_title, body in merged:
            r = import_one(name, sub_title, body, existing_sources)
            summary.append((f"{name} · {sub_title}", r))

    banner("[全部完成] 汇总")
    ok = sum(1 for _, r in summary if "error" not in r and "warn" not in r)
    failed = [(n, r) for n, r in summary if "error" in r]
    warn = [(n, r) for n, r in summary if "warn" in r]
    print(f"  成功 {ok} / 失败 {len(failed)} / 警告 {len(warn)}")
    for name, r in failed:
        print(f"  ✗ {name}: {r['error'][:160]}")
    for name, r in warn:
        print(f"  ⚠ {name}: {r['warn']}")

    banner("[列表] packs / chunks 数量")
    packs = _request("GET", "/operation-knowledge")
    chunks_resp = _request("GET", "/operation-knowledge/chunks")
    print(f"  packs 总数 = {len(packs.get('items', []))}")
    print(f"  chunks 总数 = {len(chunks_resp.get('items', []))}")

    banner("DONE")


if __name__ == "__main__":
    main()
