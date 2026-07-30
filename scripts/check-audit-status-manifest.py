#!/usr/bin/env python3
"""Validate the reproducible 47-domain audit protocol and status ledger (SR-183).

The 2026-06-30 artifact remains useful research material, but its legacy
workflow had no frozen run manifest or per-claim evidence and could silently
drop failed domains.  The status manifest therefore classifies all legacy
records as inconclusive.  A future schema-v2 result may be supplied with
``--result`` and is accepted as complete only when every selected domain has a
fixed slot, both phases completed, and evidence points to repository files.
"""

from __future__ import annotations

import argparse
import copy
import hashlib
import json
import pathlib
import re
import sys
from collections import Counter
from typing import Any


ROOT = pathlib.Path(__file__).resolve().parents[1]
SPEC_DIR = ROOT / ".kiro/specs/universal-test-coverage"
DOMAIN_MANIFEST = SPEC_DIR / "biz-domains-2026-06-30.json"
ANCHORS = SPEC_DIR / "audit-2026-06-30-anchors.json"
LEGACY_RESULT = SPEC_DIR / "deepread-verify-result-2026-06-30.json"
STATUS_MANIFEST = SPEC_DIR / "audit-status-manifest.json"
WORKFLOW = SPEC_DIR / "deepread-verify-workflow.mjs"
ALLOWED_STATUSES = {"complete", "inconclusive", "failed"}
SHA256_RE = re.compile(r"[0-9a-f]{64}")
COMMIT_RE = re.compile(r"[0-9a-f]{40}")


def load_json(path: pathlib.Path, failures: list[str]) -> Any:
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        failures.append(f"cannot read JSON {path.relative_to(ROOT)}: {error}")
        return None


def canonical_hash_bytes(content: bytes) -> bytes:
    """Normalize UTF-8 text line endings before integrity hashing.

    Git may materialize the same text blob as LF or CRLF depending on checkout
    policy.  Audit hashes describe repository content, so those representations
    must be equivalent.  Non-text artifacts remain byte-for-byte hashed.
    """
    try:
        text = content.decode("utf-8")
    except UnicodeDecodeError:
        return content
    return text.replace("\r\n", "\n").replace("\r", "\n").encode("utf-8")


def sha256_bytes(content: bytes) -> str:
    return hashlib.sha256(canonical_hash_bytes(content)).hexdigest()


def sha256(path: pathlib.Path) -> str:
    return sha256_bytes(path.read_bytes())


def require_hash(entry: Any, label: str, failures: list[str]) -> pathlib.Path | None:
    if not isinstance(entry, dict):
        failures.append(f"{label} must be an object")
        return None
    relative = entry.get("path")
    expected = entry.get("sha256")
    if not isinstance(relative, str) or not relative:
        failures.append(f"{label}.path must be a non-empty string")
        return None
    if not isinstance(expected, str) or not SHA256_RE.fullmatch(expected):
        failures.append(f"{label}.sha256 must be a lowercase SHA-256")
        return None
    path = (ROOT / relative).resolve()
    try:
        path.relative_to(ROOT.resolve())
    except ValueError:
        failures.append(f"{label}.path escapes repository: {relative}")
        return None
    if not path.is_file():
        failures.append(f"{label}.path is missing: {relative}")
        return None
    actual = sha256(path)
    if actual != expected:
        failures.append(f"{label} hash drift: expected {expected}, got {actual}")
    return path


def domain_map(domains_doc: Any, failures: list[str]) -> dict[str, dict[str, Any]]:
    domains = domains_doc.get("domains") if isinstance(domains_doc, dict) else None
    if not isinstance(domains, list) or not domains:
        failures.append("domain manifest must contain a non-empty domains array")
        return {}
    result: dict[str, dict[str, Any]] = {}
    for index, domain in enumerate(domains):
        label = f"domains[{index}]"
        if not isinstance(domain, dict):
            failures.append(f"{label} must be an object")
            continue
        domain_id = domain.get("id")
        name = domain.get("name")
        if not isinstance(domain_id, str) or not domain_id:
            failures.append(f"{label}.id must be non-empty")
            continue
        if domain_id in result:
            failures.append(f"duplicate domain id: {domain_id}")
        if not isinstance(name, str) or not name:
            failures.append(f"{label}.name must be non-empty")
        result[domain_id] = domain
    if len(result) != 47:
        failures.append(f"domain manifest must contain exactly 47 unique ids; got {len(result)}")
    return result


