#!/usr/bin/env python3
"""Rotate the leaked default LLM credential without putting it in argv/logs.

The default ``preflight`` mode is read-only. ``apply`` requires an explicit
confirmation phrase, an owner-only key file, empty production queues, and a
successful protocol-correct probe. The environment file and Mongo references
are restored automatically when restart, health, or verification fails.

This tool does not revoke the upstream credential or delete historical copies.
Those actions happen only after the new production configuration is verified.
"""

from __future__ import annotations

import argparse
from contextlib import contextmanager
import hashlib
import json
import os
from pathlib import Path
import re
import signal
import stat
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.parse
import urllib.request


DEFAULT_ENV_FILE = Path("/opt/wechatagent/.env")
DEFAULT_UNIT = "wechatagent"
DEFAULT_HEALTH_URL = "http://127.0.0.1:3003/api/health"
DEFAULT_LOCK_FILE = Path("/run/wechatagent-hc001-credential-rotation.lock")
CONFIRMATION = "HC001-ROTATE-LEAKED-LLM-CREDENTIAL"
KEY_NAME = "OPENAI_API_KEY"
BASE_URL_NAME = "OPENAI_BASE_URL"
MODEL_NAME = "OPENAI_MODEL"
SAFE_KEY = re.compile(r"^[A-Za-z0-9._~+/=-]+$")
SAFE_MODEL = re.compile(r"^[A-Za-z0-9._:/-]{1,160}$")


class RotationError(RuntimeError):
    """The rotation cannot proceed safely."""


def require_posix_host() -> None:
    """Reject hosts that cannot enforce owner-only files and advisory locks."""

    if os.name != "posix" or not hasattr(os, "geteuid"):
        raise RotationError("credential rotation requires a POSIX host")


def parse_dotenv(path: Path) -> dict[str, str]:
    values: dict[str, str] = {}
    for number, raw in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        if line.startswith("export "):
            line = line[7:].lstrip()
        if "=" not in line:
            raise RotationError(f"unsupported env line {number}")
        key, value = line.split("=", 1)
        value = value.strip()
        if len(value) >= 2 and value[0] == value[-1] and value[0] in "\"'":
            value = value[1:-1]
        values[key.strip()] = value
    return values


def read_new_key(path: Path) -> str:
    if path.is_symlink() or not path.is_file():
        raise RotationError("new key path must be a regular non-symlink file")
    metadata = path.stat()
    mode = stat.S_IMODE(metadata.st_mode)
    if mode & 0o077:
        raise RotationError("new key file must not grant group/other permissions")
    if hasattr(os, "geteuid") and metadata.st_uid != os.geteuid():
        raise RotationError("new key file must be owned by the invoking user")
    value = path.read_text(encoding="utf-8").strip()
    if len(value) < 20 or not SAFE_KEY.fullmatch(value):
        raise RotationError("new key has an unsupported shape")
    return value


def replace_env_value(text: str, name: str, value: str) -> str:
    if "'" in value or "\n" in value or "\r" in value:
        raise RotationError(f"{name} cannot be represented safely in the env file")
    pattern = re.compile(
        rf"(?m)^(?P<prefix>\s*(?:export\s+)?{re.escape(name)}\s*=).*$"
    )
    matches = list(pattern.finditer(text))
    if len(matches) != 1:
        raise RotationError(f"env must contain exactly one {name} assignment")
    return pattern.sub(
        lambda match: match.group("prefix") + "'" + value + "'", text
    )


def replace_env_config(
    original: bytes, new_key: str, new_base_url: str, new_model: str
) -> bytes:
    text = original.decode("utf-8")
    for name, value in (
        (KEY_NAME, new_key),
        (BASE_URL_NAME, new_base_url),
        (MODEL_NAME, new_model),
    ):
        text = replace_env_value(text, name, value)
    return text.encode("utf-8")


def replace_env_key(original: bytes, new_key: str) -> bytes:
    """Backward-compatible key-only helper used by existing callers/tests."""

    text = original.decode("utf-8")
    return replace_env_value(text, KEY_NAME, new_key).encode("utf-8")


