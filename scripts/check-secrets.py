#!/usr/bin/env python3
"""Fail when source candidates contain credentials or unsafe provider bindings.

The checker deliberately reports only path, line and rule. It never prints the
candidate value or a reversible digest. The CI default scans tracked files.
Local audits may add ``--include-untracked``; oversized untracked candidates
then fail closed instead of being silently skipped.
"""

from __future__ import annotations

import argparse
import math
import pathlib
import re
import subprocess
import sys
from collections import Counter
from dataclasses import dataclass


PLACEHOLDER_WORDS = (
    "abc123",
    "changeme",
    "dummy",
    "example",
    "fake",
    "not-a-real",
    "placeholder",
    "redacted",
    "replace-with",
    "sample",
    "synthetic",
    "test-",
)

PUBLIC_TEST_PRIVATE_KEY_FILES = {"tests/fixtures/jwt_test_private.pem"}

# Prefixes with public, recognizable credential formats. Keep the prefix and
# body in separate literals so this checker does not match its own source.
TOKEN_PREFIXES = ("sk", "nvapi", "github_pat", "ghp", "gho", "ghu", "ghs", "xoxb")
TOKEN_PATTERN = re.compile(
    rf"(?i)(?<![a-z0-9])(?:{'|'.join(map(re.escape, TOKEN_PREFIXES))})"
    r"[-_][a-z0-9_-]{16,}"
)
SECRET_NAME = r"(?:api[_-]?key|token|secret|password|passwd|deploy_pass|ssh_pass)"
LITERAL_ASSIGNMENT = re.compile(
    rf"(?i)\b{SECRET_NAME}\b\s*[:=]\s*([\"'])(?P<value>[^\"'\r\n]+)\1"
)
ENV_ASSIGNMENT = re.compile(
    r"^\s*(?:export\s+)?[A-Z_][A-Z0-9_]*"
    r"(?:API[_-]?KEY|TOKEN|SECRET|PASSWORD|PASSWD|DEPLOY_PASS|SSH_PASS)"
    r"\s*=\s*(?P<value>[^#\r\n]+?)\s*$"
)
WORKFLOW_SECRET_ASSIGNMENT = re.compile(
    r"^\s*[A-Z_][A-Z0-9_]*"
    r"(?:API[_-]?KEY|TOKEN|SECRET|PASSWORD|PASSWD|DEPLOY_PASS|SSH_PASS)"
    r"\s*:\s*(?P<value>[^#\r\n]*?)\s*$"
)
DIRECT_GITHUB_SECRET = re.compile(
    r"^\$\{\{\s*secrets\.[A-Za-z_][A-Za-z0-9_]*\s*\}\}$"
)
PRIVATE_URI = re.compile(
    r"(?i)\b(?:mongodb(?:\+srv)?|postgres(?:ql)?|mysql|redis)://"
    r"[^\s:/@]+:(?P<value>[^\s/@]+)@"
)
PRIVATE_KEY_MARKER = "-----BEGIN " + "PRIVATE KEY-----"
MAX_UNTRACKED_BYTES = 5 * 1024 * 1024
FORBIDDEN_TRACKED_PATHS = {".env.e2e"}

# HC-001: this repository secret belongs to one provider configuration.  A
# secret-only rotation is unsafe if a workflow still sends the replacement to
# the former gateway, model, or protocol.  Keep the binding public and exact;
# the credential itself remains in GitHub Actions secrets.
ROTATED_WORKFLOW_SECRET = "secrets.RSXERMU_KEY"
ROTATED_WORKFLOW_BINDINGS = {
    "REAL_LLM_BASE_URL": "https://gateway.oeezzk.cn/v1",
    "REAL_LLM_MODEL": "gpt-5.6-auto",
    "REAL_LLM_FORMAT": "openai",
    "REAL_LLM_JUDGE_BASE_URL": "https://gateway.oeezzk.cn/v1",
    "REAL_LLM_JUDGE_MODEL": "codex-auto-review",
    "REAL_LLM_JUDGE1_MODEL": "codex-auto-review",
    "REAL_LLM_JUDGE_LITE_MODEL": "codex-auto-review",
    "REAL_LLM_JUDGE_FORMAT": "openai",
    "REAL_LLM_VISION_BASE_URL": "https://gateway.oeezzk.cn/v1",
    "REAL_LLM_VISION_MODEL": "gpt-5.6-auto",
}
WORKFLOW_BINDING_ASSIGNMENT = re.compile(
    r"^\s*(?P<name>REAL_LLM_(?:BASE_URL|MODEL|FORMAT|JUDGE_BASE_URL|"
    r"JUDGE_MODEL|JUDGE1_MODEL|JUDGE_LITE_MODEL|JUDGE_FORMAT|"
    r"VISION_BASE_URL|VISION_MODEL))\s*:\s*(?P<value>[^#\r\n]+?)\s*$"
)


