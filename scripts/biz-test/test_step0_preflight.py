import importlib.util
import sys
import unittest
from pathlib import Path
from unittest import mock

HERE = Path(__file__).parent
MODULE_PATH = HERE / "step0_preflight.py"
SPEC = importlib.util.spec_from_file_location("biztest_preflight", MODULE_PATH)
assert SPEC is not None and SPEC.loader is not None
preflight = importlib.util.module_from_spec(SPEC)
with mock.patch.dict(sys.modules, {"_lib": mock.Mock(BIZ_PREFIX="biztest_")}):
    SPEC.loader.exec_module(preflight)


class PrincipalSafetyTests(unittest.TestCase):
    def test_rejects_real_decider_in_camel_and_legacy_shapes(self) -> None:
        rows = [
            {"ask_human_policy": {"deciderChain": [{"wxid": "wxid_real"}]}},
            {"askHumanPolicy": {"principalDecider": "wxid_legacy"}},
        ]
        self.assertEqual(
            preflight.unsafe_principal_targets(rows),
            ["wxid_legacy", "wxid_real"],
        )

    def test_allows_only_test_namespaced_deciders(self) -> None:
        rows = [{"ask_human_policy": {"decider_chain": [{"wxid": "biztest_leader"}]}}]
        self.assertEqual(preflight.unsafe_principal_targets(rows), [])

    def test_unreadable_policy_fails_closed(self) -> None:
        self.assertTrue(preflight.unsafe_principal_targets({"unexpected": True}))


if __name__ == "__main__":
    unittest.main()
