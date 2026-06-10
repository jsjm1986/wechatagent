#!/usr/bin/env python
"""Run a remote command over SSH using paramiko.

Connection details come from env vars (DEPLOY_HOST/PORT/USER/PASS) so the
password never lands in argv or shell history. Usage:

    DEPLOY_PASS=... python scripts/_remote_run.py "uname -a"

Reads the command from argv[1] (or stdin if argv[1] == '-'). Streams combined
stdout+stderr and exits with the remote command's exit status.
"""
import os
import sys

import paramiko

host = os.environ.get("DEPLOY_HOST", "117.72.54.28")
port = int(os.environ.get("DEPLOY_PORT", "3003"))
user = os.environ.get("DEPLOY_USER", "root")
password = os.environ["DEPLOY_PASS"]

if len(sys.argv) > 1 and sys.argv[1] == "-":
    cmd = sys.stdin.buffer.read().decode("utf-8", errors="replace")
else:
    cmd = sys.argv[1]

client = paramiko.SSHClient()
client.set_missing_host_key_policy(paramiko.AutoAddPolicy())
client.connect(
    hostname=host,
    port=port,
    username=user,
    password=password,
    timeout=30,
    banner_timeout=30,
    auth_timeout=30,
)

# Use get_pty so long-running build output streams; merge stderr into stdout.
chan = client.get_transport().open_session()
chan.get_pty()
chan.exec_command(cmd)

buf = b""
while True:
    data = chan.recv(4096)
    if not data:
        break
    sys.stdout.buffer.write(data)
    sys.stdout.buffer.flush()

status = chan.recv_exit_status()
client.close()
sys.exit(status)
