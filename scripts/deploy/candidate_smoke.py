#!/usr/bin/env python3
"""Run a release candidate against an isolated MongoDB database.

The parent process starts a transient systemd service with outbound networking
blocked.  The child loads the deployment ``.env`` itself and *then* applies
the isolation overrides.  This ordering is intentional: systemd's
``EnvironmentFile=`` can override values supplied with ``Environment=``, which
can silently point a smoke process back at the production database.
"""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import re
import shlex
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.parse
import urllib.request


SYSTEM_DATABASES = {"admin", "config", "local"}
DATABASE_RE = re.compile(r"^[A-Za-z0-9_-]+$")
DEFAULT_PRODUCTION_DATABASE = "wechatagent"
QUEUE_COLLECTIONS = (
    "agent_tasks",
    "import_jobs",
    "agent_send_outbox",
    "knowledge_chat_tasks",
    "catalog_rebuild_jobs",
)

ISOLATION_OVERRIDES = {
    "APP_ENV": "production",
    "APP_HOST": "127.0.0.1",
    "STRATEGIC_PLANNER_ENABLED": "false",
    "COLD_CONTACT_WORKER_ENABLED": "false",
    "SILENCE_SIGNAL_WORKER_ENABLED": "false",
    "EVOLUTION_ENABLED": "false",
    "EVOLUTION_AUTO_RELEASE_ENABLED": "false",
    "KNOWLEDGE_DIGEST_ENABLED": "false",
    "KNOWLEDGE_TASK_WORKER_INTERVAL_SECONDS": "0",
    "CATALOG_REBUILD_WORKER_INTERVAL_SECONDS": "0",
    "KNOWLEDGE_FEEDBACK_INTERVAL_SECONDS": "0",
    "INGEST_WORKER_ENABLED": "false",
    # The always-on task/import/outbox workers have no global off switch.  The
    # caller must use an empty queue; loopback-only networking is the final
    # guard against an accidental external send.
    "TASK_WORKER_INTERVAL_SECONDS": "86400",
    "IMPORT_WORKER_INTERVAL_SECONDS": "86400",
    "MCP_BASE_URL": "http://127.0.0.1:9",
    "OPENAI_BASE_URL": "http://127.0.0.1:9",
}


class SmokeConfigError(ValueError):
    """A smoke launch would not be isolated."""


def parse_dotenv(path: Path) -> dict[str, str]:
    """Parse the conservative KEY=VALUE subset used by the deployment file."""

    values: dict[str, str] = {}
    for line_number, raw in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        if line.startswith("export "):
            line = line[7:].lstrip()
        if "=" not in line:
            raise SmokeConfigError(f"unsupported .env line {line_number}")
        key, value = line.split("=", 1)
        key = key.strip()
        if not re.fullmatch(r"[A-Za-z_][A-Za-z0-9_]*", key):
            raise SmokeConfigError(f"invalid .env key on line {line_number}")
        value = value.strip()
        if value.startswith(("'", '"')):
            try:
                parsed = shlex.split(value, comments=True, posix=True)
            except ValueError as error:
                raise SmokeConfigError(f"invalid quoted .env value on line {line_number}") from error
            if len(parsed) != 1:
                raise SmokeConfigError(f"unsupported quoted .env value on line {line_number}")
            value = parsed[0]
        else:
            value = re.split(r"\s+#", value, maxsplit=1)[0].rstrip()
        values[key] = value
    return values


def validate_database(database: str, production_database: str) -> None:
    if not DATABASE_RE.fullmatch(database):
        raise SmokeConfigError("database must contain only letters, digits, '_' or '-'")
    if database in SYSTEM_DATABASES:
        raise SmokeConfigError(f"refusing system database {database}")
    normalized = database.casefold()
    if normalized in {
        DEFAULT_PRODUCTION_DATABASE.casefold(),
        production_database.casefold(),
    }:
        raise SmokeConfigError(f"refusing production database {database}")
    if not normalized.startswith("wechatagent_"):
        raise SmokeConfigError("smoke database must use the wechatagent_ prefix")


