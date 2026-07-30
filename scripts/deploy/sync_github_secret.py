#!/usr/bin/env python3
"""Safely synchronize one GitHub Actions repository secret from stdin.

The secret is never accepted as an argument or environment variable.  Apply
mode requires an explicit confirmation phrase and forwards the value to
``gh secret set`` over stdin while discarding all child-process output.
"""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys


DEFAULT_SECRET_NAME = "RSXERMU_KEY"
CONFIRMATION = "HC001-SYNC-GITHUB-RSXERMU-KEY"
REPOSITORY_RE = re.compile(r"^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$")
SECRET_NAME_RE = re.compile(r"^[A-Z][A-Z0-9_]*$")


class SyncError(RuntimeError):
    """The requested synchronization cannot be performed safely."""


def validate_target(repository: str, secret_name: str) -> None:
    if not REPOSITORY_RE.fullmatch(repository):
        raise SyncError("repository must use the OWNER/REPO form")
    if not SECRET_NAME_RE.fullmatch(secret_name):
        raise SyncError("secret name must use uppercase GitHub Actions syntax")


def run_gh(
    arguments: list[str],
    *,
    input_value: str | None = None,
    capture_output: bool = False,
) -> subprocess.CompletedProcess[str]:
    stdin_options: dict[str, object]
    if input_value is None:
        stdin_options = {"stdin": subprocess.DEVNULL}
    else:
        stdin_options = {"input": input_value}
    try:
        result = subprocess.run(
            ["gh", *arguments],
            text=True,
            stdout=subprocess.PIPE if capture_output else subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            check=False,
            timeout=45,
            **stdin_options,
        )
    except (OSError, subprocess.SubprocessError) as error:
        raise SyncError("GitHub CLI invocation failed") from error
    if result.returncode != 0:
        raise SyncError("GitHub CLI refused the operation")
    return result


def preflight(repository: str) -> None:
    run_gh(["auth", "status", "--hostname", "github.com"])
    run_gh(["repo", "view", repository, "--json", "nameWithOwner"])


def read_secret_from_stdin() -> str:
    if sys.stdin.isatty():
        raise SyncError("apply requires a non-interactive stdin source")
    value = sys.stdin.read()
    if value.endswith("\r\n"):
        value = value[:-2]
    elif value.endswith("\n"):
        value = value[:-1]
    if not value or len(value) < 20:
        raise SyncError("stdin did not contain a plausible secret")
    if "\n" in value or "\r" in value or "\x00" in value:
        raise SyncError("stdin must contain exactly one secret value")
    if value != value.strip():
        raise SyncError("secret must not have leading or trailing whitespace")
    return value


def apply(repository: str, secret_name: str, confirmation: str | None) -> None:
    if confirmation != CONFIRMATION:
        raise SyncError("apply requires the exact confirmation phrase")
    preflight(repository)
    value = read_secret_from_stdin()
    run_gh(
        ["secret", "set", secret_name, "--repo", repository, "--app", "actions"],
        input_value=value,
    )
    listed = run_gh(
        ["secret", "list", "--repo", repository, "--app", "actions", "--json", "name"],
        capture_output=True,
    )
    try:
        names = {item["name"] for item in json.loads(listed.stdout or "[]")}
    except (json.JSONDecodeError, KeyError, TypeError) as error:
        raise SyncError("GitHub secret metadata verification failed") from error
    if secret_name not in names:
        raise SyncError("GitHub did not report the synchronized secret")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("mode", choices=("preflight", "apply"))
    parser.add_argument("--repo", required=True, help="target repository as OWNER/REPO")
    parser.add_argument("--secret-name", default=DEFAULT_SECRET_NAME)
    parser.add_argument("--confirm")
    args = parser.parse_args()

    try:
        validate_target(args.repo, args.secret_name)
        if args.mode == "preflight":
            preflight(args.repo)
            print("HC001_GITHUB_SECRET_PREFLIGHT=ok")
        else:
            apply(args.repo, args.secret_name, args.confirm)
            print("HC001_GITHUB_SECRET_SYNC=ok")
        return 0
    except SyncError as error:
        print(f"HC001_GITHUB_SECRET_SYNC_REFUSED={error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