@dataclass(frozen=True, order=True)
class Finding:
    path: str
    line: int
    rule: str


def entropy(value: str) -> float:
    counts = Counter(value)
    length = len(value)
    return -sum((count / length) * math.log2(count / length) for count in counts.values())


def strip_quotes(value: str) -> str:
    value = value.strip()
    if len(value) >= 2 and value[0] == value[-1] and value[0] in "\"'":
        return value[1:-1]
    return value


def is_placeholder(value: str) -> bool:
    lowered = value.lower()
    return (
        ("<" in value and ">" in value)
        or any(word in lowered for word in PLACEHOLDER_WORDS)
    )


def is_high_entropy(value: str) -> bool:
    value = strip_quotes(value)
    if is_placeholder(value) or len(value) < 20:
        return False
    if value.startswith(("${", "$env:", "secrets.", "process.env", "std::env")):
        return False
    return entropy(value) >= 3.5


def scan_text(path: str, text: str) -> list[Finding]:
    findings: set[Finding] = set()
    for number, line in enumerate(text.splitlines(), 1):
        if PRIVATE_KEY_MARKER in line and path not in PUBLIC_TEST_PRIVATE_KEY_FILES:
            findings.add(Finding(path, number, "private-key-marker"))

        for match in TOKEN_PATTERN.finditer(line):
            if not is_placeholder(match.group(0)):
                findings.add(Finding(path, number, "credential-prefix"))

        for pattern, rule in (
            (LITERAL_ASSIGNMENT, "literal-secret-assignment"),
            (ENV_ASSIGNMENT, "literal-env-secret"),
            (PRIVATE_URI, "credential-in-uri"),
        ):
            for match in pattern.finditer(line):
                if is_high_entropy(match.group("value")):
                    findings.add(Finding(path, number, rule))

        if path.startswith(".github/workflows/"):
            workflow_assignment = WORKFLOW_SECRET_ASSIGNMENT.match(line)
            if workflow_assignment:
                value = workflow_assignment.group("value").strip()
                if value not in {"", "''", '""'} and not DIRECT_GITHUB_SECRET.fullmatch(value):
                    findings.add(
                        Finding(path, number, "workflow-secret-must-be-direct")
                    )
    if path.startswith(".github/workflows/") and ROTATED_WORKFLOW_SECRET in text:
        for number, line in enumerate(text.splitlines(), 1):
            match = WORKFLOW_BINDING_ASSIGNMENT.match(line)
            if not match:
                continue
            expected = ROTATED_WORKFLOW_BINDINGS[match.group("name")]
            if strip_quotes(match.group("value")) != expected:
                findings.add(Finding(path, number, "rotated-secret-provider-binding"))

    return sorted(findings)


def git_files(*arguments: str) -> list[pathlib.Path]:
    result = subprocess.run(
        ["git", "ls-files", "-z", *arguments],
        check=True,
        stdout=subprocess.PIPE,
    )
    return [pathlib.Path(raw.decode("utf-8")) for raw in result.stdout.split(b"\0") if raw]


def candidate_files(include_untracked: bool) -> tuple[list[pathlib.Path], set[pathlib.Path]]:
    tracked = git_files("--cached")
    untracked = git_files("--others", "--exclude-standard") if include_untracked else []
    return sorted(set(tracked + untracked)), set(untracked)


