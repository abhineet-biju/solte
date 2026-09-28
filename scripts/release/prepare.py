#!/usr/bin/env python3
"""Validate a channel and stamp only Solte's version in a disposable build checkout."""
import argparse
import json
import os
from pathlib import Path
import re
import subprocess
import tomllib

VERSION_RE = r"(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)"
TARGETS = ["aarch64-apple-darwin", "x86_64-apple-darwin", "aarch64-unknown-linux-gnu", "x86_64-unknown-linux-gnu"]


def select(channel, base, source, ref, run, attempt):
    if not re.fullmatch(VERSION_RE, base):
        raise ValueError("Cargo.toml must contain a plain major.minor.patch version")
    if not re.fullmatch(r"[0-9a-f]{40}", source):
        raise ValueError("Source must be an immutable full commit SHA")
    if channel == "stable":
        if ref != f"refs/tags/v{base}":
            raise ValueError("Stable publication requires an explicit vMAJOR.MINOR.PATCH tag matching Cargo.toml")
        version = base
    elif channel == "development":
        if ref != "refs/heads/main" or run < 1 or attempt < 1:
            raise ValueError("Development releases require main and positive run/attempt numbers")
        major, minor, patch = map(int, base.split("."))
        version = f"{major}.{minor}.{patch + 1}-dev.{run}.{attempt}.g{source[:12]}"
    elif channel == "check":
        version = base
    else:
        raise ValueError("Unknown release channel")
    return {"channel": channel, "base_version": base, "version": version, "tag": f"v{version}",
            "source_commit": source, "source_ref": ref, "prerelease": channel == "development",
            "targets": TARGETS, "dist_version": "0.33.0", "rust_toolchain": "1.97.1"}


def stamp(root, version):
    manifest = root / "Cargo.toml"
    lockfile = root / "Cargo.lock"
    cargo = manifest.read_text()
    lock = lockfile.read_text()
    before = tomllib.loads(lock)
    package = tomllib.loads(cargo)["package"]
    entries = [p for p in before["package"] if p["name"] == "solte" and "source" not in p]
    if package["name"] != "solte" or len(entries) != 1 or entries[0]["version"] != package["version"]:
        raise ValueError("Root package and committed lockfile versions disagree")
    if version == package["version"]:
        return
    cargo, count = re.subn(r'(\[package\][\s\S]*?\nversion = ")[^"]+("\n)', lambda m: m[1] + version + m[2], cargo, count=1)
    lock, lock_count = re.subn(r'(\[\[package\]\]\nname = "solte"\nversion = ")[^"]+("\n)', lambda m: m[1] + version + m[2], lock)
    if count != 1 or lock_count != 1:
        raise ValueError("Cannot locate an unambiguous package version to stamp")
    expected = json.loads(json.dumps(before))
    next(p for p in expected["package"] if p["name"] == "solte" and "source" not in p)["version"] = version
    if tomllib.loads(lock) != expected:
        raise ValueError("Version stamping changed dependency resolution")
    manifest.write_text(cargo)
    lockfile.write_text(lock)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--channel", choices=["check", "development", "stable"], required=True)
    parser.add_argument("--source", required=True)
    parser.add_argument("--ref", required=True)
    parser.add_argument("--run", type=int, default=1)
    parser.add_argument("--attempt", type=int, default=1)
    parser.add_argument("--root", type=Path, default=Path.cwd())
    parser.add_argument("--output", type=Path, default=Path("target/release-build.json"))
    args = parser.parse_args()
    actual = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=args.root, text=True).strip()
    if actual != args.source:
        raise SystemExit("Checkout differs from the requested source commit")
    package = tomllib.loads((args.root / "Cargo.toml").read_text())["package"]
    if package.get("license") != "MIT" or not (args.root / "LICENSE").is_file():
        raise SystemExit("The approved project license must be included")
    spec = select(args.channel, package["version"], args.source, args.ref, args.run, args.attempt)
    stamp(args.root, spec["version"])
    subprocess.run(["cargo", "metadata", "--locked", "--no-deps", "--format-version=1"], cwd=args.root, check=True, stdout=subprocess.DEVNULL)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(spec, indent=2) + "\n")
    if os.environ.get("GITHUB_OUTPUT"):
        with open(os.environ["GITHUB_OUTPUT"], "a") as output:
            for key in ["version", "tag", "source_commit"]:
                output.write(f"{key}={spec[key]}\n")
    print(json.dumps(spec))


if __name__ == "__main__":
    main()