def candidate_config(base_url: str, model: str, protocol: str) -> dict[str, str]:
    parsed = urllib.parse.urlsplit(base_url)
    if (
        parsed.scheme != "https"
        or not parsed.hostname
        or parsed.username is not None
        or parsed.password is not None
        or parsed.query
        or parsed.fragment
    ):
        raise RotationError("candidate base URL must be a credential-free HTTPS URL")
    normalized_url = base_url.rstrip("/")
    if not SAFE_MODEL.fullmatch(model):
        raise RotationError("candidate model has an unsupported shape")
    normalized_protocol = protocol_name(protocol)
    return {
        "baseUrl": normalized_url,
        "model": model,
        "format": "openai" if normalized_protocol == "chat" else "anthropic",
    }


def atomic_write(path: Path, content: bytes) -> None:
    metadata = path.stat()
    descriptor, temporary = tempfile.mkstemp(prefix=".hc001-env-", dir=path.parent)
    try:
        os.fchmod(descriptor, stat.S_IMODE(metadata.st_mode))
        if hasattr(os, "fchown"):
            os.fchown(descriptor, metadata.st_uid, metadata.st_gid)
        with os.fdopen(descriptor, "wb") as output:
            output.write(content)
            output.flush()
            os.fsync(output.fileno())
        os.replace(temporary, path)
    except BaseException:
        try:
            os.close(descriptor)
        except OSError:
            pass
        Path(temporary).unlink(missing_ok=True)
        raise


def run(command: list[str], *, env: dict[str, str] | None = None) -> subprocess.CompletedProcess[str]:
    try:
        return subprocess.run(command, check=True, text=True, capture_output=True, env=env)
    except (OSError, subprocess.SubprocessError) as error:
        executable = Path(command[0]).name
        raise RotationError(f"{executable} command failed") from error