def assert_queues_empty(base: dict[str, str], database: str) -> None:
    """Fail closed unless every worker-owned queue collection is empty.

    The Mongo URI and database are passed through the child environment, never
    argv.  Counting all rows is deliberately stricter than counting claimable
    statuses: stale-running recovery is itself a startup side effect.
    """

    if not base.get("MONGODB_URI"):
        raise SmokeConfigError("MONGODB_URI is required for queue preflight")
    javascript = """
const client = new Mongo(process.env.MONGODB_URI);
const database = process.env.CANDIDATE_SMOKE_DATABASE;
const rawDatabaseNames = client.getDBNames();
const databaseNames = Array.isArray(rawDatabaseNames)
  ? rawDatabaseNames
  : (rawDatabaseNames.databases || []).map(entry =>
      typeof entry === "string" ? entry : entry.name
    );
if (!databaseNames.includes(database)) {
  print(JSON.stringify({error: "database_missing", database}));
  quit(24);
}
const target = client.getDB(database);
const collections = target.getCollectionNames();
if (collections.length === 0 || !collections.includes("migrations")) {
  print(JSON.stringify({error: "database_not_migrated", database, collections: collections.length}));
  quit(24);
}
const migrationCount = target.getCollection("migrations").countDocuments({});
if (migrationCount === 0) {
  print(JSON.stringify({error: "migration_ledger_empty", database}));
  quit(24);
}
const names = JSON.parse(process.env.CANDIDATE_SMOKE_QUEUE_COLLECTIONS);
const counts = Object.fromEntries(names.map(name => [name, target.getCollection(name).countDocuments({})]));
print(JSON.stringify({database, migrationCount, queues: counts}));
if (Object.values(counts).some(count => count !== 0)) { quit(23); }
""".strip()
    env = os.environ.copy()
    env.update(base)
    env["CANDIDATE_SMOKE_DATABASE"] = database
    env["CANDIDATE_SMOKE_QUEUE_COLLECTIONS"] = json.dumps(QUEUE_COLLECTIONS)
    result = subprocess.run(
        ["mongosh", "--quiet", "--eval", javascript],
        check=False,
        text=True,
        capture_output=True,
        env=env,
    )
    if result.returncode != 0:
        summary = result.stdout.strip() or "queue query failed"
        raise SmokeConfigError(f"candidate queue preflight failed: {summary}")


def build_child_environment(
    base: dict[str, str], database: str, port: int, media_dir: Path
) -> dict[str, str]:
    env = os.environ.copy()
    env.update(base)
    env.update(ISOLATION_OVERRIDES)
    # These assignments deliberately happen last.
    env["MONGODB_DATABASE"] = database
    env["APP_PORT"] = str(port)
    env["MEDIA_STORAGE_DIR"] = str(media_dir)
    env["CANDIDATE_SMOKE_ISOLATED"] = "1"
    return env


def stage_runner(source: Path, directory: Path = Path("/run")) -> Path:
    """Copy this runner outside PrivateTmp using a random owner-only path."""

    directory.mkdir(parents=True, exist_ok=True)
    with tempfile.NamedTemporaryFile(
        mode="wb",
        prefix="wechatagent-candidate-smoke-",
        suffix=".py",
        dir=directory,
        delete=False,
    ) as staged:
        staged.write(source.read_bytes())
        staged_path = Path(staged.name)
    staged_path.chmod(0o700)
    return staged_path


def systemd_run_command(args: argparse.Namespace, runner_script: Path) -> list[str]:
    return [
        "systemd-run",
        f"--unit={args.unit}",
        "--no-block",
        "--property=Type=exec",
        f"--property=WorkingDirectory={args.workdir}",
        "--property=IPAddressDeny=any",
        "--property=IPAddressAllow=localhost",
        "--property=NoNewPrivileges=yes",
        "--property=PrivateTmp=yes",
        sys.executable,
        str(runner_script),
        "_inner",
        f"--candidate={args.candidate}",
        f"--database={args.database}",
        f"--env-file={args.env_file}",
        f"--port={args.port}",
        f"--media-dir={args.media_dir}",
    ]


def health_ok(port: int) -> bool:
    try:
        with urllib.request.urlopen(
            f"http://127.0.0.1:{port}/api/health", timeout=2
        ) as response:
            body = json.loads(response.read().decode("utf-8"))
            return response.status == 200 and body.get("ok") is True
    except (OSError, ValueError, urllib.error.URLError):
        return False


def fetch_bytes(port: int, path: str) -> bytes:
    with urllib.request.urlopen(f"http://127.0.0.1:{port}{path}", timeout=2) as response:
        if response.status != 200:
            raise SmokeConfigError(f"candidate static request failed: {path}")
        return response.read()


def assert_static_bundle_served(port: int, workdir: Path) -> int:
    """Require every candidate frontend file to be served byte-for-byte."""

    dist = workdir.resolve() / "frontend" / "dist"
    index = dist / "index.html"
    if not index.is_file():
        raise SmokeConfigError(f"candidate frontend index missing: {index}")
    files = sorted(path for path in dist.rglob("*") if path.is_file())
    if not files:
        raise SmokeConfigError("candidate frontend bundle is empty")
    try:
        if fetch_bytes(port, "/") != index.read_bytes():
            raise SmokeConfigError("candidate root does not serve candidate index.html")
        for path in files:
            relative = path.relative_to(dist).as_posix()
            request_path = "/" + urllib.parse.quote(relative, safe="/")
            if fetch_bytes(port, request_path) != path.read_bytes():
                raise SmokeConfigError(f"candidate static asset mismatch: {relative}")
    except (OSError, urllib.error.URLError) as error:
        raise SmokeConfigError(f"candidate static bundle request failed: {error}") from error
    return len(files)


