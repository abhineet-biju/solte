#!/usr/bin/env python3
"""Publish tested assets from a trusted push. Never replace an existing release."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess

from prepare import select

REPOSITORY = "abhineet-biju/solte"


def publication(spec, event, ref, source, repository):
    if event != "push" or repository != REPOSITORY or source != spec["source_commit"] or ref != spec["source_ref"]:
        raise ValueError("Publication must use the exact source of a trusted push")
    if spec["channel"] not in {"development", "stable"}:
        raise ValueError("Check builds cannot publish")
    if spec["channel"] == "development":
        suffix = spec["version"].split("-dev.", 1)[1].split(".")
        run, attempt = int(suffix[0]), int(suffix[1])
    else:
        run, attempt = 1, 1
    expected = select(spec["channel"], spec["base_version"], source, ref, run, attempt)
    if spec != expected:
        raise ValueError("Invalid release metadata")
    return {"draft": False, "prerelease": spec["prerelease"], "make_latest": "false" if spec["prerelease"] else "legacy"}


def api(path, payload=None):
    command = ["gh", "api", f"repos/{REPOSITORY}/{path}"]
    if payload is not None:
        command += ["--method", "PATCH", "--input", "-"]
    result = subprocess.run(command, input=json.dumps(payload) if payload is not None else None, capture_output=True, text=True)
    if result.returncode:
        if payload is None and "(HTTP 404)" in result.stderr:
            return None
        raise RuntimeError(f"GitHub API failed for {path}: {result.stderr}")
    return json.loads(result.stdout)


def find_release(tag):
    # gh resolves both published tags and drafts with pending tags.
    result = subprocess.run(["gh", "release", "view", tag, "--repo", REPOSITORY, "--json", "databaseId"], capture_output=True, text=True)
    if result.returncode:
        if result.stderr.strip() == "release not found":
            return None
        raise RuntimeError(f"Release lookup failed: {result.stderr}")
    return api(f"releases/{json.loads(result.stdout)['databaseId']}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--assets", type=Path, required=True)
    parser.add_argument("--dry-run", action="store_true", help="Validate files without contacting GitHub")
    args = parser.parse_args()
    spec = json.loads((args.assets / "release-build.json").read_text())
    patch = publication(spec, os.environ.get("GITHUB_EVENT_NAME"), os.environ.get("GITHUB_REF"), os.environ.get("GITHUB_SHA"), os.environ.get("GITHUB_REPOSITORY"))
    actual = subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip()
    if actual != spec["source_commit"]:
        raise SystemExit("Publication checkout is not the packaged source")
    expected_files = {"sha256.sum"}
    for line in (args.assets / "sha256.sum").read_text().splitlines():
        checksum, name = line.split("  ", 1)
        if Path(name).name != name:
            raise SystemExit("Unsafe artifact path")
        path = args.assets / name
        if path.is_symlink() or hashlib.sha256(path.read_bytes()).hexdigest() != checksum:
            raise SystemExit(f"Artifact checksum mismatch: {name}")
        expected_files.add(name)
    if {p.name for p in args.assets.iterdir()} != expected_files:
        raise SystemExit("Unlisted release assets")
    if args.dry_run:
        print(json.dumps({"tag": spec["tag"], "source": actual, "publication": patch, "files": sorted(expected_files)}, indent=2))
        return
    if os.environ.get("GITHUB_ACTIONS") != "true":
        raise SystemExit("Publishing is restricted to the release workflow")
    if find_release(spec["tag"]) is not None:
        raise SystemExit("Release already exists; refusing to replace published or draft assets")
    if spec["channel"] == "stable":
        tagged = subprocess.check_output(["git", "rev-parse", f"{spec['tag']}^{{commit}}"], text=True).strip()
        if tagged != actual:
            raise SystemExit("Stable tag points to a different commit")
    command = ["gh", "release", "create", spec["tag"], "--repo", REPOSITORY, "--draft", "--latest=false", "--target", actual,
               "--title", f"Solte {spec['version']}", "--notes-file", str(args.assets / "release-notes.md")]
    if spec["prerelease"]:
        command.append("--prerelease")
    else:
        command.append("--verify-tag")
    command.extend(str(args.assets / name) for name in sorted(expected_files))
    subprocess.run(command, check=True)
    release = find_release(spec["tag"])
    if not release or not release["draft"] or {a["name"] for a in release["assets"]} != expected_files:
        raise SystemExit("Draft release assets are incomplete; leaving the release unpublished")
    api(f"releases/{release['id']}", patch)
    print(f"Published {spec['tag']} from {actual}")


if __name__ == "__main__":
    main()
