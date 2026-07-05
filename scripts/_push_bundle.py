"""Stream a local file to the server in base64 chunks via shell args.

Bypasses the flaky GitHub link and SFTP (/tmp NO_SUCH_FILE). Appends ~30KB
base64 chunks to a remote file via `printf '%s' <chunk> >> file` (each chunk a
shell arg, well under ARG_MAX), then `base64 -d` on the server. Avoids stdin
streaming (which closed the channel on large writes).
Usage: DEPLOY_PASS=... python scripts/_push_bundle.py <local> <remote>
"""
import base64
import os
import sys

import paramiko

host = os.environ.get("DEPLOY_HOST", "117.72.54.28")
port = int(os.environ.get("DEPLOY_PORT", "22"))
user = os.environ.get("DEPLOY_USER", "root")
password = os.environ["DEPLOY_PASS"]

local, remote = sys.argv[1], sys.argv[2]
remote_b64 = remote + ".b64"
with open(local, "rb") as f:
    b64 = base64.b64encode(f.read()).decode("ascii")

client = paramiko.SSHClient()
client.set_missing_host_key_policy(paramiko.AutoAddPolicy())
client.connect(hostname=host, port=port, username=user, password=password,
               timeout=30, banner_timeout=30, auth_timeout=30)


def run(cmd: str) -> tuple[int, str]:
    _, stdout, stderr = client.exec_command(cmd)
    out = stdout.read().decode("utf-8", "replace")
    err = stderr.read().decode("utf-8", "replace")
    rc = stdout.channel.recv_exit_status()
    return rc, out + err


run(f"rm -f {remote_b64} {remote}")
CHUNK = 30000
n = (len(b64) + CHUNK - 1) // CHUNK
for i in range(n):
    part = b64[i * CHUNK:(i + 1) * CHUNK]
    rc, msg = run(f"printf '%s' '{part}' >> {remote_b64}")
    if rc != 0:
        print(f"chunk {i}/{n} failed rc={rc}: {msg}")
        sys.exit(1)
rc, msg = run(f"base64 -d {remote_b64} > {remote} && rm -f {remote_b64} && wc -c {remote}")
print(f"exit={rc}\n{msg}")
client.close()
sys.exit(rc)
