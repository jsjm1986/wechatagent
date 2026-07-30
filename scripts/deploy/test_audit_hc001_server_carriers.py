from __future__ import annotations

import importlib.util
import io
import json
import os
from pathlib import Path
import stat
import subprocess
import tarfile
import tempfile
import unittest
from unittest import mock


MODULE_PATH = Path(__file__).with_name("audit_hc001_server_carriers.py")
SPEC = importlib.util.spec_from_file_location("audit_hc001_server_carriers", MODULE_PATH)
assert SPEC and SPEC.loader
audit = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(audit)


class AuditHc001ServerCarriersTests(unittest.TestCase):
    def private_env(self, directory: str, value: str) -> Path:
        path = Path(directory) / ".env"
        path.write_text(f"OPENAI_API_KEY='{value}'\n", encoding="utf-8")
        path.chmod(0o600)
        return path

    def test_key_source_requires_private_regular_file(self) -> None:
        if os.name != "posix":
            self.skipTest("POSIX permission semantics are verified on Linux")
        key = "synthetic-server-audit-key-1234567890"
        with tempfile.TemporaryDirectory() as directory:
            path = self.private_env(directory, key)
            self.assertEqual(audit.read_exposed_key(path), key.encode())
            path.chmod(0o640)
            with self.assertRaises(audit.AuditError):
                audit.read_exposed_key(path)

    def test_stream_match_detects_value_across_chunk_boundary(self) -> None:
        needle = b"synthetic-boundary-secret"
        prefix = b"x" * (1024 * 1024 - 7)
        size, matches = audit.count_stream(io.BytesIO(prefix + needle), needle)
        self.assertEqual(size, len(prefix) + len(needle))
        self.assertEqual(matches, 1)

    def test_ordinary_scan_excludes_archives_and_git_directory(self) -> None:
        needle = b"synthetic-ordinary-secret-12345"
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "plain.env").write_bytes(needle)
            (root / "ignored.gz").write_bytes(needle)
            (root / ".git").mkdir()
            (root / ".git" / "object").write_bytes(needle)
            result = audit.scan_ordinary_files((root,), needle)
        self.assertEqual(result["hitFiles"], 1)
        self.assertEqual(result["matches"], 1)
        self.assertEqual(result["hitPaths"], [str(root / "plain.env")])

    def test_archive_scan_reports_tar_and_gzip_without_extracting(self) -> None:
        needle = b"synthetic-archive-secret-12345"
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            tar_path = root / "source.tar.gz"
            content = b"prefix-" + needle
            info = tarfile.TarInfo(".env")
            info.size = len(content)
            with tarfile.open(tar_path, "w:gz") as archive:
                archive.addfile(info, io.BytesIO(content))
            gzip_path = root / "database.archive.gz"
            import gzip

            with gzip.open(gzip_path, "wb") as output:
                output.write(needle + b"-payload")

            tar_result = audit.scan_archive(tar_path, needle)
            gzip_result = audit.scan_archive(gzip_path, needle)
        self.assertEqual(tar_result["status"], "scanned")
        self.assertEqual(tar_result["kind"], "tar")
        self.assertEqual(tar_result["matches"], 1)
        self.assertEqual(gzip_result["kind"], "gzip_stream")
        self.assertEqual(gzip_result["matches"], 1)

    def test_archive_limit_failure_is_explicit_and_not_a_zero_scan(self) -> None:
        needle = b"synthetic-limit-secret-12345"
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "large.gz"
            import gzip

            with gzip.open(path, "wb") as output:
                output.write(needle + b"x" * 32)
            with mock.patch.object(audit, "MAX_ARCHIVE_STREAM_BYTES", 16):
                result = audit.scan_archive(path, needle)
        self.assertEqual(result["status"], "error")
        self.assertEqual(result["matches"], 0)

    def test_git_secret_stays_in_process_and_is_not_in_argv(self) -> None:
        needle = b"synthetic-git-secret-123456"
        listing = subprocess.CompletedProcess([], 0, stdout=b"abc blob 4\n")
        process = mock.MagicMock()
        process.stdin = io.BytesIO()
        process.stdout = io.BytesIO(b"abc blob 4\ndata\n")
        process.returncode = 0
        process.wait.return_value = 0
        with mock.patch.object(audit.subprocess, "run", return_value=listing) as run, mock.patch.object(
            audit.subprocess, "Popen", return_value=process
        ) as popen:
            result = audit.scan_git_objects(Path("/private/repo"), needle)
        rendered = repr(run.call_args_list) + repr(popen.call_args_list)
        self.assertNotIn(needle.decode(), rendered)
        self.assertEqual(result["matches"], 0)

    def test_blockers_distinguish_hits_incomplete_scans_and_health(self) -> None:
        result = {
            "ordinaryFiles": {"errors": 1, "hitFiles": 2},
            "git": {"hitBlobs": 3},
            "archives": {"errors": 1, "hitArchives": 4},
            "service": {"active": True, "healthy": False},
        }
        self.assertEqual(
            audit.blockers_for(result),
            [
                "ordinary_file_scan_incomplete",
                "ordinary_file_copies_present",
                "git_object_copies_present",
                "archive_scan_incomplete",
                "archive_copies_present",
                "production_service_not_healthy",
            ],
        )

    def test_source_has_no_mutation_mode_or_delete_primitive(self) -> None:
        source = MODULE_PATH.read_text(encoding="utf-8")
        self.assertNotIn('choices=("preflight", "apply")', source)
        self.assertNotIn("unlink(", source)
        self.assertNotIn("remove(", source)
        self.assertNotIn("rmtree(", source)


if __name__ == "__main__":
    unittest.main()
