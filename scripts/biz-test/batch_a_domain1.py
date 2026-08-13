"""域①：文章进知识库的分析整理能力。

import-preview 真调 LLM(knowledge.import.preview)析出 document/items/chunks，
验 document.riskNotes 留痕绝对承诺、sourceQuote 忠于原文、落库全 needs_review(红线)。

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
        {"accountId": account_id, "sourceName": "biztest_edu_article", "content": content},
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

    # 现行契约已删除 chunk/item.forbiddenClaims；绝对承诺统一留痕在 document.riskNotes。
    # 服务端有原文行级确定性下限，模型可补充更广的上下文风险，但不能漏掉显式保证/最高级宣传。
    document = preview.get("document", {}) if isinstance(preview, dict) else {}
    risk_notes = document.get("riskNotes", []) if isinstance(document, dict) else []
    risk_blob = "\n".join(str(note) for note in risk_notes)
    expected_markers = ("保证学会", "包教包会", "全市第一", "无条件退款", "保证考进")
    captured = [marker for marker in expected_markers if marker in risk_blob]
    _lib.expect(len(captured) >= 3, DOMAIN,
                "document.riskNotes 留痕多条原文绝对承诺",
                f"captured={captured} riskNotes={risk_notes}", "high",
                "显式保证/最高级宣传未进入现行 riskNotes 审计契约")

    # import-apply 落库 → 验红线：全 needs_review
    print(f"[{DOMAIN}] import-apply 落库...")
    applied = _lib.api(
        "POST", "/api/operation-knowledge/import-apply",
        {
            "previewId": preview.get("previewId"),
            "previewHash": preview.get("previewHash"),
            "chunks": [
                {"candidateId": chunk.get("candidateId"), "patch": {}}
                for chunk in chunks
            ],
        },
        admin=True, timeout=120,
    )
    print(f"  applied={str(applied)[:200]}")
    doc_id = applied.get("documentId") if isinstance(applied, dict) else None
    chunk_ids = applied.get("chunkIds", []) if isinstance(applied, dict) else []
    time.sleep(2)
    # chunk 无 source_name 字段（在 document 层）；按 apply 返回的 documentId 关联查询。
    # document_id 是 ObjectId 类型，字符串查询命中 0，必须 ObjectId() 包裹。
    if not isinstance(doc_id, str) or len(doc_id) != 24:
        _lib.expect(False, DOMAIN, "import-apply 返回合法 documentId",
                    f"applied={applied}", "critical", "无法回读持久化导入结果")
        return
    rows = _lib.mongo_json(
        f'db.operation_knowledge_chunks.find({{document_id:ObjectId("{doc_id}")}},'
        '{integrity_status:1,status:1,_id:0}).toArray()'
    )
    rows = rows if isinstance(rows, list) else []
    stored_doc = _lib.mongo_json(
        f'db.operation_knowledge_documents.findOne({{_id:ObjectId("{doc_id}")}},'
        '{risk_notes:1,_id:0})'
    )
    stored_risks = stored_doc.get("risk_notes", []) if isinstance(stored_doc, dict) else []
    stored_blob = "\n".join(str(note) for note in stored_risks)
    _lib.expect(all(marker in stored_blob for marker in captured), DOMAIN,
                "riskNotes 随 Document 持久化且未在 Apply 丢失",
                f"captured={captured} stored={stored_risks}", "critical",
                "预览风险提示未持久化，运营审核界面无法追溯")
    all_review = bool(rows) and all(r.get("integrity_status") == "needs_review" for r in rows)
    _lib.expect(all_review, DOMAIN, "落库全 needs_review(AI 永不自动 verify 红线)",
                f"doc_id={doc_id} chunkIds={len(chunk_ids)} rows={rows}", "critical",
                "若有 verified=红线破")

    print(f"[{DOMAIN}] 完成。落库 {len(rows)} chunks, riskNotes {len(stored_risks)} 条")


if __name__ == "__main__":
    main()
