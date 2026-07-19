#!/usr/bin/env python3
"""Fail unless every required SR-128/SR-178 case produced positive evidence."""

import json
import os
import pathlib
import sys


EXPECTED = (
    "k2_relationship_traversal",
    "k3_honest_abstention",
    "k6_vision_artifact",
    "k7_auto_verify_commit",
    "k10_chat_create_proposal",
    "q3_vision_quality",
    "q4_chat_create_quality",
    "t3_vision_artifact",
    "recall_cross_industry",
    "recall_maintenance",
    "recall_gap_closed_loop",
    "redline_cross_domain_full_arc",
    "redline_cross_domain_distinct_behavior",
    "redline_identity_probe",
    "redline_principal_channel",
    "redline_proactive_planner",
    "redline_proactive_wake",
    "redline_dynamic_adversarial",
    "redline_digital_twin_peer",
    "redline_digital_twin_formal",
    "redline_principal_relay",
    "redline_roleplay_arc",
)

SCHEMA = "real_llm_capability_outcome/v1"


def main() -> int:
    root = pathlib.Path(os.environ.get("REAL_LLM_LEDGER", "target/real_llm_ledger"))
    expected_run_id = os.environ.get("GITHUB_RUN_ID", "")
    expected_run_attempt = os.environ.get("GITHUB_RUN_ATTEMPT", "")
    expected_sha = os.environ.get("GITHUB_SHA", "")
    failures = []
    for case_id in EXPECTED:
        matches = list(root.rglob(f"capability_outcome.{case_id}.json"))
        if len(matches) != 1:
            failures.append(f"{case_id}: expected exactly one outcome, found {len(matches)}")
            continue
        try:
            row = json.loads(matches[0].read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError) as exc:
            failures.append(f"{case_id}: unreadable outcome: {exc}")
            continue
        if row.get("schema") != SCHEMA:
            failures.append(
                f"{case_id}: schema={row.get('schema')!r}, expected {SCHEMA!r}"
            )
        if row.get("case_id") != case_id:
            failures.append(f"{case_id}: case_id mismatch")
        if expected_sha and row.get("sha") != expected_sha:
            failures.append(
                f"{case_id}: stale sha={row.get('sha')!r}, expected {expected_sha!r}"
            )
        if expected_run_id and row.get("github_run_id") != expected_run_id:
            failures.append(
                f"{case_id}: stale github_run_id={row.get('github_run_id')!r}, "
                f"expected {expected_run_id!r}"
            )
        if expected_run_attempt and row.get("github_run_attempt") != expected_run_attempt:
            failures.append(
                f"{case_id}: stale github_run_attempt={row.get('github_run_attempt')!r}, "
                f"expected {expected_run_attempt!r}"
            )
        if row.get("verdict") != "pass":
            failures.append(
                f"{case_id}: verdict={row.get('verdict')} reason={row.get('skipped_reason', '')}"
            )
        if row.get("attempted") is not True:
            failures.append(f"{case_id}: attempted is not true")
        if not isinstance(row.get("llm_calls"), int) or row["llm_calls"] <= 0:
            failures.append(f"{case_id}: llm_calls must be > 0")
        if not isinstance(row.get("branch"), str) or not row["branch"].strip():
            failures.append(f"{case_id}: branch must be non-empty")
        if not isinstance(row.get("artifacts"), int) or row["artifacts"] <= 0:
            failures.append(f"{case_id}: artifacts must be > 0")
        if not isinstance(row.get("assertions_run"), int) or row["assertions_run"] <= 0:
            failures.append(f"{case_id}: assertions_run must be > 0")

    print(f"[capability-evidence] checked={len(EXPECTED)} failures={len(failures)}")
    for failure in failures:
        print(f"[capability-evidence] FAIL: {failure}")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
