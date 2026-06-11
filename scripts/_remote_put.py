#!/usr/bin/env python
"""Upload local files to the server over SFTP (paramiko).

Bypasses the flaky GitHub link for direct file sync. Reads password from env.
Usage: DEPLOY_PASS=... python scripts/_remote_put.py <local1> <remote1> [<local2> <remote2> ...]
"""
import os, sys
import paramiko

host = os.environ.get("DEPLOY_HOST", "117.72.54.28")
port = int(os.environ.get("DEPLOY_PORT", "22"))
user = os.environ.get("DEPLOY_USER", "root")
password = os.environ["DEPLOY_PASS"]

pairs = sys.argv[1:]
assert len(pairs) % 2 == 0 and pairs, "need <local> <remote> pairs"

client = paramiko.SSHClient()
client.set_missing_host_key_policy(paramiko.AutoAddPolicy())
client.connect(hostname=host, port=port, username=user, password=password,
               timeout=30, banner_timeout=30, auth_timeout=30)
sftp = client.open_sftp()
for i in range(0, len(pairs), 2):
    local, remote = pairs[i], pairs[i+1]
    sftp.put(local, remote)
    st = sftp.stat(remote)
    print(f"PUT {local} -> {remote} ({st.st_size} bytes)")
sftp.close()
client.close()
