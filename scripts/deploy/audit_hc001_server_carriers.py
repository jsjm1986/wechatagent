#!/usr/bin/env python3
"""Read-only audit of server-side carriers for the exposed HC-001 key.

The key is read from a private environment file and never accepted through
argv, written to evidence, or printed.  This tool has no mutation mode: it
reports ordinary-file, Git-object, archive, and service-state blockers only.
"""

from __future__ import annotations

import argparse
import gzip
import io
import json
import os
from pathlib import Path
import stat
import subprocess
import sys
import tarfile
import urllib.error
import urllib.request
import zipfile


DEFAULT_ENV_FILE = Path("/opt/wechatagent/.env")
DEFAULT_REPOSITORY = Path("/opt/wechatagent")
DEFAULT_ROOTS = (
    Path("/opt/wechatagent"),
    Path("/opt/wechatagent-backups"),
    Path("/opt/wechatagent-release-20260721-031031"),
)
DEFAULT_UNIT = "wechatagent"
DEFAULT_HEALTH_URL = "http://127.0.0.1:3003/api/health"
ARCHIVE_SUFFIXES = (".tar", ".tar.gz", ".tgz", ".zip", ".gz")
MAX_ARCHIVE_STREAM_BYTES = 4 * 1024 * 1024 * 1024


class AuditError(RuntimeError):
    """The audit cannot produce a complete and trustworthy result."""


def read_exposed_key(path: Path) -> bytes:
    if path.is_symlink() or not path.is_file():
        raise AuditError("environment source must be a regular non-symlink file")
    metadata = path.stat()
    if stat.S_IMODE(metadata.st_mode) & 0o077:
        raise AuditError("environment source must not grant group/other permissions")
    values: list[bytes] = []
    for raw in path.read_bytes().splitlines():
        line = raw.strip()
        if line.startswith(b"export "):
            line = line[7:].lstrip()
        if not line.startswith(b"OPENAI_API_KEY="):
            continue
        value = line.split(b"=", 1)[1].strip()
        if len(value) >= 2 and value[:1] == value[-1:] and value[:1] in {b"'", b'"'}:
            value = value[1:-1]
        values.append(value)
    if len(values) != 1 or len(values[0]) < 20:
        raise AuditError("environment source must contain exactly one plausible key")
    return values[0]


def is_archive(path: Path) -> bool:
    lowered = path.name.lower()
    return lowered.endswith(ARCHIVE_SUFFIXES)


def count_stream(source: io.BufferedReader, needle: bytes, limit: int | None = None) -> tuple[int, int]:
    total = 0
    matches = 0
    overlap = b""
    while True:
        chunk = source.read(1024 * 1024)
        if not chunk:
            break
        total += len(chunk)
        if limit is not None and total > limit:
            raise AuditError("stream exceeds the audit safety limit")
        combined = overlap + chunk
        matches += combined.count(needle)
        overlap = combined[-(len(needle) - 1) :] if len(needle) > 1 else b""
    return total, matches


def walk_files(roots: tuple[Path, ...]) -> list[Path]:
    files: set[Path] = set()
    for root in roots:
        if not root.exists():
            continue
        for directory, names, filenames in os.walk(root):
            names[:] = [
                name
                for name in names
                if name != ".git" and not (Path(directory) / name).is_symlink()
            ]
            for name in filenames:
                path = Path(directory) / name
                if path.is_file() and not path.is_symlink():
                    files.add(path)
    return sorted(files)


def scan_ordinary_files(roots: tuple[Path, ...], needle: bytes) -> dict[str, object]:
    scanned = 0
    scanned_bytes = 0
    matches = 0
    hits: list[str] = []
    errors: list[str] = []
    for path in walk_files(roots):
        if is_archive(path):
            continue
        try:
            with path.open("rb") as source:
                size, count = count_stream(source, needle)
        except OSError:
            errors.append(str(path))
            continue
        scanned += 1
        scanned_bytes += size
        matches += count
        if count:
            hits.append(str(path))
    return {
        "scanned": scanned,
        "bytes": scanned_bytes,
        "hitFiles": len(hits),
        "matches": matches,
        "hitPaths": hits,
        "errors": len(errors),
        "errorPaths": errors,
    }