def validate_summary(summary: Any, statuses: list[str], label: str, failures: list[str]) -> None:
    if not isinstance(summary, dict):
        failures.append(f"{label} must be an object")
        return
    counts = Counter(statuses)
    expected = {
        "total": len(statuses),
        "complete": counts["complete"],
        "inconclusive": counts["inconclusive"],
        "failed": counts["failed"],
    }
    for key, value in expected.items():
        if summary.get(key) != value:
            failures.append(f"{label}.{key} must equal {value}; got {summary.get(key)!r}")


def validate_evidence(values: Any, label: str, failures: list[str]) -> None:
    if not isinstance(values, list) or not values:
        failures.append(f"{label} must be a non-empty array")
        return
    for index, evidence in enumerate(values):
        item_label = f"{label}[{index}]"
        if not isinstance(evidence, dict) or set(evidence) != {"path", "locator", "claim"}:
            failures.append(f"{item_label} must contain exactly path/locator/claim")
            continue
        if not all(isinstance(evidence.get(key), str) and evidence[key].strip() for key in evidence):
            failures.append(f"{item_label} fields must be non-empty strings")
            continue
        path = (ROOT / evidence["path"]).resolve()
        try:
            path.relative_to(ROOT.resolve())
        except ValueError:
            failures.append(f"{item_label}.path escapes repository")
            continue
        if not path.is_file():
            failures.append(f"{item_label}.path is missing: {evidence['path']}")
            continue
        try:
            content = path.read_text(encoding="utf-8")
        except (OSError, UnicodeDecodeError) as error:
            failures.append(f"{item_label}.path is not readable UTF-8 text: {error}")
            continue
        if evidence["locator"] not in content:
            failures.append(
                f"{item_label}.locator is not present in {evidence['path']}: "
                f"{evidence['locator']!r}"
            )


def validate_phase(
    phase: Any,
    label: str,
    expected_id: str,
    expected_name: str,
    phase_kind: str,
    failures: list[str],
) -> None:
    if not isinstance(phase, dict):
        failures.append(f"{label} must be an object")
        return
    if phase.get("status") != "complete":
        failures.append(f"{label}.status must be complete")
    if not isinstance(phase.get("attempts"), int) or phase["attempts"] < 1:
        failures.append(f"{label}.attempts must be >= 1")
    if not isinstance(phase.get("errors"), list) or not all(
        isinstance(item, str) for item in phase.get("errors", [])
    ):
        failures.append(f"{label}.errors must be an array")
    payload = phase.get("payload")
    if not isinstance(payload, dict):
        failures.append(f"{label}.payload must be an object")
        return
    if payload.get("domain_id") != expected_id or payload.get("domain") != expected_name:
        failures.append(f"{label}.payload domain identity mismatch")
    required = (
        {
            "domain_id",
            "domain",
            "design_behavior",
            "redlines",
            "existing_coverage",
            "test_trust",
            "test_trust_reason",
            "correctness_layer",
            "gaps",
            "suspected_orphans",
            "evidence",
        }
        if phase_kind == "deepread"
        else {
            "domain_id",
            "domain",
            "verified_gaps",
            "refuted",
            "confirmed_orphans",
            "test_priority",
            "verdict",
            "evidence",
        }
    )
    if set(payload) != required:
        failures.append(
            f"{label}.payload keys must exactly match the {phase_kind} schema; "
            f"missing={sorted(required - set(payload))} extra={sorted(set(payload) - required)}"
        )
    if phase_kind == "deepread":
        if payload.get("test_trust") not in {"可信", "假绿", "缺失"}:
            failures.append(f"{label}.payload.test_trust is invalid")
        if payload.get("correctness_layer") not in {
            "红线否定式",
            "设计意图正向",
            "正向质量主观",
            "孤儿行为无定义",
            "混合",
        }:
            failures.append(f"{label}.payload.correctness_layer is invalid")
        string_fields = ("design_behavior", "existing_coverage", "test_trust_reason")
        array_fields = ("redlines", "gaps", "suspected_orphans")
    else:
        if payload.get("test_priority") not in {
            "P0_redline",
            "P1_closed_loop",
            "P2_quality",
            "P3_crud",
        }:
            failures.append(f"{label}.payload.test_priority is invalid")
        string_fields = ("verdict",)
        array_fields = ("verified_gaps", "refuted", "confirmed_orphans")
    for field in string_fields:
        if not isinstance(payload.get(field), str) or not payload[field].strip():
            failures.append(f"{label}.payload.{field} must be non-empty")
    for field in array_fields:
        if not isinstance(payload.get(field), list) or not all(
            isinstance(item, str) for item in payload.get(field, [])
        ):
            failures.append(f"{label}.payload.{field} must be a string array")
    validate_evidence(payload.get("evidence"), f"{label}.payload.evidence", failures)


