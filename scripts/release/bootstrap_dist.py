#!/usr/bin/env python3
"""Fetch the pinned dist binary without installing into the user's PATH."""
import argparse
import hashlib
import io
from pathlib import Path
import platform
import tarfile
import urllib.request

VERSION = "0.33.0"
DIGESTS = {
    "aarch64-apple-darwin": "7b3cbe25511de01d74c0f5fcb7909edabd379bea9cfa284d93af5a3cdfa3247c",
    "x86_64-apple-darwin": "6a49bfb61bd86770d79c27f3d2b40c6b2e71cde940d3d31a6ccaaffc124d7a29",
    "aarch64-unknown-linux-gnu": "9c554ab21a58ad46eb9b6710f89633ccb26bc2577cf4b0c2d18a5b826a31913e",
    "x86_64-unknown-linux-gnu": "4b3f0a5f0ebbdb798f6db649d01b32ba1518376b6f7a0502b7d92b75cc2c8293",
}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--directory", type=Path, required=True)
    args = parser.parse_args()
    machine = {"arm64": "aarch64", "aarch64": "aarch64", "x86_64": "x86_64"}[platform.machine()]
    system = {"Darwin": "apple-darwin", "Linux": "unknown-linux-gnu"}[platform.system()]
    target = f"{machine}-{system}"
    url = f"https://github.com/axodotdev/cargo-dist/releases/download/v{VERSION}/cargo-dist-{target}.tar.xz"
    with urllib.request.urlopen(url, timeout=120) as response:
        data = response.read()
    if hashlib.sha256(data).hexdigest() != DIGESTS[target]:
        raise SystemExit("dist archive checksum mismatch")
    args.directory.mkdir(parents=True, exist_ok=True)
    with tarfile.open(fileobj=io.BytesIO(data)) as archive:
        members = [m for m in archive if m.isfile() and m.name.endswith("/dist")]
        if len(members) != 1:
            raise SystemExit("Unexpected dist archive layout")
        executable = args.directory / "dist"
        executable.write_bytes(archive.extractfile(members[0]).read())
        executable.chmod(0o755)
    print(executable.resolve())


if __name__ == "__main__":
    main()