@contextmanager
def operation_lock(path: Path = DEFAULT_LOCK_FILE):
    """Prevent concurrent preflight/apply processes without logging secrets."""

    try:
        import fcntl
    except ImportError as error:  # pragma: no cover - production is Linux.
        raise RotationError("credential rotation requires a POSIX host") from error
    descriptor = os.open(path, os.O_CREAT | os.O_RDWR, 0o600)
    try:
        try:
            fcntl.flock(descriptor, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except OSError as error:
            raise RotationError("another credential rotation process is active") from error
        yield
    finally:
        os.close(descriptor)


def mongo_eval(base: dict[str, str], javascript: str, extra: dict[str, str]) -> str:
    environment = os.environ.copy()
    environment.update(extra)
    environment["HC001_DB"] = base.get("MONGODB_DATABASE", "wechatagent")
    environment["MONGODB_URI"] = base.get("MONGODB_URI", "mongodb://127.0.0.1:27017")
    result = run(["mongosh", "--quiet", "--eval", javascript], env=environment)
    return result.stdout.strip()


INVENTORY_JS = r"""
const client = new Mongo(process.env.MONGODB_URI);
const d = client.getDB(process.env.HC001_DB);
const oldKey = process.env.HC001_OLD_KEY;
const newKey = process.env.HC001_NEW_KEY;
const projection = {
  _id: 1, workspaceId: 1, providerId: 1, apiKey: 1,
  format: 1, baseUrl: 1, model: 1, isActive: 1
};
function rows(filter) {
  return d.llm_provider_configs.find(filter, projection).toArray().map(row => ({
    id: row._id.toString(),
    workspaceId: row.workspaceId || "",
    providerId: row.providerId || "",
    apiKey: row.apiKey || "",
    format: row.format || "openai",
    baseUrl: row.baseUrl || "",
    model: row.model || "",
    isActive: row.isActive === true
  }));
}
const oldRows = rows({apiKey: oldKey});
const newRows = rows({apiKey: newKey});
const activeRows = rows({isActive: true});
const result = {
  oldRefs: oldRows.length,
  oldActiveRefs: oldRows.filter(row => row.isActive === true).length,
  newRefs: newRows.length,
  oldRows,
  newRows,
  activeRows
};
print(JSON.stringify(result));
""".strip()


QUEUE_JS = r"""
const client = new Mongo(process.env.MONGODB_URI);
const d = client.getDB(process.env.HC001_DB);
const counts = {
  tasks: d.agent_tasks.countDocuments({status: {$in: ["pending","retry","running","committing"]}}),
  outbox: d.agent_send_outbox.countDocuments({status: {$in: ["pending","in_flight","claimed","sending"]}}),
  imports: d.import_jobs.countDocuments({status: {$in: ["pending","running"]}}),
  knowledge: d.knowledge_chat_tasks.countDocuments({status: {$in: ["pending","running"]}}),
  catalog: d.catalog_rebuild_jobs.countDocuments({status: {$in: ["queued","processing"]}})
};
print(JSON.stringify(counts));
""".strip()


APPLY_PROVIDER_PLAN_JS = r"""
const client = new Mongo(process.env.MONGODB_URI);
const session = client.startSession();
let output;
try {
  session.startTransaction();
  const d = session.getDatabase(process.env.HC001_DB);
  const plan = JSON.parse(process.env.HC001_PROVIDER_PLAN);
  const candidate = JSON.parse(process.env.HC001_CANDIDATE);
  const restore = process.env.HC001_RESTORE === "1";
  let modified = 0;
  for (const row of plan) {
    const before = restore ? {
      apiKey: candidate.apiKey,
      format: candidate.format,
      baseUrl: candidate.baseUrl,
      model: candidate.model,
      isActive: row.isActive
    } : {
      apiKey: row.apiKey,
      format: row.format,
      baseUrl: row.baseUrl,
      model: row.model,
      isActive: row.isActive
    };
    const after = restore ? {
      apiKey: row.apiKey,
      format: row.format,
      baseUrl: row.baseUrl,
      model: row.model
    } : {
      apiKey: candidate.apiKey,
      format: candidate.format,
      baseUrl: candidate.baseUrl,
      model: candidate.model
    };
    const result = d.llm_provider_configs.updateOne(
      {_id: ObjectId(row.id), ...before},
      {$set: after}
    );
    if (result.matchedCount !== 1) {
      throw new Error("provider plan changed");
    }
    modified += result.modifiedCount;
  }
  session.commitTransaction();
  output = {matched: plan.length, modified};
} catch (error) {
  try { session.abortTransaction(); } catch (_) {}
  throw error;
} finally {
  session.endSession();
}
print(JSON.stringify(output));
""".strip()


def inventory(base: dict[str, str], old_key: str, new_key: str) -> dict[str, object]:
    output = mongo_eval(base, INVENTORY_JS, {"HC001_OLD_KEY": old_key, "HC001_NEW_KEY": new_key})
    return json.loads(output)


def queue_counts(base: dict[str, str]) -> dict[str, int]:
    return json.loads(mongo_eval(base, QUEUE_JS, {}))


def require_empty_queues(base: dict[str, str]) -> dict[str, int]:
    counts = queue_counts(base)
    if any(value != 0 for value in counts.values()):
        raise RotationError("production queues are not empty")
    return counts


def apply_provider_plan(
    base: dict[str, str],
    plan: list[dict[str, object]],
    candidate: dict[str, str],
    *,
    restore: bool = False,
) -> dict[str, int]:
    output = mongo_eval(
        base,
        APPLY_PROVIDER_PLAN_JS,
        {
            "HC001_PROVIDER_PLAN": json.dumps(plan, separators=(",", ":")),
            "HC001_CANDIDATE": json.dumps(candidate, separators=(",", ":")),
            "HC001_RESTORE": "1" if restore else "0",
        },
    )
    return json.loads(output)


def build_rotation_plan(state: dict[str, object]) -> list[dict[str, object]]:
    old_rows = list(state["oldRows"])
    active_rows = list(state["activeRows"])
    if not old_rows:
        raise RotationError("no provider records reference the exposed key")
    if len(active_rows) != 1:
        raise RotationError("rotation requires exactly one active provider record")
    rows: dict[str, dict[str, object]] = {}
    for row in old_rows + active_rows:
        rows[str(row["id"])] = dict(row)
    return [rows[identifier] for identifier in sorted(rows)]


def provider_state_matches(
    state: dict[str, object],
    plan: list[dict[str, object]],
    candidate: dict[str, str],
    *,
    migrated: bool,
) -> bool:
    rows: dict[str, dict[str, object]] = {}
    for group in ("oldRows", "newRows", "activeRows"):
        for row in state[group]:
            rows[str(row["id"])] = row
    for expected in plan:
        actual = rows.get(str(expected["id"]))
        if actual is None or bool(actual["isActive"]) != bool(expected["isActive"]):
            return False
        source = candidate if migrated else expected
        for name in ("apiKey", "format", "baseUrl", "model"):
            if actual[name] != source[name]:
                return False
    return True


def protocol_name(value: str) -> str:
    normalized = value.strip().lower()
    if normalized in {"", "chat", "openai"}:
        return "chat"
    if normalized in {"messages", "anthropic", "claude"}:
        return "messages"
    raise RotationError("affected provider uses an unsupported protocol")


def probe(base_url: str, model: str, protocol: str, key: str, timeout: int) -> str:
    if not base_url or not model:
        return "not_probeable"
    if protocol_name(protocol) == "messages":
        url = base_url.rstrip("/") + "/v1/messages"
        body = {
            "model": model,
            "max_tokens": 8,
            "temperature": 0,
            "system": "Credential validation only.",
            "messages": [{"role": "user", "content": "Reply with exactly OK"}],
        }
        headers = {
            "x-api-key": key,
            "anthropic-version": "2023-06-01",
            "content-type": "application/json",
        }
    else:
        url = base_url.rstrip("/") + "/chat/completions"
        body = {
            "model": model,
            "max_tokens": 8,
            "temperature": 0,
            "messages": [
                {"role": "system", "content": "Credential validation only."},
                {"role": "user", "content": "Reply with exactly OK"},
            ],
        }
        headers = {"authorization": "Bearer " + key, "content-type": "application/json"}
    request = urllib.request.Request(
        url,
        data=json.dumps(body).encode("utf-8"),
        method="POST",
        headers={**headers, "user-agent": "wechatagent-hc001-rotation"},
    )
    try:
        with urllib.request.urlopen(request, timeout=timeout) as response:
            response.read(4096)
            status_code = response.status
    except urllib.error.HTTPError as error:
        status_code = error.code
    except Exception as error:  # Network/TLS details can contain endpoints; report only the class.
        return "transport_" + type(error).__name__.lower()
    if 200 <= status_code < 300:
        return "accepted"
    if status_code in {401, 403}:
        return "rejected_auth"
    if status_code == 429:
        return "accepted_rate_limited"
    return "http_" + str(status_code)


def probe_targets(
    rows: list[dict[str, object]], base: dict[str, str], key: str, timeout: int
) -> tuple[list[dict[str, object]], list[dict[str, object]]]:
    targets = list(rows)
    targets.append(
        {
            "format": "chat",
            "baseUrl": base.get("OPENAI_BASE_URL", ""),
            "model": base.get("OPENAI_MODEL", ""),
            "active": False,
        }
    )
    unique: list[dict[str, object]] = []
    results: list[dict[str, object]] = []
    seen: set[tuple[str, str, str, bool]] = set()
    for row in targets:
        identity = (
            str(row.get("format", "chat")),
            str(row.get("baseUrl", "")),
            str(row.get("model", "")),
            bool(row.get("active", False)),
        )
        if identity in seen:
            continue
        seen.add(identity)
        target = {
            "format": identity[0],
            "baseUrl": identity[1],
            "model": identity[2],
            "active": identity[3],
        }
        unique.append(target)
        results.append(
            {
                "active": identity[3],
                "result": probe(identity[1], identity[2], identity[0], key, timeout),
            }
        )
    return unique, results


def require_usable_probe_results(results: list[dict[str, object]]) -> None:
    if any(item["result"] == "rejected_auth" for item in results):
        raise RotationError("new key was rejected by an affected endpoint")
    if not any(item["result"] == "accepted" for item in results):
        raise RotationError("new key did not complete any affected endpoint probe")
    if any(item["active"] and item["result"] != "accepted" for item in results):
        raise RotationError("an active affected provider did not pass its probe")


def health_ok(url: str) -> bool:
    try:
        with urllib.request.urlopen(url, timeout=5) as response:
            body = json.loads(response.read().decode("utf-8"))
            return response.status == 200 and body.get("ok") is True
    except (OSError, ValueError, urllib.error.URLError):
        return False


def wait_for_health(url: str, timeout: int) -> None:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if health_ok(url):
            time.sleep(2)
            if health_ok(url):
                return
        time.sleep(1)
    raise RotationError("service health did not stabilize")


def file_sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def service_state(unit: str) -> dict[str, object]:
    output = run(
        [
            "systemctl",
            "show",
            unit,
            "-p",
            "MainPID",
            "-p",
            "NRestarts",
            "-p",
            "ActiveState",
            "-p",
            "SubState",
        ]
    ).stdout
    values = dict(line.split("=", 1) for line in output.splitlines() if "=" in line)
    pid = int(values.get("MainPID", "0"))
    if pid <= 0:
        raise RotationError("service has no running process")
    executable = Path(f"/proc/{pid}/exe").resolve(strict=True)
    return {
        "active": values.get("ActiveState") == "active",
        "running": values.get("SubState") == "running",
        "restarts": int(values.get("NRestarts", "0")),
        "exeSha256": file_sha256(executable),
    }


def preflight_details(
    base: dict[str, str],
    new_key: str,
    candidate: dict[str, str],
    timeout: int,
    health_url: str,
) -> tuple[dict[str, object], list[dict[str, object]]]:
    old_key = base.get(KEY_NAME, "")
    if len(old_key) < 20:
        raise RotationError("existing OPENAI_API_KEY is missing or malformed")
    if old_key == new_key:
        raise RotationError("new key must differ from the exposed key")
    if not health_ok(health_url):
        raise RotationError("production health check is not green")
    queues = require_empty_queues(base)
    state = inventory(base, old_key, new_key)
    if int(state["newRefs"]) != 0:
        raise RotationError("new key already appears in provider records")
    plan = build_rotation_plan(state)
    probes = [
        {
            "active": True,
            "result": probe(
                candidate["baseUrl"],
                candidate["model"],
                candidate["format"],
                new_key,
                timeout,
            ),
        }
    ]
    require_usable_probe_results(probes)
    return {
        "oldProviderRefs": int(state["oldRefs"]),
        "oldActiveProviderRefs": int(state["oldActiveRefs"]),
        "targetProviderRecords": len(plan),
        "probeResults": [item["result"] for item in probes],
        "queues": queues,
    }, plan


def preflight(
    base: dict[str, str],
    new_key: str,
    candidate: dict[str, str],
    timeout: int,
    health_url: str,
) -> dict[str, object]:
    return preflight_details(base, new_key, candidate, timeout, health_url)[0]


def secure_runtime_backup(original: bytes) -> Path:
    descriptor, name = tempfile.mkstemp(prefix="hc001-env-rollback-", dir="/run")
    os.fchmod(descriptor, 0o600)
    with os.fdopen(descriptor, "wb") as output:
        output.write(original)
        output.flush()
        os.fsync(output.fileno())
    return Path(name)


def systemctl(action: str, unit: str) -> None:
    run(["systemctl", action, unit])


def apply_rotation(
    args: argparse.Namespace,
    base: dict[str, str],
    new_key: str,
    candidate: dict[str, str],
) -> dict[str, object]:
    if args.confirm != CONFIRMATION:
        raise RotationError("apply requires the exact confirmation phrase")
    if hasattr(os, "geteuid") and os.geteuid() != 0:
        raise RotationError("apply must run as root")
    env_metadata = args.env_file.stat()
    if args.env_file.is_symlink() or stat.S_IMODE(env_metadata.st_mode) & 0o077:
        raise RotationError("production env file must be a private regular file")
    preflight_result, plan = preflight_details(
        base, new_key, candidate, args.probe_timeout, args.health_url
    )
    runtime_before = service_state(args.unit)
    if not runtime_before["active"] or not runtime_before["running"]:
        raise RotationError("production service is not active/running")
    old_key = base[KEY_NAME]
    original = args.env_file.read_bytes()
    replacement = replace_env_config(
        original, new_key, candidate["baseUrl"], candidate["model"]
    )
    rollback_path = secure_runtime_backup(original)
    provider_count = int(preflight_result["targetProviderRecords"])
    candidate_with_key = {**candidate, "apiKey": new_key}
    stopped = False
    try:
        systemctl("stop", args.unit)
        stopped = True
        require_empty_queues(base)
        atomic_write(args.env_file, replacement)
        apply_provider_plan(base, plan, candidate_with_key)
        systemctl("start", args.unit)
        stopped = False
        wait_for_health(args.health_url, args.health_timeout)
        updated_base = parse_dotenv(args.env_file)
        verified = inventory(updated_base, old_key, new_key)
        if (
            int(verified["oldRefs"]) != 0
            or int(verified["newRefs"]) != provider_count
            or not provider_state_matches(
                verified, plan, candidate_with_key, migrated=True
            )
        ):
            raise RotationError("post-rotation provider verification failed")
        post_probe_results = [
            {
                "active": True,
                "result": probe(
                    candidate["baseUrl"],
                    candidate["model"],
                    candidate["format"],
                    new_key,
                    args.probe_timeout,
                ),
            }
        ]
        require_usable_probe_results(post_probe_results)
        require_empty_queues(updated_base)
        runtime_after = service_state(args.unit)
        if (
            not runtime_after["active"]
            or not runtime_after["running"]
            or runtime_after["exeSha256"] != runtime_before["exeSha256"]
            or runtime_after["restarts"] != runtime_before["restarts"]
        ):
            raise RotationError("service runtime identity changed during rotation")
        rollback_path.unlink(missing_ok=True)
        return {
            **preflight_result,
            "rotationApplied": True,
            "upstreamRevocationRequired": True,
            "historicalCopyCleanupRequired": True,
        }
    except BaseException as original_error:
        try:
            if not stopped:
                systemctl("stop", args.unit)
                stopped = True
            atomic_write(args.env_file, original)
            actual = inventory(base, old_key, new_key)
            if provider_state_matches(
                actual, plan, candidate_with_key, migrated=True
            ):
                apply_provider_plan(
                    base, plan, candidate_with_key, restore=True
                )
            elif not provider_state_matches(
                actual, plan, candidate_with_key, migrated=False
            ):
                raise RotationError("provider references are in an ambiguous rollback state")
            systemctl("start", args.unit)
            stopped = False
            wait_for_health(args.health_url, args.health_timeout)
            rollback_path.unlink(missing_ok=True)
        except BaseException as rollback_error:
            raise RotationError(
                f"rotation failed and automatic rollback failed; recover from {rollback_path}"
            ) from rollback_error
        raise RotationError("rotation failed; previous configuration was restored") from original_error


def write_evidence(directory: Path, result: dict[str, object]) -> None:
    directory.mkdir(mode=0o700, parents=True, exist_ok=False)
    path = directory / "result.json"
    path.write_text(json.dumps(result, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    path.chmod(0o600)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("mode", choices=("preflight", "apply"))
    parser.add_argument("--new-key-file", type=Path, required=True)
    parser.add_argument("--env-file", type=Path, default=DEFAULT_ENV_FILE)
    parser.add_argument("--unit", default=DEFAULT_UNIT)
    parser.add_argument("--health-url", default=DEFAULT_HEALTH_URL)
    parser.add_argument("--health-timeout", type=int, default=45)
    parser.add_argument("--probe-timeout", type=int, default=30)
    parser.add_argument("--new-base-url")
    parser.add_argument("--new-model")
    parser.add_argument("--new-format", default="openai")
    parser.add_argument("--confirm")
    parser.add_argument("--evidence-dir", type=Path)
    args = parser.parse_args()

    def interrupted(_signum: int, _frame: object) -> None:
        raise RotationError("rotation interrupted")

    signal.signal(signal.SIGTERM, interrupted)
    try:
        require_posix_host()
        with operation_lock():
            new_key = read_new_key(args.new_key_file)
            base = parse_dotenv(args.env_file)
            candidate = candidate_config(
                args.new_base_url or base.get(BASE_URL_NAME, ""),
                args.new_model or base.get(MODEL_NAME, ""),
                args.new_format,
            )
            if args.mode == "preflight":
                result = preflight(
                    base,
                    new_key,
                    candidate,
                    args.probe_timeout,
                    args.health_url,
                )
                result["rotationApplied"] = False
            else:
                result = apply_rotation(args, base, new_key, candidate)
        if args.evidence_dir:
            write_evidence(args.evidence_dir, result)
        print(json.dumps(result, sort_keys=True))
        return 0
    except RotationError as error:
        print(f"HC001_ROTATION_REFUSED={error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
