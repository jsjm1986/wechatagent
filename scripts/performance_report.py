#!/usr/bin/env python3
"""Fetch and print the authenticated Gateway performance baseline endpoint."""

import argparse
import json
import os
import sys
import urllib.parse
import urllib.request


def metric(label, value):
    if not value or value.get("count", 0) == 0:
        return f"{label}: no samples"
    return (
        f"{label}: n={value['count']} mean={value['mean']:.1f} "
        f"p50={value['p50']} p95={value['p95']} "
        f"p99={value['p99']} max={value['max']}"
    )


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--base-url",
        default=os.getenv("WECHATAGENT_BASE_URL", "http://127.0.0.1:3000"),
    )
    parser.add_argument("--hours", type=int, default=24)
    parser.add_argument("--account-id")
    parser.add_argument(
        "--path",
        choices=["direct", "escalated", "rewrite", "revision", "no_reply", "manual"],
    )
    parser.add_argument(
        "--session", default=os.getenv("WA_SESSION"), help="wa_session cookie value"
    )
    parser.add_argument("--bearer", default=os.getenv("WA_BEARER_TOKEN"))
    parser.add_argument("--json", action="store_true")
    args = parser.parse_args()

    query = {"hours": args.hours}
    if args.account_id:
        query["accountId"] = args.account_id
    if args.path:
        query["path"] = args.path
    url = (
        args.base_url.rstrip("/")
        + "/api/admin/observability/performance?"
        + urllib.parse.urlencode(query)
    )
    request = urllib.request.Request(url)
    if args.session:
        request.add_header("Cookie", f"wa_session={args.session}")
    if args.bearer:
        request.add_header("Authorization", f"Bearer {args.bearer}")
    if not args.session and not args.bearer:
        parser.error("provide --session/WA_SESSION or --bearer/WA_BEARER_TOKEN")

    try:
        with urllib.request.urlopen(request, timeout=30) as response:
            body = json.load(response)
    except Exception as exc:  # CLI boundary: present network/JSON errors uniformly.
        print(f"performance report failed: {exc}", file=sys.stderr)
        return 1
    if args.json:
        print(json.dumps(body, ensure_ascii=False, indent=2))
        return 0

    print(
        f"Gateway performance: {body['windowStart']} -> {body['asOf']} "
        f"truncated={body['truncated']}"
    )
    print(metric("overall.totalMs", body["overall"]["totalMs"]))
    for path, bucket in body.get("byPath", {}).items():
        print(metric(f"path.{path}.totalMs", bucket["totalMs"]))
        for stage, values in bucket.get("stages", {}).items():
            print("  " + metric(stage, values))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
