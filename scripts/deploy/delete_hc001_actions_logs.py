#!/usr/bin/env python3
"""Delete only the GitHub Actions logs confirmed by the HC-001 audit.

The default ``preflight`` mode is read-only.  ``apply`` requires an exact
confirmation phrase and accepts only the complete, immutable scan result from
the 2026-07-30 HC-001 audit.  Workflow runs, artifacts, commits, and branches
are never deleted by this tool.
"""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import sys


REPOSITORY = "jsjm1986/wechatagent"
CONFIRMATION = "HC001-DELETE-69-LEAKED-ACTIONS-LOGS"
EXPECTED_OBJECTS = 807
EXPECTED_HIT_RUNS = 69
EXPECTED_MATCHES = 1795
EXPECTED_ID_SET_SHA256 = (
    "454c6da5160506a46c02d91ff53faac5e486332b1528aed288fc51da7a1c2229"
)


class CleanupError(RuntimeError):
    """The requested log cleanup cannot proceed safely."""


def run_gh(arguments: list[str], *, capture_output: bool = False) -> str:
    try:
        result = subprocess.run(
            ["gh", *arguments],
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE if capture_output else subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            text=True,
            check=False,
            timeout=60,
        )
    except (OSError, subprocess.SubprocessError) as error:
        raise CleanupError("GitHub CLI invocation failed") from error
    if result.returncode != 0:
        raise CleanupError("GitHub CLI refused the operation")
    return result.stdout or ""


def load_confirmed_run_ids(path: Path) -> list[int]:
    try:
        data = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise CleanupError("scan checkpoint is unreadable") from error
    if data.get("schema") != 1 or data.get("kind") != "runs":
        raise CleanupError("scan checkpoint has the wrong identity")
    objects = data.get("objects")
    if not isinstance(objects, dict) or len(objects) != EXPECTED_OBJECTS:
        raise CleanupError("scan checkpoint is not the complete 807-run audit")

    hits: list[int] = []
    matches = 0
    for key, row in objects.items():
        if not isinstance(row, dict) or row.get("status") != "scanned":
            raise CleanupError("scan checkpoint contains an unscanned run")
        try:
            run_id = int(key)
            count = int(row.get("matches", 0))
        except (TypeError, ValueError) as error:
            raise CleanupError("scan checkpoint contains invalid counters") from error
        if run_id <= 0 or count < 0:
            raise CleanupError("scan checkpoint contains invalid counters")
        if count > 0:
            hits.append(run_id)
            matches += count

    hits.sort()
    identity = hashlib.sha256(",".join(map(str, hits)).encode("ascii")).hexdigest()
    if (
        len(hits) != EXPECTED_HIT_RUNS
        or matches != EXPECTED_MATCHES
        or identity != EXPECTED_ID_SET_SHA256
    ):
        raise CleanupError("scan checkpoint does not match the confirmed HC-001 result")
    return hits


def preflight(run_ids: list[int]) -> dict[str, int]:
    run_gh(["auth", "status", "--hostname", "github.com"])
    repository = run_gh(
        ["repo", "view", REPOSITORY, "--json", "nameWithOwner"],
        capture_output=True,
    )
    try:
        owner = json.loads(repository)["nameWithOwner"]
    except (json.JSONDecodeError, KeyError, TypeError) as error:
        raise CleanupError("GitHub repository identity could not be verified") from error
    if owner.casefold() != REPOSITORY.casefold():
        raise CleanupError("GitHub repository identity does not match")

    verified = 0
    for run_id in run_ids:
        output = run_gh(
            [
                "api",
                f"repos/{REPOSITORY}/actions/runs/{run_id}",
                "--jq",
                "{id:.id,name:.name,status:.status}",
            ],
            capture_output=True,
        )
        try:
            metadata = json.loads(output)
        except json.JSONDecodeError as error:
            raise CleanupError("GitHub run metadata could not be verified") from error
        if (
            metadata.get("id") != run_id
            or metadata.get("name") != "CI"
            or metadata.get("status") != "completed"
        ):
            raise CleanupError("a target run no longer matches the audited identity")
        verified += 1
    return {"targetRuns": len(run_ids), "verifiedRuns": verified}


def delete_logs(run_ids: list[int], confirmation: str | None) -> dict[str, int]:
    if confirmation != CONFIRMATION:
        raise CleanupError("apply requires the exact confirmation phrase")
    result = preflight(run_ids)
    deleted = 0
    for run_id in run_ids:
        run_gh(
            [
                "api",
                "--method",
                "DELETE",
                f"repos/{REPOSITORY}/actions/runs/{run_id}/logs",
            ]
        )
        deleted += 1
    return {**result, "deletedLogs": deleted}


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("mode", choices=("preflight", "apply"))
    parser.add_argument("--checkpoint", type=Path, required=True)
    parser.add_argument("--confirm")
    args = parser.parse_args()

    try:
        run_ids = load_confirmed_run_ids(args.checkpoint)
        if args.mode == "preflight":
            result = preflight(run_ids)
            result["deletedLogs"] = 0
        else:
            result = delete_logs(run_ids, args.confirm)
        print(json.dumps(result, sort_keys=True))
        return 0
    except CleanupError as error:
        print(f"HC001_ACTIONS_LOG_CLEANUP_REFUSED={error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