def scan_git_objects(repository: Path, needle: bytes) -> dict[str, int]:
    listing_process = subprocess.run(
        [
            "git",
            "-C",
            str(repository),
            "cat-file",
            "--batch-all-objects",
            "--batch-check=%(objectname) %(objecttype) %(objectsize)",
        ],
        check=False,
        stdout=subprocess.PIPE,
        stderr=subprocess.DEVNULL,
    )
    if listing_process.returncode != 0:
        raise AuditError("Git object inventory failed")
    listing = listing_process.stdout.splitlines()
    blobs = [
        (fields[0], int(fields[2]))
        for row in listing
        if len(fields := row.split()) == 3 and fields[1] == b"blob"
    ]
    process = subprocess.Popen(
        ["git", "-C", str(repository), "cat-file", "--batch"],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.DEVNULL,
    )
    assert process.stdin is not None and process.stdout is not None
    scanned_bytes = 0
    matches = 0
    hit_blobs = 0
    try:
        for object_name, expected_size in blobs:
            process.stdin.write(object_name + b"\n")
            process.stdin.flush()
            header = process.stdout.readline().split()
            if len(header) != 3 or header[1] != b"blob":
                raise AuditError("unexpected Git object response")
            size = int(header[2])
            if size != expected_size:
                raise AuditError("Git object changed during audit")
            content = process.stdout.read(size)
            terminator = process.stdout.read(1)
            if len(content) != size or terminator != b"\n":
                raise AuditError("Git object stream was truncated")
            count = content.count(needle)
            scanned_bytes += size
            matches += count
            hit_blobs += int(count > 0)
    finally:
        process.stdin.close()
        process.stdout.close()
        process.wait(timeout=30)
    if process.returncode != 0:
        raise AuditError("Git object audit failed")
    return {
        "objects": len(listing),
        "blobs": len(blobs),
        "bytes": scanned_bytes,
        "hitBlobs": hit_blobs,
        "matches": matches,
    }


def scan_archive(path: Path, needle: bytes) -> dict[str, object]:
    members = 0
    extracted_bytes = 0
    matches = 0
    try:
        if tarfile.is_tarfile(path):
            kind = "tar"
            with tarfile.open(path, "r:*") as archive:
                for member in archive:
                    if not member.isfile():
                        continue
                    source = archive.extractfile(member)
                    if source is None:
                        continue
                    with source:
                        size, count = count_stream(source, needle, MAX_ARCHIVE_STREAM_BYTES)
                    members += 1
                    extracted_bytes += size
                    if extracted_bytes > MAX_ARCHIVE_STREAM_BYTES:
                        raise AuditError("archive exceeds the audit safety limit")
                    matches += count
        elif zipfile.is_zipfile(path):
            kind = "zip"
            with zipfile.ZipFile(path) as archive:
                for member in archive.infolist():
                    if member.is_dir():
                        continue
                    with archive.open(member) as source:
                        size, count = count_stream(source, needle, MAX_ARCHIVE_STREAM_BYTES)
                    members += 1
                    extracted_bytes += size
                    if extracted_bytes > MAX_ARCHIVE_STREAM_BYTES:
                        raise AuditError("archive exceeds the audit safety limit")
                    matches += count
        else:
            kind = "gzip_stream"
            with gzip.open(path, "rb") as source:
                extracted_bytes, matches = count_stream(
                    source, needle, MAX_ARCHIVE_STREAM_BYTES
                )
            members = 1
        return {
            "path": str(path),
            "status": "scanned",
            "kind": kind,
            "members": members,
            "bytes": extracted_bytes,
            "matches": matches,
        }
    except (AuditError, OSError, EOFError, tarfile.TarError, zipfile.BadZipFile):
        return {
            "path": str(path),
            "status": "error",
            "kind": "unknown",
            "members": members,
            "bytes": extracted_bytes,
            "matches": 0,
        }


