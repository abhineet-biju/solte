#!/usr/bin/env python3
"""Exercise the real generated installer against an isolated loopback mirror."""
import argparse
import functools
import hashlib
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import platform
import re
import shutil
import subprocess
import tarfile
import tempfile
import threading


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def portability(binary, target):
    machine = {"arm64": "aarch64", "aarch64": "aarch64", "x86_64": "x86_64"}[platform.machine()]
    assert target.startswith(machine + "-"), "This check must RUN natively, not just cross-compile"
    report = {"target": target, "native_machine": platform.machine(), "os": platform.platform()}
    if "apple" in target:
        dependencies = subprocess.check_output(["otool", "-L", str(binary)], text=True)
        libraries = [line.strip().split(" (")[0] for line in dependencies.splitlines()[1:]]
        assert all(lib.startswith(("/usr/lib/", "/System/Library/")) for lib in libraries), libraries
        load = subprocess.check_output(["xcrun", "vtool", "-show-build", str(binary)], text=True)
        minimum = re.search(r"minos\s+(\d+)\.(\d+)", load)
        assert minimum and tuple(map(int, minimum.groups())) <= (11, 0), load
        report.update(dependencies=libraries, minimum_macos=minimum.group(0))
    else:
        dependencies = subprocess.check_output(["readelf", "-d", str(binary)], text=True)
        libraries = re.findall(r"Shared library: \[([^]]+)\]", dependencies)
        allowed = {"libc.so.6", "libm.so.6", "libgcc_s.so.1", "libpthread.so.0", "libdl.so.2", "librt.so.1", "ld-linux-x86-64.so.2", "ld-linux-aarch64.so.1"}
        assert set(libraries) <= allowed, libraries
        symbols = subprocess.check_output(["readelf", "--version-info", str(binary)], text=True)
        versions = [tuple(map(int, version.split("."))) for version in re.findall(r"GLIBC_([0-9]+\.[0-9]+)", symbols)]
        assert versions and max(versions) <= (2, 35), sorted(set(versions))
        linked = subprocess.check_output(["ldd", str(binary)], text=True)
        assert "not found" not in linked, linked
        report.update(dependencies=libraries, minimum_glibc=".".join(map(str, max(versions))))
    return report


