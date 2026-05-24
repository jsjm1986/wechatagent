"""
Tiny paramiko-based SCP/SFTP uploader for the deploy flow.

Usage:
    python scripts/deploy/ssh_put.py <local> <remote>
    python scripts/deploy/ssh_put.py --dir <local_dir> <remote_dir>

Reads SSH_HOST / SSH_USER / SSH_PASS from env.
"""

from __future__ import annotations

import os
import posixpath
import sys

import paramiko


def upload_file(sftp: paramiko.SFTPClient, local: str, remote: str) -> None:
    parent = posixpath.dirname(remote)
    if parent:
        try:
            sftp.stat(parent)
        except FileNotFoundError:
            mkdirs(sftp, parent)
    sftp.put(local, remote)
    print(f"[put] {local} -> {remote}", file=sys.stderr)


def mkdirs(sftp: paramiko.SFTPClient, remote_dir: str) -> None:
    parts = remote_dir.strip("/").split("/")
    cur = ""
    for p in parts:
        cur += "/" + p
        try:
            sftp.stat(cur)
        except FileNotFoundError:
            sftp.mkdir(cur)


def upload_dir(sftp: paramiko.SFTPClient, local_dir: str, remote_dir: str,
               excludes: set[str]) -> None:
    mkdirs(sftp, remote_dir)
    for root, dirs, files in os.walk(local_dir):
        # prune excluded
        dirs[:] = [d for d in dirs if d not in excludes]
        rel = os.path.relpath(root, local_dir).replace("\\", "/")
        remote_root = remote_dir if rel == "." else posixpath.join(remote_dir, rel)
        try:
            sftp.stat(remote_root)
        except FileNotFoundError:
            mkdirs(sftp, remote_root)
        for fname in files:
            if fname in excludes:
                continue
            local_path = os.path.join(root, fname)
            remote_path = posixpath.join(remote_root, fname)
            sftp.put(local_path, remote_path)
            print(f"[put] {local_path}", file=sys.stderr)


def main() -> int:
    host = os.environ["SSH_HOST"]
    user = os.environ["SSH_USER"]
    password = os.environ["SSH_PASS"]
    port = int(os.environ.get("SSH_PORT", "22"))

    args = sys.argv[1:]
    if not args:
        print("usage: ssh_put.py [--dir] <local> <remote>", file=sys.stderr)
        return 2

    is_dir = False
    if args[0] == "--dir":
        is_dir = True
        args = args[1:]

    local, remote = args[0], args[1]
    excludes = set((os.environ.get("UPLOAD_EXCLUDES", "")).split(",")) - {""}

    client = paramiko.SSHClient()
    client.set_missing_host_key_policy(paramiko.AutoAddPolicy())
    client.connect(
        hostname=host,
        port=port,
        username=user,
        password=password,
        look_for_keys=False,
        allow_agent=False,
        timeout=60,
        banner_timeout=30,
        auth_timeout=30,
    )
    try:
        sftp = client.open_sftp()
        if is_dir:
            upload_dir(sftp, local, remote, excludes)
        else:
            upload_file(sftp, local, remote)
        sftp.close()
    finally:
        client.close()
    return 0


if __name__ == "__main__":
    sys.exit(main())