def scan_archives(roots: tuple[Path, ...], needle: bytes) -> dict[str, object]:
    rows = [scan_archive(path, needle) for path in walk_files(roots) if is_archive(path)]
    hits = [row["path"] for row in rows if int(row["matches"]) > 0]
    errors = [row["path"] for row in rows if row["status"] != "scanned"]
    return {
        "candidates": len(rows),
        "scanned": sum(row["status"] == "scanned" for row in rows),
        "hitArchives": len(hits),
        "matches": sum(int(row["matches"]) for row in rows),
        "hitPaths": hits,
        "errors": len(errors),
        "errorPaths": errors,
    }


def service_state(unit: str, health_url: str) -> dict[str, object]:
    process = subprocess.run(
        ["systemctl", "show", unit, "-p", "ActiveState", "-p", "MainPID", "-p", "NRestarts"],
        check=False,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.DEVNULL,
    )
    if process.returncode != 0:
        return {"active": False, "pid": 0, "restarts": -1, "healthy": False}
    values = dict(line.split("=", 1) for line in process.stdout.splitlines() if "=" in line)
    healthy = False
    try:
        with urllib.request.urlopen(health_url, timeout=5) as response:
            body = json.loads(response.read().decode("utf-8"))
            healthy = response.status == 200 and body.get("ok") is True
    except (OSError, ValueError, urllib.error.URLError):
        pass
    return {
        "active": values.get("ActiveState") == "active",
        "pid": int(values.get("MainPID", "0")),
        "restarts": int(values.get("NRestarts", "-1")),
        "healthy": healthy,
    }


def blockers_for(result: dict[str, object]) -> list[str]:
    blockers: list[str] = []
    ordinary = result["ordinaryFiles"]
    git = result["git"]
    archives = result["archives"]
    service = result["service"]
    assert isinstance(ordinary, dict) and isinstance(git, dict)
    assert isinstance(archives, dict) and isinstance(service, dict)
    if ordinary["errors"]:
        blockers.append("ordinary_file_scan_incomplete")
    if ordinary["hitFiles"]:
        blockers.append("ordinary_file_copies_present")
    if git["hitBlobs"]:
        blockers.append("git_object_copies_present")
    if archives["errors"]:
        blockers.append("archive_scan_incomplete")
    if archives["hitArchives"]:
        blockers.append("archive_copies_present")
    if not service["active"] or not service["healthy"]:
        blockers.append("production_service_not_healthy")
    return blockers


def audit(
    env_file: Path,
    repository: Path,
    roots: tuple[Path, ...],
    unit: str,
    health_url: str,
) -> dict[str, object]:
    needle = read_exposed_key(env_file)
    result: dict[str, object] = {
        "schema": 1,
        "mode": "read_only",
        "ordinaryFiles": scan_ordinary_files(roots, needle),
        "git": scan_git_objects(repository, needle),
        "archives": scan_archives(roots, needle),
        "service": service_state(unit, health_url),
    }
    result["blockers"] = blockers_for(result)
    return result


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--env-file", type=Path, default=DEFAULT_ENV_FILE)
    parser.add_argument("--repository", type=Path, default=DEFAULT_REPOSITORY)
    parser.add_argument("--root", type=Path, action="append")
    parser.add_argument("--unit", default=DEFAULT_UNIT)
    parser.add_argument("--health-url", default=DEFAULT_HEALTH_URL)
    args = parser.parse_args()
    roots = tuple(args.root) if args.root else DEFAULT_ROOTS
    try:
        result = audit(args.env_file, args.repository, roots, args.unit, args.health_url)
        print(json.dumps(result, sort_keys=True))
        return 2 if result["blockers"] else 0
    except AuditError as error:
        print(f"HC001_SERVER_CARRIER_AUDIT_REFUSED={error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