class Mirror(SimpleHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def do_GET(self):
        if self.server.corrupt and self.path.endswith(".tar.xz"):
            self.send_response(200)
            self.end_headers()
            self.wfile.write(b"deliberately corrupted archive")
        else:
            super().do_GET()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--dist-dir", type=Path, default=Path("target/distrib"))
    parser.add_argument("--target", required=True)
    parser.add_argument("--version", required=True)
    parser.add_argument("--report", type=Path)
    parser.add_argument("--previous-binary", type=Path, help="Also exercise replacement of a real earlier build")
    args = parser.parse_args()
    directory = args.dist_dir.resolve()
    archive_path = directory / f"solte-{args.target}.tar.xz"
    installer = directory / "solte-installer.sh"
    checksum = (directory / (archive_path.name + ".sha256")).read_text().split()[0]
    assert digest(archive_path) == checksum
    assert checksum in installer.read_text(), "Installer must embed the actual archive checksum"
    with tempfile.TemporaryDirectory(prefix="solte-install-") as temporary:
        root = Path(temporary)
        fake_home = root / "home"
        project = root / "project"
        project_data = project / ".solte"
        project_data.mkdir(parents=True)
        for name, content in {"config.toml": "theme = 'neon'\n", "keys/test.json": "test key sentinel\n", "history.sqlite": "history sentinel\n"}.items():
            path = project_data / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(content)
        (project / "wallet.json").write_text("external key sentinel\n")
        before = {str(p.relative_to(project)): digest(p) for p in project.rglob("*") if p.is_file()}
        install_dir = fake_home / ".local/bin"
        install_dir.mkdir(parents=True)
        binary = install_dir / "solte"
        binary.write_text("#!/bin/sh\nprintf 'solte 0.0.0-install-test\\n'\n")
        if args.previous_binary:
            shutil.copy2(args.previous_binary, binary)
        binary.chmod(0o755)
        server = ThreadingHTTPServer(("127.0.0.1", 0), functools.partial(Mirror, directory=str(directory)))
        server.corrupt = False
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        env = {k: v for k, v in os.environ.items() if not k.startswith(("SOLTE_", "CARGO_DIST_", "INSTALLER_", "XDG_", "GITHUB_", "ACTIONS_", "GH_TOKEN"))}
        env.update(HOME=str(fake_home), XDG_CONFIG_HOME=str(fake_home / ".config"), XDG_DATA_HOME=str(fake_home / ".local/share"),
                   SOLTE_DOWNLOAD_URL=f"http://127.0.0.1:{server.server_port}", PATH="/usr/bin:/bin:/usr/sbin:/sbin", TERM="xterm-256color")
        previous_version = subprocess.check_output([binary, "--version"], text=True).strip()
        assert previous_version.startswith("solte ") and previous_version != f"solte {args.version}"
        try:
            for _ in range(2):
                result = subprocess.run(["/bin/sh", str(installer)], cwd=project, env=env, capture_output=True, text=True, timeout=90)
                assert result.returncode == 0, result.stdout + result.stderr
                assert "no checksums to verify" not in result.stderr
                assert subprocess.check_output([binary, "--version"], env=env, text=True).strip() == f"solte {args.version}"
            path_setup = fake_home / ".config/solte/env.sh"
            assert path_setup.is_file(), "Default install should provide a PATH setup file"
            resolved = subprocess.check_output(["/bin/sh", "-c", '. "$XDG_CONFIG_HOME/solte/env.sh"; command -v solte'], env=env, text=True).strip()
            assert resolved == str(binary), resolved
            installed_hash = digest(binary)
            server.corrupt = True
            failed = subprocess.run(["/bin/sh", str(installer)], cwd=project, env=env, capture_output=True, text=True, timeout=90)
            assert failed.returncode != 0, "Corrupted artifact must not install"
            assert digest(binary) == installed_hash, "Rejected update must leave the installed binary intact"
            server.corrupt = False
            custom = root / "custom-bin"
            subprocess.run(["/bin/sh", str(installer)], cwd=project, env=dict(env, SOLTE_INSTALL_DIR=str(custom), SOLTE_NO_MODIFY_PATH="1"), check=True, capture_output=True, timeout=90)
            assert (custom / "solte").is_file()
            with tarfile.open(archive_path) as archive:
                assert any(m.name.endswith("/LICENSE") for m in archive.getmembers())
                member = next(m for m in archive if m.isfile() and m.name.endswith("/solte"))
                unpacked = root / "archive-solte"
                unpacked.write_bytes(archive.extractfile(member).read())
                unpacked.chmod(0o755)
            assert digest(unpacked) == installed_hash
            report = portability(binary, args.target)
            subprocess.run([binary, "--demo", "--snapshot", str(root / "demo.svg")], cwd=project, env=env, check=True, stdout=subprocess.DEVNULL)
            subprocess.run([shutil.which("python3") or "python3", str(Path(__file__).resolve().parents[1] / "terminal_smoke.py"), "--binary", str(binary)], env=env, check=True, timeout=120)
            after = {str(p.relative_to(project)): digest(p) for p in project.rglob("*") if p.is_file()}
            assert before == after, "Installation or demo launch modified wallet/configuration data"
            report.update(version=args.version, previous_version=previous_version, archive_sha256=checksum, tests="upgrade, reinstall, checksum rejection, custom path, archive, version, demo, terminal smoke, project preservation")
            if args.report:
                args.report.parent.mkdir(parents=True, exist_ok=True)
                args.report.write_text(json.dumps(report, indent=2) + "\n")
            print(json.dumps(report, indent=2))
        finally:
            server.shutdown()
            server.server_close()
            thread.join()


if __name__ == "__main__":
    main()
