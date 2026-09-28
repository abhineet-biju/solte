#!/usr/bin/env python3
"""Verify all native builds and stage the exact public release assets."""
import hashlib
import json
from pathlib import Path
import shutil


def assemble(directory, spec, manifest, output):
    if manifest["announcement_tag"] != spec["tag"]:
        raise ValueError("Global manifest tag mismatch")
    if manifest["releases"][0]["app_version"] != spec["version"]:
        raise ValueError("Global manifest package version mismatch")
    if manifest["announcement_is_prerelease"] != spec["prerelease"]:
        raise ValueError("Global manifest channel mismatch")
    installer = directory / "solte-installer.sh"
    script = installer.read_text()
    files = [installer]
    for target in spec["targets"]:
        local = json.loads((directory / f"{target}-dist-manifest.json").read_text())
        if local["announcement_tag"] != spec["tag"] or local["releases"][0]["app_version"] != spec["version"]:
            raise ValueError(f"Mixed build versions for {target}")
        archive = directory / f"solte-{target}.tar.xz"
        checksum = directory / (archive.name + ".sha256")
        digest = hashlib.sha256(archive.read_bytes()).hexdigest()
        if checksum.read_text().split()[0] != digest or digest not in script:
            raise ValueError(f"Archive or installer checksum mismatch: {target}")
        files.extend([archive, checksum])
    output.mkdir(parents=True, exist_ok=False)
    for path in files:
        shutil.copy2(path, output / path.name)
    (output / "dist-manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    (output / "release-build.json").write_text(json.dumps(spec, indent=2) + "\n")
    note = "Development prerelease. Not the latest stable release." if spec["prerelease"] else "Stable release published from an explicit version tag."
    source = spec["source_commit"]
    notes = f"""{note}

Source commit: [{source}](https://github.com/abhineet-biju/solte/commit/{source})
Version: `{spec['version']}`
Rust: `{spec['rust_toolchain']}`; cargo-dist: `{spec['dist_version']}`.

Development builds stamp only the package version in Cargo.toml and Cargo.lock; dependency resolution is unchanged. Stable builds use the committed version. See release-build.json for the source ref and build metadata.

Install this exact version:

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/abhineet-biju/solte/releases/download/{spec['tag']}/solte-installer.sh | sh
```

Archives and checksums are attached as an alternative. These are development-wallet binaries, not independently audited software for real funds. See the release guide for platform requirements and verification.
"""
    (output / "release-notes.md").write_text(notes)
    sums = [f"{hashlib.sha256(path.read_bytes()).hexdigest()}  {path.name}" for path in sorted(output.iterdir())]
    (output / "sha256.sum").write_text("\n".join(sums) + "\n")


def main():
    spec = json.loads(Path("target/release-build.json").read_text())
    manifest = json.loads(Path("target/global-dist-manifest.json").read_text())
    assemble(Path("target/distrib"), spec, manifest, Path("target/release-assets"))


if __name__ == "__main__":
    main()
