#!/usr/bin/env python3
"""Fail CI when a production outbound send bypasses the durable Dispatcher.

This intentionally checks a small, explicit call graph instead of relying on naming
conventions. The low-level MCP send logger may only be called by the Dispatcher and
its three audited delivery helpers. Those helpers may only be invoked by the
Dispatcher. Any new occurrence requires an explicit protocol review and this
allowlist to change in the same diff.
"""

from __future__ import annotations

import pathlib
import re
import sys
from collections import Counter

ROOT = pathlib.Path(__file__).resolve().parents[1]
SRC = ROOT / "src"


def rust_files() -> list[pathlib.Path]:
    return sorted(SRC.rglob("*.rs"))


def occurrence_counts(pattern: str) -> Counter[str]:
    regex = re.compile(pattern)
    counts: Counter[str] = Counter()
    for path in rust_files():
        count = len(regex.findall(path.read_text(encoding="utf-8")))
        if count:
            counts[path.relative_to(ROOT).as_posix()] = count
    return counts


def check_exact(label: str, actual: Counter[str], expected: dict[str, int]) -> list[str]:
    expected_counter = Counter(expected)
    failures: list[str] = []
    for path in sorted(set(actual) | set(expected_counter)):
        got = actual[path]
        want = expected_counter[path]
        if got != want:
            failures.append(f"{label}: {path} has {got} occurrence(s), expected {want}")
    return failures


def evaluate(counts: dict[str, Counter[str]]) -> list[str]:
    failures: list[str] = []
    failures += check_exact(
        "low-level MCP send definition",
        counts["low_level_definition"],
        {"src/mcp.rs": 1},
    )
    failures += check_exact(
        "low-level MCP send callers",
        counts["low_level_calls"],
        {
            "src/agent/outbox_dispatcher.rs": 1,  # internal notifications
            "src/agent/gateway.rs": 1,  # text delivery helper body
            "src/agent/media_send.rs": 1,  # media delivery helper body
            "src/agent/referral.rs": 1,  # namecard delivery helper body
        },
    )
    failures += check_exact(
        "text delivery helper",
        counts["text_helper"],
        {
            "src/agent/gateway.rs": 1,  # definition
            "src/agent/outbox_dispatcher.rs": 1,  # sole caller
        },
    )
    failures += check_exact(
        "media delivery helper",
        counts["media_helper"],
        {
            "src/agent/media_send.rs": 1,  # definition
            "src/agent/outbox_dispatcher.rs": 1,  # sole caller
        },
    )
    failures += check_exact(
        "namecard delivery helper",
        counts["namecard_helper"],
        {
            "src/agent/referral.rs": 1,  # definition
            "src/agent/outbox_dispatcher.rs": 1,  # sole caller
        },
    )
    return failures


def self_test() -> int:
    valid = {
        "low_level_definition": Counter({"src/mcp.rs": 1}),
        "low_level_calls": Counter(
            {
                "src/agent/outbox_dispatcher.rs": 1,
                "src/agent/gateway.rs": 1,
                "src/agent/media_send.rs": 1,
                "src/agent/referral.rs": 1,
            }
        ),
        "text_helper": Counter(
            {"src/agent/gateway.rs": 1, "src/agent/outbox_dispatcher.rs": 1}
        ),
        "media_helper": Counter(
            {"src/agent/media_send.rs": 1, "src/agent/outbox_dispatcher.rs": 1}
        ),
        "namecard_helper": Counter(
            {"src/agent/referral.rs": 1, "src/agent/outbox_dispatcher.rs": 1}
        ),
    }
    if evaluate(valid):
        print("[delivery-boundary] self-test failed: valid graph was rejected")
        return 1
    invalid = {key: Counter(value) for key, value in valid.items()}
    invalid["low_level_calls"]["src/agent/escalation/mod.rs"] = 1
    invalid["text_helper"]["src/routes/management.rs"] = 1
    failures = evaluate(invalid)
    if len(failures) != 2:
        print(f"[delivery-boundary] self-test failed: expected 2 violations, got {failures}")
        return 1
    print("[delivery-boundary] self-test passed")
    return 0


def main() -> int:
    if sys.argv[1:] == ["--self-test"]:
        return self_test()
    if sys.argv[1:]:
        print("usage: check-delivery-boundary.py [--self-test]")
        return 2
    counts = {
        "low_level_definition": occurrence_counts(
            r"\bfn\s+logged_send_call_for_account\s*<"
        ),
        "low_level_calls": occurrence_counts(
            r"(?<!fn )\blogged_send_call_for_account\s*\("
        ),
        "text_helper": occurrence_counts(r"\bsend_outbound_message\s*\("),
        "media_helper": occurrence_counts(r"\bsend_outbound_media\s*\("),
        "namecard_helper": occurrence_counts(r"\bsend_outbound_namecard\s*\("),
    }
    failures = evaluate(counts)
    if failures:
        for failure in failures:
            print(f"[delivery-boundary] FAIL: {failure}")
        return 1
    print("[delivery-boundary] ok: all outbound MCP sends remain Dispatcher-owned")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
