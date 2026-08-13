import importlib.util
import unittest
from pathlib import Path

MODULE_PATH = Path(__file__).with_name("run_all.py")
SPEC = importlib.util.spec_from_file_location("biztest_run_all", MODULE_PATH)
assert SPEC is not None and SPEC.loader is not None
run_all = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(run_all)


class RunAllSafetyTests(unittest.TestCase):
    def test_preflight_failure_still_runs_final_cleanup_and_returns_nonzero(self) -> None:
        calls: list[str] = []

        def fake_run(module: str) -> int:
            calls.append(module)
            return 7 if module == "step0_preflight" else 0

        self.assertEqual(run_all.execute_suite(fake_run), 7)
        self.assertEqual(calls, ["cleanup", "step0_preflight", "cleanup"])

    def test_domain_failure_is_reported_after_remaining_domains_and_cleanup(self) -> None:
        calls: list[str] = []
        failed = run_all.BATCH_A[1]

        def fake_run(module: str) -> int:
            calls.append(module)
            return 3 if module == failed else 0

        self.assertEqual(run_all.execute_suite(fake_run), 3)
        self.assertEqual(calls[0], "cleanup")
        self.assertEqual(calls[-1], "cleanup")
        self.assertIn("batch_b_industry", calls)


    def test_blocked_summary_is_machine_readable(self) -> None:
        import json
        import tempfile
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "blocked.jsonl"
            path.write_text(
                json.dumps({"domain": "d", "capability": "vision"}) + "\n",
                encoding="utf-8",
            )
            summary = run_all.blocked_summary(path)
        self.assertEqual(summary["count"], 1)
        self.assertEqual(summary["items"][0]["capability"], "vision")

    def test_final_cleanup_failure_is_returned(self) -> None:
        calls: list[str] = []

        def fake_run(module: str) -> int:
            calls.append(module)
            return 9 if module == "cleanup" and calls.count("cleanup") == 2 else 0

        self.assertEqual(run_all.execute_suite(fake_run), 9)
        self.assertEqual(calls[-1], "cleanup")


if __name__ == "__main__":
    unittest.main()
