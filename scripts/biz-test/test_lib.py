import hashlib
import hmac
import importlib.util
import unittest
from pathlib import Path
from unittest import mock

MODULE_PATH = Path(__file__).with_name("_lib.py")
SPEC = importlib.util.spec_from_file_location("biztest_lib", MODULE_PATH)
assert SPEC is not None and SPEC.loader is not None
lib = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(lib)


class WebhookSigningTests(unittest.TestCase):
    def test_signs_timestamp_dot_raw_body(self) -> None:
        body = b'{"appId":"app","content":"hello"}'
        expected = hmac.new(b"secret", b"1700000000000." + body, hashlib.sha256).hexdigest()
        self.assertEqual(lib.sign_webhook_body("secret", "1700000000000", body), expected)

    def test_rejects_non_test_sender_before_remote_io(self) -> None:
        with mock.patch.object(lib, "_webhook_secret") as secret:
            with self.assertRaisesRegex(ValueError, "biztest_"):
                lib.send_webhook("app", "real_user", "hello", "biztest_message")
            secret.assert_not_called()

    def test_mongo_failure_is_not_reported_as_cleanup_success(self) -> None:
        with mock.patch.object(lib, "remote_run", return_value=(2, "mongo unavailable")):
            with self.assertRaisesRegex(RuntimeError, "exit code 2"):
                lib.mongo("print(1)")


if __name__ == "__main__":
    unittest.main()
