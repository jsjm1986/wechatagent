"""只读审计：扫 operation_knowledge_documents / items / chunks，找出与"全新通用
知识 schema"不兼容的旧数据。

不做任何删除/写操作。报告：
- 总数 / domain 分布 / sourceType 分布；
- 缺关键通用字段（catalogSummary / routingMap / triggerKeywords / businessTopics）的记录数；
- 仅有销售形态字段（commonObjections / customerStages 等填了但不属于通用语义）的记录；
- chunk 缺 sourceQuote+source_anchors 的记录（verify gate 通不过）；
- chunk integrityStatus 分布；
- 与新 schema 直接冲突的字段（如 chunk.body 为空 / pack 缺 routingCard）。

跑法：python scripts/diag/audit_knowledge_legacy.py
"""

from __future__ import annotations

import os
import sys
from collections import Counter
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

UNIVERSAL_DOC_FIELDS = ["catalog_summary", "routing_map", "trigger_keywords", "business_topics"]
UNIVERSAL_PACK_FIELDS = [
    "routing_card",
    "applicable_scenes",
    "trigger_keywords",
    "business_topics",
    "operation_states",
    "intent_levels",
]
UNIVERSAL_CHUNK_FIELDS = [
    "routing_card",
    "applicable_scenes",
    "trigger_keywords",
    "business_topics",
    "source_quote",
]
SALES_ONLY_FIELDS = [
    "customer_stages",
    "common_objections",
    "common_questions",
]


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


def field_fill_rate(coll, fields: list[str]) -> dict[str, int]:
    out = {f: 0 for f in fields}
    out["__total__"] = 0
    for d in coll.find({}, {f: 1 for f in fields}):
        out["__total__"] += 1
        for f in fields:
            if not is_blank(d.get(f)):
                out[f] += 1
    return out


