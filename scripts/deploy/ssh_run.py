"""
Tiny paramiko-based SSH runner for the deploy flow.

Usage:
    python scripts/deploy/ssh_run.py <command>           # run on remote
    python scripts/deploy/ssh_run.py @<file>             # run a script file

Reads SSH_HOST / SSH_USER / SSH_PASS from env.
Streams stdout+stderr live to local stdout. Exits with the remote exit code.
"""

from __future__ import annotations

import os
import sys
import time

import paramiko


def main() -> int:
    host = os.environ["SSH_HOST"]
    user = os.environ["SSH_USER"]
    password = os.environ["SSH_PASS"]
    port = int(os.environ.get("SSH_PORT", "22"))

    if len(sys.argv) < 2:
        print("usage: ssh_run.py <command|@file>", file=sys.stderr)
        return 2

    arg = sys.argv[1]
    if arg.startswith("@"):
        with open(arg[1:], "r", encoding="utf-8") as f:
            cmd = f.read()
    else:
        cmd = " ".join(sys.argv[1:])

    client = paramiko.SSHClient()
    client.set_missing_host_key_policy(paramiko.AutoAddPolicy())
    client.connect(
        hostname=host,
        port=port,
        username=user,
        password=password,
        look_for_keys=False,
        allow_agent=False,
        timeout=30,
        banner_timeout=30,
        auth_timeout=30,
    )
    try:
        transport = client.get_transport()
        assert transport is not None
        chan = transport.open_session()
        chan.get_pty()
        chan.exec_command(cmd)
        # stream stdout/stderr
        while True:
            if chan.recv_ready():
                data = chan.recv(65536)
                if data:
                    sys.stdout.write(data.decode("utf-8", errors="replace"))
                    sys.stdout.flush()
            if chan.recv_stderr_ready():
                data = chan.recv_stderr(65536)
                if data:
                    sys.stderr.write(data.decode("utf-8", errors="replace"))
                    sys.stderr.flush()
            if chan.exit_status_ready() and not chan.recv_ready() and not chan.recv_stderr_ready():
                break
            time.sleep(0.05)
        # drain
        while chan.recv_ready():
            sys.stdout.write(chan.recv(65536).decode("utf-8", errors="replace"))
        while chan.recv_stderr_ready():
            sys.stderr.write(chan.recv_stderr(65536).decode("utf-8", errors="replace"))
        rc = chan.recv_exit_status()
        return rc
    finally:
        client.close()


if __name__ == "__main__":
    sys.exit(main())