def validate_v2_document(
    result: Any,
    domains: dict[str, dict[str, Any]],
    failures: list[str],
) -> None:
    if not isinstance(result, dict):
        failures.append("v2 result must be an object")
        return
    if result.get("schemaVersion") != 2:
        failures.append("v2 result schemaVersion must equal 2")
    run = result.get("runManifest")
    if not isinstance(run, dict):
        failures.append("v2 runManifest must be an object")
        return
    for key in ("runId", "model", "startedAt"):
        if not isinstance(run.get(key), str) or not run[key].strip():
            failures.append(f"v2 runManifest.{key} must be non-empty")
    if not isinstance(run.get("sourceCommit"), str) or not COMMIT_RE.fullmatch(run["sourceCommit"]):
        failures.append("v2 runManifest.sourceCommit must be a 40-char commit")
    inputs = run.get("inputs")
    if not isinstance(inputs, dict):
        failures.append("v2 runManifest.inputs must be an object")
    else:
        expected_inputs = {
            "domainManifest": DOMAIN_MANIFEST,
            "anchors": ANCHORS,
            "workflow": WORKFLOW,
        }
        for key, expected_path in expected_inputs.items():
            entry = inputs.get(key)
            checked = require_hash(entry, f"v2 runManifest.inputs.{key}", failures)
            if checked is not None and checked != expected_path.resolve():
                failures.append(f"v2 input {key} must reference {expected_path.relative_to(ROOT)}")

    selected = run.get("selectedDomainIds")
    if not isinstance(selected, list) or not selected or not all(isinstance(item, str) for item in selected):
        failures.append("v2 selectedDomainIds must be a non-empty string array")
        selected = []
    if len(selected) != len(set(selected)):
        failures.append("v2 selectedDomainIds contains duplicates")
    unknown = sorted(set(selected) - domains.keys())
    if unknown:
        failures.append("v2 selectedDomainIds contains unknown ids: " + ", ".join(unknown))

    findings = result.get("findings")
    if not isinstance(findings, list):
        failures.append("v2 findings must be an array")
        findings = []
    seen: list[str] = []
    statuses: list[str] = []
    for index, finding in enumerate(findings):
        label = f"v2 findings[{index}]"
        if not isinstance(finding, dict):
            failures.append(f"{label} must be an object")
            continue
        domain_id = finding.get("domainId")
        status = finding.get("status")
        seen.append(domain_id if isinstance(domain_id, str) else "")
        statuses.append(status if isinstance(status, str) else "")
        if domain_id not in domains:
            failures.append(f"{label}.domainId is unknown: {domain_id!r}")
            continue
        expected_name = domains[domain_id].get("name")
        if finding.get("domain") != expected_name:
            failures.append(f"{label}.domain name mismatch")
        if status not in ALLOWED_STATUSES:
            failures.append(f"{label}.status is invalid: {status!r}")
        if status == "complete":
            phases = finding.get("phases")
            if not isinstance(phases, dict):
                failures.append(f"{label}.phases must be an object")
            else:
                validate_phase(
                    phases.get("deepread"),
                    f"{label}.phases.deepread",
                    domain_id,
                    expected_name,
                    "deepread",
                    failures,
                )
                validate_phase(
                    phases.get("falsify"),
                    f"{label}.phases.falsify",
                    domain_id,
                    expected_name,
                    "falsify",
                    failures,
                )
        elif not isinstance(finding.get("reason"), str) or not finding["reason"].strip():
            failures.append(f"{label} non-complete status requires reason")

    if seen != selected:
        failures.append("v2 finding slots must exactly match selectedDomainIds order")
    duplicates = [item for item, count in Counter(seen).items() if item and count > 1]
    if duplicates:
        failures.append("v2 findings contain duplicate domains: " + ", ".join(sorted(duplicates)))
    validate_summary(result.get("summary"), statuses, "v2 summary", failures)