def scan_repository(include_untracked: bool = False) -> list[Finding]:
    findings: list[Finding] = []
    tracked = git_files("--cached")
    for path in tracked:
        if path.as_posix() in FORBIDDEN_TRACKED_PATHS:
            findings.append(Finding(path.as_posix(), 1, "private-env-must-not-be-tracked"))
    candidates, untracked = candidate_files(include_untracked)
    for path in candidates:
        if not path.is_file():
            continue
        if path in untracked and path.stat().st_size > MAX_UNTRACKED_BYTES:
            findings.append(Finding(path.as_posix(), 1, "oversized-untracked-candidate"))
            continue
        data = path.read_bytes()
        if b"\0" in data:
            continue
        findings.extend(scan_text(path.as_posix(), data.decode("utf-8", errors="replace")))
    return sorted(set(findings))


def self_test() -> None:
    positive_token = "nvapi" + "-" + "A7c9F2m4Q6s8V1x3Z5b7N9k2P4r6T8w1"
    positive_assignment = 'API_KEY="' + "Z9y8X7w6V5u4T3s2R1q0P9o8N7m6L5k4" + '"'
    positive_uri = "mongodb://agent:" + "N8m7B6v5C4x3Z2a1S9d8F7g6" + "@db.invalid/app"
    negative = "\n".join(
        (
            "OPENAI_API_KEY=replace-with-deepseek-key",
            'apiKey: "test-provider-secret"',
            'token: "synthetic-active-update-token"',
            "MCP_API_KEY=<account-mcp-api-key>",
            "REAL_LLM_API_KEY=${{ secrets.REAL_LLM_API_KEY }}",
            "reaction_claim_token = %reaction_claim_token,",
        )
    )
    workflow_positive = (
        "REAL_LLM_FAILOVER_API_KEY: "
        "${{ secrets.BACKUP_KEY || 'synthetic-but-forbidden-fallback-value' }}"
    )
    workflow_negative = "\n".join(
        (
            "REAL_LLM_API_KEY: ${{ secrets.REAL_LLM_API_KEY }}",
            "OPTIONAL_API_KEY: ''",
        )
    )
    binding_positive = "\n".join(
        (
            "REAL_LLM_API_KEY: ${{ secrets.RSXERMU_KEY }}",
            "REAL_LLM_BASE_URL: https://old-gateway.invalid/v1",
            "REAL_LLM_MODEL: old-model",
            "REAL_LLM_FORMAT: messages",
        )
    )
    binding_negative = "\n".join(
        (
            "REAL_LLM_API_KEY: ${{ secrets.RSXERMU_KEY }}",
            "REAL_LLM_BASE_URL: https://gateway.oeezzk.cn/v1",
            "REAL_LLM_MODEL: gpt-5.6-auto",
            "REAL_LLM_FORMAT: openai",
            "REAL_LLM_JUDGE_BASE_URL: https://gateway.oeezzk.cn/v1",
            "REAL_LLM_JUDGE_MODEL: codex-auto-review",
            "REAL_LLM_JUDGE_FORMAT: openai",
            "REAL_LLM_VISION_BASE_URL: https://gateway.oeezzk.cn/v1",
            "REAL_LLM_VISION_MODEL: gpt-5.6-auto",
        )
    )
    assert scan_text("positive-token", positive_token)
    assert scan_text("positive-assignment", positive_assignment)
    assert scan_text("positive-uri", positive_uri)
    assert not scan_text("negative", negative)
    assert scan_text(".github/workflows/positive.yml", workflow_positive) == [
        Finding(
            ".github/workflows/positive.yml",
            1,
            "workflow-secret-must-be-direct",
        )
    ]
    assert not scan_text(".github/workflows/negative.yml", workflow_negative)
    assert {
        finding.rule
        for finding in scan_text(".github/workflows/binding-positive.yml", binding_positive)
    } == {"rotated-secret-provider-binding"}
    assert not scan_text(".github/workflows/binding-negative.yml", binding_negative)
    assert ".env.e2e" in FORBIDDEN_TRACKED_PATHS


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--self-test", action="store_true")
    parser.add_argument("--include-untracked", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        self_test()
        print("[secret-scan] self-test passed")
        return 0

    findings = scan_repository(include_untracked=args.include_untracked)
    scope = "tracked+untracked" if args.include_untracked else "tracked"
    print(f"[secret-scan] scope={scope} findings={len(findings)}")
    for finding in findings:
        print(f"[secret-scan] FAIL {finding.path}:{finding.line} rule={finding.rule}")
    return 1 if findings else 0


if __name__ == "__main__":
    sys.exit(main())
