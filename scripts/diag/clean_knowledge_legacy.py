"""清理旧格式知识数据。默认 dry-run：只打印将要删除的 _id，不写入。

执行实删：CLEAN_KNOWLEDGE_APPLY=1 python scripts/diag/clean_knowledge_legacy.py

删除范围（用户选 C 档）：
  1. 旧格式 documents（缺 catalog_summary 或 routing_map）+ 它们下属的 packs/chunks
  2. 孤儿 packs（document_id 为空 / null）+ 这些 pack 下属的 chunks
  3. 缺 source_quote 或 source_anchors 的 chunks（verify gate 永远通不过）

执行顺序刻意先 chunks → packs → documents，避免悬空引用。
"""

from __future__ import annotations

import os
import sys
from typing import Any

if hasattr(sys.stdout, "reconfigure"):
    try:
        sys.stdout.reconfigure(encoding="utf-8")  # type: ignore[attr-defined]
        sys.stderr.reconfigure(encoding="utf-8")  # type: ignore[attr-defined]
    except Exception:
        pass

try:
    from pymongo import MongoClient
except ImportError:
    raise SystemExit("缺 pymongo: pip install pymongo")

URI = os.environ.get("MONGODB_URI", "mongodb://127.0.0.1:27017")
DB = os.environ.get("MONGODB_DATABASE", "wechatagent")
APPLY = os.environ.get("CLEAN_KNOWLEDGE_APPLY", "0") == "1"


def banner(t: str) -> None:
    line = "=" * 78
    print(f"\n{line}\n{t}\n{line}", flush=True)


def is_blank(v: Any) -> bool:
    if v is None:
        return True
    if isinstance(v, str):
        return v.strip() == ""
    if isinstance(v, (list, dict)):
        return len(v) == 0
    return False


def main() -> None:
    client = MongoClient(URI, serverSelectionTimeoutMS=5000)
    db = client[DB]

    docs = db["operation_knowledge_documents"]
    items = db["operation_knowledge_items"]
    chunks = db["operation_knowledge_chunks"]

    banner(f"MODE = {'APPLY (将真实删除)' if APPLY else 'DRY-RUN (仅打印不删除)'}")
    print(f"DB = {DB}")

    legacy_doc_ids: list[str] = []
    for d in docs.find({}, {"_id": 1, "title": 1, "source_name": 1, "catalog_summary": 1, "routing_map": 1}):
        if is_blank(d.get("catalog_summary")) or is_blank(d.get("routing_map")):
            legacy_doc_ids.append(d["_id"])
    print(f"\n[1] 旧格式 documents 待删 = {len(legacy_doc_ids)}")
    for d in docs.find({"_id": {"$in": legacy_doc_ids}}, {"_id": 1, "title": 1, "source_name": 1, "domain": 1}):
        print(f"  - {d.get('_id')} | domain={d.get('domain')!r} | title={(d.get('title') or '').strip()[:50]!r}")

    orphan_pack_ids: list[str] = []
    for p in items.find(
        {"$or": [{"document_id": {"$in": [None, ""]}}, {"document_id": {"$exists": False}}]},
        {"_id": 1, "title": 1, "domain": 1},
    ):
        orphan_pack_ids.append(p["_id"])
    print(f"\n[2] 孤儿 packs 待删 = {len(orphan_pack_ids)}")
    for p in items.find({"_id": {"$in": orphan_pack_ids}}, {"_id": 1, "title": 1}).limit(40):
        print(f"  - {p.get('_id')} | title={(p.get('title') or '').strip()[:50]!r}")

    bad_chunk_ids: list[str] = []
    for c in chunks.find(
        {
            "$or": [
                {"source_quote": {"$in": [None, ""]}},
                {"source_anchors": {"$exists": False}},
                {"source_anchors": {"$size": 0}},
            ]
        },
        {"_id": 1, "title": 1, "integrity_status": 1, "document_id": 1, "item_id": 1},
    ):
        bad_chunk_ids.append(c["_id"])
    print(f"\n[3] 缺 source_quote/anchor 的 chunks 待删 = {len(bad_chunk_ids)}")
    for c in chunks.find(
        {"_id": {"$in": bad_chunk_ids}},
        {"_id": 1, "title": 1, "integrity_status": 1},
    ).limit(40):
        print(
            f"  - {c.get('_id')} | integrity={c.get('integrity_status')!r} | title={(c.get('title') or '').strip()[:50]!r}"
        )
    if len(bad_chunk_ids) > 40:
        print(f"  ... 共 {len(bad_chunk_ids)} 条，仅显示前 40")

    chunks_under_legacy_doc: list[str] = []
    if legacy_doc_ids:
        for c in chunks.find({"document_id": {"$in": legacy_doc_ids}}, {"_id": 1}):
            chunks_under_legacy_doc.append(c["_id"])
    print(f"\n[4] 旧格式 documents 下属 chunks = {len(chunks_under_legacy_doc)} (并入 [3] 里)")

    chunks_under_orphan_pack: list[str] = []
    if orphan_pack_ids:
        for c in chunks.find({"item_id": {"$in": orphan_pack_ids}}, {"_id": 1}):
            chunks_under_orphan_pack.append(c["_id"])
    print(f"\n[5] 孤儿 packs 下属 chunks = {len(chunks_under_orphan_pack)} (并入 [3] 里)")

    all_chunk_to_delete = set(bad_chunk_ids) | set(chunks_under_legacy_doc) | set(chunks_under_orphan_pack)
    all_pack_to_delete = set(orphan_pack_ids)
    all_doc_to_delete = set(legacy_doc_ids)

    banner("汇总")
    print(f"  documents 删除：{len(all_doc_to_delete)}")
    print(f"  packs 删除：{len(all_pack_to_delete)}")
    print(f"  chunks 删除（取并集）：{len(all_chunk_to_delete)}")

    if not APPLY:
        print("\nDRY-RUN：未执行任何删除。确认后用 CLEAN_KNOWLEDGE_APPLY=1 重跑。")
        return

    banner("EXECUTE 实删")
    if all_chunk_to_delete:
        r = chunks.delete_many({"_id": {"$in": list(all_chunk_to_delete)}})
        print(f"  chunks deleted = {r.deleted_count}")
    if all_pack_to_delete:
        r = items.delete_many({"_id": {"$in": list(all_pack_to_delete)}})
        print(f"  packs deleted = {r.deleted_count}")
    if all_doc_to_delete:
        r = docs.delete_many({"_id": {"$in": list(all_doc_to_delete)}})
        print(f"  documents deleted = {r.deleted_count}")

    banner("剩余统计")
    print(f"  documents 剩余 = {docs.count_documents({})}")
    print(f"  packs 剩余 = {items.count_documents({})}")
    print(f"  chunks 剩余 = {chunks.count_documents({})}")


if __name__ == "__main__":
    main()