def validate_v2_result(
    path: pathlib.Path,
    domains: dict[str, dict[str, Any]],
    failures: list[str],
) -> None:
    result = load_json(path, failures)
    if result is not None:
        validate_v2_document(result, domains, failures)


def validate_workflow(domains_doc: Any, failures: list[str]) -> None:
    try:
        text = WORKFLOW.read_text(encoding="utf-8")
    except OSError as error:
        failures.append(f"cannot read workflow: {error}")
        return
    required_fragments = [
        "additionalProperties: false",
        "type: 'array', minItems: 1",
        "audit_run_manifest_invalid",
        "status: 'inconclusive'",
        "status: 'failed'",
        "status: 'complete'",
        "schemaVersion: 2",
        "selectedDomainIds",
    ]
    for fragment in required_fragments:
        if fragment not in text:
            failures.append(f"workflow is missing protocol fragment: {fragment}")
    if "filter(Boolean)" in text:
        failures.append("workflow must not silently drop domain slots with filter(Boolean)")
    embedded_match = re.search(
        r"const ALL_DOMAINS = (\[.*?\])\s*const BG\s*=",
        text,
        re.DOTALL,
    )
    if not embedded_match:
        failures.append("workflow embedded ALL_DOMAINS JSON was not found")
    else:
        try:
            embedded = json.loads(embedded_match.group(1))
        except json.JSONDecodeError as error:
            failures.append(f"workflow embedded ALL_DOMAINS is invalid JSON: {error}")
        else:
            expected = domains_doc.get("domains") if isinstance(domains_doc, dict) else None
            if embedded != expected:
                failures.append("workflow embedded ALL_DOMAINS drifted from domain manifest")


def validate_status_manifest(
    manifest: Any,
    domains: dict[str, dict[str, Any]],
    failures: list[str],
) -> None:
    if not isinstance(manifest, dict):
        failures.append("status manifest must be an object")
        return
    if manifest.get("schemaVersion") != 1:
        failures.append("status manifest schemaVersion must equal 1")
    if manifest.get("auditId") != "universal-test-coverage-47-domains":
        failures.append("status manifest auditId is invalid")
    if manifest.get("authority") != "status_only":
        failures.append("status manifest authority must be status_only")
    legacy_entry = manifest.get("legacyArtifact")
    checked_legacy = require_hash(legacy_entry, "legacyArtifact", failures)
    if checked_legacy is not None and checked_legacy != LEGACY_RESULT.resolve():
        failures.append("legacyArtifact must reference the frozen 2026-06-30 result")
    if isinstance(legacy_entry, dict):
        if legacy_entry.get("classification") != "legacy_inconclusive":
            failures.append("legacyArtifact.classification must be legacy_inconclusive")
        if not isinstance(legacy_entry.get("evidenceCommit"), str) or not COMMIT_RE.fullmatch(
            legacy_entry["evidenceCommit"]
        ):
            failures.append("legacyArtifact.evidenceCommit must be a 40-char commit")
        limitations = legacy_entry.get("limitations")
        if not isinstance(limitations, list) or not all(
            isinstance(item, str) and item for item in limitations
        ):
            failures.append("legacyArtifact.limitations must be a non-empty string array")

    frozen = manifest.get("frozenInputs")
    if not isinstance(frozen, list) or len(frozen) != 2:
        failures.append("frozenInputs must contain domain manifest and anchors")
    else:
        checked = [require_hash(item, f"frozenInputs[{i}]", failures) for i, item in enumerate(frozen)]
        if {path for path in checked if path} != {DOMAIN_MANIFEST.resolve(), ANCHORS.resolve()}:
            failures.append("frozenInputs must exactly reference domain manifest and anchors")

    records = manifest.get("records")
    if not isinstance(records, list):
        failures.append("status manifest records must be an array")
        records = []
    seen: list[str] = []
    statuses: list[str] = []
    for index, record in enumerate(records):
        label = f"records[{index}]"
        if not isinstance(record, dict):
            failures.append(f"{label} must be an object")
            continue
        domain_id = record.get("domainId")
        status = record.get("status")
        seen.append(domain_id if isinstance(domain_id, str) else "")
        statuses.append(status if isinstance(status, str) else "")
        if domain_id not in domains:
            failures.append(f"{label}.domainId is unknown: {domain_id!r}")
            continue
        if record.get("domain") != domains[domain_id].get("name"):
            failures.append(f"{label}.domain name mismatch")
        if status != "inconclusive":
            failures.append(f"{label}.status must remain inconclusive for legacy evidence")
        if record.get("reason") != "legacy_result_missing_v2_run_manifest_and_claim_evidence":
            failures.append(f"{label}.reason must describe the legacy v2 evidence gap")
    if seen != list(domains):
        failures.append("status records must exactly cover all 47 domain ids in manifest order")
    if len(seen) != len(set(seen)):
        failures.append("status records contain duplicate domain ids")
    validate_summary(manifest.get("summary"), statuses, "status summary", failures)
    if Counter(statuses) != Counter({"inconclusive": 47}):
        failures.append("legacy status distribution must remain 47 inconclusive")