def main() -> None:
    client = MongoClient(URI, serverSelectionTimeoutMS=5000)
    db = client[DB]

    banner(f"DB = {DB}")
    print("collections =", sorted(db.list_collection_names()))

    docs = db["operation_knowledge_documents"]
    items = db["operation_knowledge_items"]
    chunks = db["operation_knowledge_chunks"]

    banner("[1] documents 概览")
    total_docs = docs.count_documents({})
    print(f"total = {total_docs}")
    print("domain 分布:")
    for r in docs.aggregate(
        [{"$group": {"_id": "$domain", "n": {"$sum": 1}}}, {"$sort": {"n": -1}}]
    ):
        print(f"  {r['_id']!r}: {r['n']}")
    print("source_type 分布:")
    for r in docs.aggregate(
        [{"$group": {"_id": "$source_type", "n": {"$sum": 1}}}, {"$sort": {"n": -1}}]
    ):
        print(f"  {r['_id']!r}: {r['n']}")
    print("status 分布:")
    for r in docs.aggregate(
        [{"$group": {"_id": "$status", "n": {"$sum": 1}}}, {"$sort": {"n": -1}}]
    ):
        print(f"  {r['_id']!r}: {r['n']}")

    fill = field_fill_rate(docs, UNIVERSAL_DOC_FIELDS)
    print(f"通用字段填充率 (n={fill['__total__']}):")
    for f in UNIVERSAL_DOC_FIELDS:
        n = fill[f]
        pct = (100 * n / fill["__total__"]) if fill["__total__"] else 0
        flag = "  " if pct >= 50 else "⚠ "
        print(f"  {flag}{f}: {n}/{fill['__total__']} ({pct:.0f}%)")

    legacy_docs = list(
        docs.find(
            {
                "$or": [
                    {"catalog_summary": {"$in": [None, ""]}},
                    {"routing_map": {"$exists": False}},
                    {"routing_map": {"$size": 0}},
                ]
            },
            {"_id": 1, "title": 1, "source_name": 1, "domain": 1, "created_at": 1, "catalog_summary": 1, "routing_map": 1},
        ).limit(20)
    )
    print(f"\n旧格式 documents（缺 catalog_summary 或 routing_map）样本 ≤20:")
    for d in legacy_docs:
        print(
            f"  - {d.get('_id')} | domain={d.get('domain')!r} | title={(d.get('title') or '').strip()[:40]!r} | catalogSummary={'是' if d.get('catalog_summary') else '空'} | routingMap.len={len(d.get('routing_map') or [])}"
        )

    banner("[2] items (packs) 概览")
    total_items = items.count_documents({})
    print(f"total = {total_items}")
    for r in items.aggregate(
        [{"$group": {"_id": "$domain", "n": {"$sum": 1}}}, {"$sort": {"n": -1}}]
    ):
        print(f"  domain={r['_id']!r}: {r['n']}")
    fill = field_fill_rate(items, UNIVERSAL_PACK_FIELDS)
    print(f"通用字段填充率 (n={fill['__total__']}):")
    for f in UNIVERSAL_PACK_FIELDS:
        n = fill[f]
        pct = (100 * n / fill["__total__"]) if fill["__total__"] else 0
        flag = "  " if pct >= 50 else "⚠ "
        print(f"  {flag}{f}: {n}/{fill['__total__']} ({pct:.0f}%)")

    sales_only_packs = []
    for d in items.find({}, {"_id": 1, "title": 1, "domain": 1, "routing_card": 1, **{f: 1 for f in SALES_ONLY_FIELDS}, **{f: 1 for f in UNIVERSAL_PACK_FIELDS}}):
        has_sales = any(not is_blank(d.get(f)) for f in SALES_ONLY_FIELDS)
        has_universal = (
            not is_blank(d.get("routing_card"))
            and not is_blank(d.get("applicable_scenes"))
            and not is_blank(d.get("trigger_keywords"))
        )
        if has_sales and not has_universal:
            sales_only_packs.append(d)
    print(f"\n仅有销售字段、缺通用 routing_card/applicable_scenes/trigger_keywords 的 pack: {len(sales_only_packs)}")
    for d in sales_only_packs[:10]:
        print(
            f"  - {d.get('_id')} | domain={d.get('domain')!r} | title={(d.get('title') or '').strip()[:40]!r}"
        )

    banner("[3] chunks 概览")
    total_chunks = chunks.count_documents({})
    print(f"total = {total_chunks}")
    print("integrity_status 分布:")
    for r in chunks.aggregate(
        [{"$group": {"_id": "$integrity_status", "n": {"$sum": 1}}}, {"$sort": {"n": -1}}]
    ):
        print(f"  {r['_id']!r}: {r['n']}")
    print("status 分布:")
    for r in chunks.aggregate(
        [{"$group": {"_id": "$status", "n": {"$sum": 1}}}, {"$sort": {"n": -1}}]
    ):
        print(f"  {r['_id']!r}: {r['n']}")
    print("domain 分布:")
    for r in chunks.aggregate(
        [{"$group": {"_id": "$domain", "n": {"$sum": 1}}}, {"$sort": {"n": -1}}]
    ):
        print(f"  {r['_id']!r}: {r['n']}")

    fill = field_fill_rate(chunks, UNIVERSAL_CHUNK_FIELDS)
    print(f"通用字段填充率 (n={fill['__total__']}):")
    for f in UNIVERSAL_CHUNK_FIELDS:
        n = fill[f]
        pct = (100 * n / fill["__total__"]) if fill["__total__"] else 0
        flag = "  " if pct >= 50 else "⚠ "
        print(f"  {flag}{f}: {n}/{fill['__total__']} ({pct:.0f}%)")

    no_anchor = chunks.count_documents(
        {
            "$or": [
                {"source_quote": {"$in": [None, ""]}},
                {"source_anchors": {"$exists": False}},
                {"source_anchors": {"$size": 0}},
            ]
        }
    )
    print(f"chunk 缺 source_quote 或 source_anchors（verify gate 通不过）: {no_anchor}/{total_chunks}")

    no_routing = chunks.count_documents(
        {"$or": [{"routing_card": {"$in": [None, ""]}}, {"routing_card": {"$exists": False}}]}
    )
    print(f"chunk 缺 routing_card: {no_routing}/{total_chunks}")

    banner("[4] 旧/新 文档配对统计")
    new_doc_ids = set()
    for d in docs.find(
        {
            "catalog_summary": {"$nin": [None, ""]},
            "routing_map": {"$exists": True, "$not": {"$size": 0}},
        },
        {"_id": 1},
    ):
        new_doc_ids.add(d["_id"])
    legacy_doc_ids = set()
    for d in docs.find({}, {"_id": 1}):
        if d["_id"] not in new_doc_ids:
            legacy_doc_ids.add(d["_id"])
    print(f"新格式 documents = {len(new_doc_ids)}; 旧格式 documents = {len(legacy_doc_ids)}")

    legacy_chunks_n = chunks.count_documents({"document_id": {"$in": list(legacy_doc_ids)}}) if legacy_doc_ids else 0
    legacy_items_n = items.count_documents({"document_id": {"$in": list(legacy_doc_ids)}}) if legacy_doc_ids else 0
    print(f"挂在【旧文档】下的 packs = {legacy_items_n}; chunks = {legacy_chunks_n}")

    orphan_packs = items.count_documents({"document_id": {"$in": [None, ""]}})
    orphan_chunks = chunks.count_documents({"document_id": {"$in": [None, ""]}})
    print(f"孤儿 packs（无 document_id）= {orphan_packs}; 孤儿 chunks = {orphan_chunks}")

    banner("[5] 删除候选汇总（仅报告，不执行）")
    print(f"  documents 旧格式：{len(legacy_doc_ids)}")
    print(f"  这些文档下 packs：{legacy_items_n}")
    print(f"  这些文档下 chunks：{legacy_chunks_n}")
    print(f"  孤儿 packs：{orphan_packs}")
    print(f"  孤儿 chunks：{orphan_chunks}")
    print(f"  缺 source_quote/anchor 的 chunk：{no_anchor}")
    print(
        "\n建议：保留新格式 documents/packs/chunks；删除旧格式 documents 及其下属 packs+chunks；孤儿单独清理。\n"
        "下一步先把以上数字让用户确认后再执行删除。"
    )

    banner("DONE")


if __name__ == "__main__":
    main()
