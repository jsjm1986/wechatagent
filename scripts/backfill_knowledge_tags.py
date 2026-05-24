"""
v3 prompt-pack / Task 271: backfill productTags / triggerKeywords / businessTopics
on existing operation_knowledge_chunks.

Reads chunks where any of the three tag fields is missing or empty, calls
`POST /api/operation-knowledge/extract-tags` for each, and writes the result
back via direct pymongo update_one. Idempotent on re-run (already-tagged
chunks are skipped).

Usage:
    python scripts/backfill_knowledge_tags.py [--dry-run] [--limit N]
"""

from __future__ import annotations

import argparse
import os
import sys
from datetime import datetime, timezone

import requests
from pymongo import MongoClient


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--dry-run", action="store_true",
                        help="print actions without writing")
    parser.add_argument("--limit", type=int, default=0,
                        help="cap number of chunks processed (0 = all)")
    parser.add_argument("--api-base", default=os.environ.get(
        "WECHATAGENT_API_BASE", "http://localhost:8080"))
    parser.add_argument("--mongo-uri", default=os.environ.get(
        "MONGODB_URI", "mongodb://localhost:27017"))
    parser.add_argument("--mongo-db", default=os.environ.get(
        "MONGODB_DATABASE", "wechatagent"))
    args = parser.parse_args()

    client = MongoClient(args.mongo_uri)
    db = client[args.mongo_db]
    coll = db["operation_knowledge_chunks"]

    query = {
        "$or": [
            {"trigger_keywords": {"$exists": False}},
            {"trigger_keywords": {"$size": 0}},
        ]
    }
    chunks = list(coll.find(query))
    if args.limit > 0:
        chunks = chunks[: args.limit]

    print(f"[backfill] candidates: {len(chunks)} chunks", file=sys.stderr)

    ok = 0
    skipped = 0
    failed = 0
    for chunk in chunks:
        chunk_id = chunk["_id"]
        title = chunk.get("title", "")
        body = chunk.get("body") or chunk.get("summary") or ""
        if not body.strip():
            print(f"[skip-empty] {chunk_id} (no body/summary)", file=sys.stderr)
            skipped += 1
            continue
        try:
            resp = requests.post(
                f"{args.api_base}/api/operation-knowledge/extract-tags",
                json={
                    "title": title,
                    "body": body,
                    "accountId": chunk.get("account_id"),
                },
                timeout=60,
            )
            resp.raise_for_status()
            tags = resp.json()
        except Exception as exc:
            print(f"[fail-llm] {chunk_id} title={title!r}: {exc}",
                  file=sys.stderr)
            failed += 1
            continue

        product_tags = tags.get("productTags", []) or []
        trigger_keywords = tags.get("triggerKeywords", []) or []
        business_topics = tags.get("businessTopics", []) or []

        print(
            f"[ok] {chunk_id} | title={title[:30]!r} | "
            f"product={product_tags} | triggers={trigger_keywords} | "
            f"topics={business_topics}",
            file=sys.stderr,
        )
        if args.dry_run:
            ok += 1
            continue

        coll.update_one(
            {"_id": chunk_id},
            {
                "$set": {
                    "product_tags": product_tags,
                    "trigger_keywords": trigger_keywords,
                    "business_topics": business_topics,
                    "updated_at": datetime.now(timezone.utc),
                }
            },
        )
        ok += 1

    print(
        f"[done] ok={ok} skipped={skipped} failed={failed} "
        f"dry_run={args.dry_run}",
        file=sys.stderr,
    )
    return 0 if failed == 0 else 2


if __name__ == "__main__":
    sys.exit(main())