def run_negative_self_tests(
    manifest: Any,
    domains: dict[str, dict[str, Any]],
) -> tuple[int, list[str]]:
    cases: list[tuple[str, Any, str]] = []
    missing = copy.deepcopy(manifest)
    missing["records"] = missing.get("records", [])[:-1]
    cases.append(("missing_domain", missing, "exactly cover all 47"))

    forged = copy.deepcopy(manifest)
    forged["records"][0]["status"] = "complete"
    forged["summary"] = {"total": 47, "complete": 1, "inconclusive": 46, "failed": 0}
    cases.append(("forged_complete", forged, "must remain inconclusive"))

    drifted = copy.deepcopy(manifest)
    drifted["frozenInputs"][0]["sha256"] = "0" * 64
    cases.append(("hash_drift", drifted, "hash drift"))

    failures: list[str] = []
    passed = 0
    for name, mutated, expected in cases:
        case_failures: list[str] = []
        validate_status_manifest(mutated, domains, case_failures)
        if any(expected in failure for failure in case_failures):
            passed += 1
        else:
            failures.append(
                f"negative self-test {name} did not detect {expected!r}; "
                f"failures={case_failures}"
            )
    return passed, failures


def run_v2_protocol_self_tests(
    domains: dict[str, dict[str, Any]],
) -> tuple[int, list[str]]:
    evidence = [{
        "path": "src/webhooks.rs",
        "locator": "pub async fn wechat_webhook(",
        "claim": "The production webhook handler is the A1 entrypoint.",
    }]
    phase_base = {"status": "complete", "attempts": 1, "errors": []}
    deepread = {
        **phase_base,
        "payload": {
            "domain_id": "A1",
            "domain": domains["A1"]["name"],
            "design_behavior": "Webhook requests enter the scoped production handler.",
            "redlines": ["unknown app ids fail closed"],
            "existing_coverage": "The handler is covered by scoped integration tests.",
            "test_trust": "可信",
            "test_trust_reason": "The evidence points to the production entrypoint.",
            "correctness_layer": "混合",
            "gaps": [],
            "suspected_orphans": [],
            "evidence": evidence,
        },
    }
    falsify = {
        **phase_base,
        "payload": {
            "domain_id": "A1",
            "domain": domains["A1"]["name"],
            "verified_gaps": [],
            "refuted": [],
            "confirmed_orphans": [],
            "test_priority": "P0_redline",
            "verdict": "The production entrypoint is directly locatable.",
            "evidence": evidence,
        },
    }
    result = {
        "schemaVersion": 2,
        "runManifest": {
            "runId": "checker-self-test",
            "sourceCommit": "0" * 40,
            "model": "checker-fixture",
            "startedAt": "2026-07-24T00:00:00Z",
            "inputs": {
                "domainManifest": {
                    "path": str(DOMAIN_MANIFEST.relative_to(ROOT)).replace("\\", "/"),
                    "sha256": sha256(DOMAIN_MANIFEST),
                },
                "anchors": {
                    "path": str(ANCHORS.relative_to(ROOT)).replace("\\", "/"),
                    "sha256": sha256(ANCHORS),
                },
                "workflow": {
                    "path": str(WORKFLOW.relative_to(ROOT)).replace("\\", "/"),
                    "sha256": sha256(WORKFLOW),
                },
            },
            "selectedDomainIds": ["A1"],
        },
        "summary": {"total": 1, "complete": 1, "inconclusive": 0, "failed": 0},
        "findings": [{
            "domainId": "A1",
            "domain": domains["A1"]["name"],
            "entry": domains["A1"]["entry"],
            "newish": bool(domains["A1"].get("newish")),
            "status": "complete",
            "reason": None,
            "phases": {"deepread": deepread, "falsify": falsify},
        }],
    }

    failures: list[str] = []
    passed = 0
    valid_failures: list[str] = []
    validate_v2_document(result, domains, valid_failures)
    if valid_failures:
        failures.append(f"v2 positive self-test failed: {valid_failures}")
    else:
        passed += 1

    invalid = copy.deepcopy(result)
    invalid["findings"][0]["phases"]["falsify"]["payload"]["evidence"][0][
        "locator"
    ] = "__missing_audit_locator__"
    invalid_failures: list[str] = []
    validate_v2_document(invalid, domains, invalid_failures)
    if any("locator is not present" in failure for failure in invalid_failures):
        passed += 1
    else:
        failures.append(
            "v2 negative self-test did not reject a missing evidence locator; "
            f"failures={invalid_failures}"
        )
    return passed, failures


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--result", type=pathlib.Path, help="optional schema-v2 audit result")
    args = parser.parse_args()
    failures: list[str] = []

    lf_digest = sha256_bytes(b"audit\ninput\n")
    if lf_digest != sha256_bytes(b"audit\r\ninput\r\n") or lf_digest != sha256_bytes(
        b"audit\rinput\r"
    ):
        failures.append("canonical text hashing must treat LF, CRLF, and CR as equivalent")

    domains_doc = load_json(DOMAIN_MANIFEST, failures)
    domains = domain_map(domains_doc, failures)
    manifest = load_json(STATUS_MANIFEST, failures)
    legacy = load_json(LEGACY_RESULT, failures)
    validate_workflow(domains_doc, failures)
    validate_status_manifest(manifest, domains, failures)
    negative_passed, negative_failures = run_negative_self_tests(manifest, domains)
    failures.extend(negative_failures)
    v2_passed, v2_failures = run_v2_protocol_self_tests(domains)
    failures.extend(v2_failures)

    if isinstance(legacy, dict):
        if legacy.get("total") != 47 or not isinstance(legacy.get("findings"), list) or len(legacy["findings"]) != 47:
            failures.append("legacy artifact shape drifted from frozen 47-domain result")

    if args.result:
        result_path = args.result if args.result.is_absolute() else ROOT / args.result
        validate_v2_result(result_path.resolve(), domains, failures)

    complete = 0
    inconclusive = 0
    failed = 0
    if isinstance(manifest, dict) and isinstance(manifest.get("summary"), dict):
        complete = manifest["summary"].get("complete", 0)
        inconclusive = manifest["summary"].get("inconclusive", 0)
        failed = manifest["summary"].get("failed", 0)
    print(
        f"[audit-status] domains={len(domains)} complete={complete} "
        f"inconclusive={inconclusive} failed={failed} "
        f"negative={negative_passed}/3 v2={v2_passed}/2 failures={len(failures)}"
    )
    for failure in failures:
        print(f"[audit-status] FAIL: {failure}")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