def unit_state(unit: str) -> tuple[str, str]:
    result = subprocess.run(
        ["systemctl", "show", unit, "-p", "ActiveState", "-p", "SubState", "--value"],
        check=False,
        text=True,
        capture_output=True,
    )
    states = result.stdout.splitlines()
    return (states + ["unknown", "unknown"])[:2]


def run_parent(args: argparse.Namespace) -> int:
    env_path = Path(args.env_file).resolve()
    candidate = Path(args.candidate).resolve()
    media_dir = Path(args.media_dir).resolve()
    if not env_path.is_file():
        raise SmokeConfigError(f"env file not found: {env_path}")
    if not candidate.is_file():
        raise SmokeConfigError(f"candidate not found: {candidate}")
    if args.port in {3003, 8080} or not 1024 <= args.port <= 65535:
        raise SmokeConfigError("smoke port must be non-production and between 1024 and 65535")
    base = parse_dotenv(env_path)
    production_database = base.get("MONGODB_DATABASE", DEFAULT_PRODUCTION_DATABASE)
    validate_database(args.database, production_database)
    assert_queues_empty(base, args.database)
    media_dir.mkdir(parents=True, exist_ok=True)
    workdir = Path(args.workdir).resolve()

    runner_script = stage_runner(Path(__file__).resolve())
    succeeded = False
    started = False
    try:
        command = systemd_run_command(args, runner_script)
        # No .env value is present in this command or in systemd unit properties.
        subprocess.run(command, check=True)
        started = True
        deadline = time.monotonic() + args.timeout
        while time.monotonic() < deadline:
            active, sub = unit_state(args.unit)
            if health_ok(args.port):
                time.sleep(1)
                if health_ok(args.port):
                    static_files = assert_static_bundle_served(args.port, workdir)
                    succeeded = True
                    print(
                        json.dumps(
                            {
                                "ok": True,
                                "database": args.database,
                                "host": "127.0.0.1",
                                "port": args.port,
                                "network": "loopback-only",
                                "staticFiles": static_files,
                            },
                            sort_keys=True,
                        )
                    )
                    return 0
            if active in {"failed", "inactive"} and sub not in {"activating", "running"}:
                break
            time.sleep(0.5)
        subprocess.run(
            ["journalctl", "-u", args.unit, "-n", "120", "--no-pager"], check=False
        )
        return 1
    finally:
        if started:
            subprocess.run(["systemctl", "stop", args.unit], check=False, capture_output=True)
        runner_script.unlink(missing_ok=True)
        if not succeeded:
            print("candidate smoke failed", file=sys.stderr)


def run_inner(args: argparse.Namespace) -> int:
    env_path = Path(args.env_file).resolve()
    base = parse_dotenv(env_path)
    production_database = base.get("MONGODB_DATABASE", DEFAULT_PRODUCTION_DATABASE)
    validate_database(args.database, production_database)
    env = build_child_environment(base, args.database, args.port, Path(args.media_dir).resolve())
    print(
        json.dumps(
            {
                "event": "candidate-smoke-exec",
                "database": env["MONGODB_DATABASE"],
                "host": env["APP_HOST"],
                "port": int(env["APP_PORT"]),
            },
            sort_keys=True,
        ),
        flush=True,
    )
    os.execve(str(Path(args.candidate).resolve()), [args.candidate], env)
    return 127


def parser() -> argparse.ArgumentParser:
    root = argparse.ArgumentParser(description=__doc__)
    sub = root.add_subparsers(dest="mode", required=True)
    for mode in ("run", "_inner"):
        command = sub.add_parser(mode)
        command.add_argument("--candidate", required=True)
        command.add_argument("--database", required=True)
        command.add_argument("--env-file", default="/opt/wechatagent/.env")
        command.add_argument("--port", type=int, default=39083)
        command.add_argument("--media-dir", default="/tmp/wechatagent-candidate-smoke-media")
        if mode == "run":
            command.add_argument("--unit", default="wechatagent-candidate-smoke.service")
            command.add_argument("--workdir", default="/opt/wechatagent")
            command.add_argument("--timeout", type=float, default=45)
    return root


def main() -> int:
    args = parser().parse_args()
    try:
        return run_inner(args) if args.mode == "_inner" else run_parent(args)
    except (SmokeConfigError, subprocess.CalledProcessError) as error:
        print(f"candidate smoke refused: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
