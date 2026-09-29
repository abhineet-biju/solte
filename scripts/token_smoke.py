#!/usr/bin/env python3
"""Exercise token inspection, export, forms, mouse input, and resizing in a PTY."""
import argparse
import base64
import fcntl
import json
import os
from pathlib import Path
import pty
import re
import select
import signal
import struct
import subprocess
import tempfile
import termios
import time


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=Path("target/debug/solte"))
    args = parser.parse_args()
    with tempfile.TemporaryDirectory(prefix="solte-token-terminal-") as temporary:
        root = Path(temporary)
        master, slave = pty.openpty()
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 32, 120, 0, 0))
        process = subprocess.Popen([str(args.binary.resolve()), "--demo", "--view", "tokens", "--project", str(root), "--reduced-motion"],
                                   stdin=slave, stdout=slave, stderr=slave, env=dict(os.environ, TERM="xterm-256color"))
        os.close(slave)
        output = bytearray()

        def drain(seconds=0.3):
            deadline = time.monotonic() + seconds
            while time.monotonic() < deadline:
                if select.select([master], [], [], 0.05)[0]:
                    try:
                        output.extend(os.read(master, 65536))
                    except OSError:
                        break

        def send(data):
            output.clear()
            os.write(master, data)
            drain()

        def clipboard():
            matches = re.findall(rb"\x1b\]52;[^;]*;([A-Za-z0-9+/=]+)", output)
            assert matches, "Copy should emit an OSC 52 request"
            return base64.b64decode(matches[-1]).decode()

        def resize(width, height):
            fcntl.ioctl(master, termios.TIOCSWINSZ, struct.pack("HHHH", height, width, 0, 0))
            os.kill(process.pid, signal.SIGWINCH)
            drain()

        try:
            deadline = time.monotonic() + 10
            while b"Token accounts" not in output and time.monotonic() < deadline:
                drain()
            assert b"Token accounts" in output, "Tokens view should finish its initial render"
            send(b"j\r")
            assert b"Token account inspector" in output
            send(b"y")
            account = clipboard()
            send(b"M")
            mint = clipboard()
            assert account != mint
            send(b"\x1b/")
            send(b"Token-2022\r")
            send(b"j\r")
            assert b"Token account inspector" in output
            send(b"E")
            assert b"Export token account" in output
            send(b"\x15token.json\r")
            exported = root / "token.json"
            deadline = time.monotonic() + 5
            while not exported.exists() and time.monotonic() < deadline:
                drain()
            data = json.loads(exported.read_text())
            assert data["program"] == "TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb"
            assert "mint_info" in data and "account" in data
            send(b"s")
            assert b"Send tokens" in output
            for width, height in [(60, 10), (80, 20), (120, 32)]:
                resize(width, height)
                send(b"\t")
                assert process.poll() is None
            send(b"\x1b")
            send(b"a")
            assert b"Create associated token account" in output
            send(b"\x1b")
            send(b"x")
            send(b"\x1b[<0;4;9M")
            assert b"Token account inspector" in output, "Mouse row selection should open the same inspector"
            send(b"\x1b")
            send(b"q")
            process.wait(timeout=5)
            assert process.returncode == 0
            assert not (root / ".solte").exists(), "Demo token workflow must not create project wallet data"
            print("PASS: token selection, copying, filtering, JSON export, forms, mouse, resizing, and exit")
        finally:
            if process.poll() is None:
                process.terminate()
                process.wait(timeout=5)
            os.close(master)


if __name__ == "__main__":
    main()
