"""域①：文章进知识库的分析整理能力。

import-preview 真调 LLM(knowledge.import.preview)析出 document/items/chunks，
验 forbiddenClaims 识别营销夸大、sourceQuote 忠于原文、落库全 needs_review(红线)。

跑法：export DEPLOY_PASS=...; python scripts/biz-test/batch_a_domain1.py
"""
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
import _lib

DOMAIN = "①文章进库"
ARTICLE = (Path(__file__).resolve().parents[2] / "docs/smoke/biztest-article-edu.md")


def main() -> None:
    account_id, _app_id = _lib.biztest_account()
    content = ARTICLE.read_text(encoding="utf-8")

    print(f"[{DOMAIN}] import-preview（真调 LLM 分析）...")
    t0 = time.time()
    preview = _lib.api_bg(
        "POST", "/api/operation-knowledge/import-preview",
        {"sourceName": "biztest_edu_article", "content": content},
        admin=True, max_wait=720, tag="import",
    )
    print(f"  耗时 {time.time()-t0:.1f}s")
    chunks = preview.get("chunks", []) if isinstance(preview, dict) else []
    _lib.expect(len(chunks) > 0, DOMAIN, "LLM 析出至少 1 个 chunk",
                f"preview={str(preview)[:300]}", "critical")
    if not chunks:
        return

    # 真调铁证
    _lib.assert_llm_success(300, "knowledge.import.preview", DOMAIN)

    # chunk 结构：含 sourceQuote
    has_quote = any(c.get("sourceQuote") for c in chunks)
    _lib.expect(has_quote, DOMAIN, "chunk 含 sourceQuote(忠于原文)",
                f"keys={list(chunks[0].keys())}", "high")

    # forbiddenClaims 真识别营销夸大（文章含"保证学会/包教包会/全市第一/无条件退款"）
    all_forbidden = [f for c in chunks for f in (c.get("forbiddenClaims") or [])]
    doc_forbidden = []
    items = preview.get("items", [])
    for it in items:
        doc_forbidden += it.get("forbiddenClaims") or []
    total_forbidden = all_forbidden + doc_forbidden
    _lib.expect(len(total_forbidden) > 0, DOMAIN, "识别出营销夸大(forbiddenClaims 非空)",
                f"forbidden={total_forbidden}", "high",
                "文章含'保证学会/包教包会/全市第一'等夸大,应进 forbiddenClaims")

    # import-apply 落库 → 验红线：全 needs_review
    doc = preview.get("document", {})
    print(f"[{DOMAIN}] import-apply 落库...")
    applied = _lib.api(
        "POST", "/api/operation-knowledge/import-apply",
        {"accountId": account_id, "sourceName": "biztest_edu_article",
         "document": doc, "items": items, "chunks": chunks},
        admin=True, timeout=120,
    )
    print(f"  applied={str(applied)[:200]}")
    doc_id = applied.get("documentId") if isinstance(applied, dict) else None
    chunk_ids = applied.get("chunkIds", []) if isinstance(applied, dict) else []
    time.sleep(2)
    # chunk 无 source_name 字段（在 document 层）；按 apply 返回的 documentId 关联查询。
    # document_id 是 ObjectId 类型，字符串查询命中 0，必须 ObjectId() 包裹。
    rows = _lib.mongo_json(
        f'db.operation_knowledge_chunks.find({{document_id:ObjectId("{doc_id}")}},'
        '{integrity_status:1,status:1,_id:0}).toArray()'
    )
    rows = rows if isinstance(rows, list) else []
    all_review = bool(rows) and all(r.get("integrity_status") == "needs_review" for r in rows)
    _lib.expect(all_review, DOMAIN, "落库全 needs_review(AI 永不自动 verify 红线)",
                f"doc_id={doc_id} chunkIds={len(chunk_ids)} rows={rows}", "critical",
                "若有 verified=红线破")

    print(f"[{DOMAIN}] 完成。落库 {len(rows)} chunks, forbiddenClaims {len(total_forbidden)} 条")


if __name__ == "__main__":
    main()
