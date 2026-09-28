#!/usr/bin/env python3
"""Exercise Solte's actual keyboard and mouse input through a pseudo-terminal."""

import argparse
import fcntl
import json
import os
from pathlib import Path
import pty
import select
import signal
import struct
import subprocess
import tempfile
import termios
import time
import urllib.request


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=Path("target/debug/solte"))
    parser.add_argument("--local-rpc", help="Optional loopback RPC to test funding and transfer")
    parser.add_argument("--local-ws", help="Matching loopback WebSocket endpoint")
    args = parser.parse_args()
    if args.local_rpc:
        from urllib.parse import urlparse

        assert urlparse(args.local_rpc).hostname in {"127.0.0.1", "localhost", "::1"}
        assert args.local_ws

    with tempfile.TemporaryDirectory(prefix="solte-terminal-") as directory:
        root = Path(directory)
        master, slave = pty.openpty()
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 42, 140, 0, 0))
        command = [str(args.binary.resolve()), "--project", str(root), "--reduced-motion"]
        if not args.local_rpc:
            command.append("--offline")
        else:
            (root / ".solte").mkdir(mode=0o700)
            (root / ".solte/config.toml").write_text(
                '[[profiles]]\nname = "Local test"\n'
                f'http = "{args.local_rpc}"\nwebsocket = "{args.local_ws}"\n'
            )
        process = subprocess.Popen(
            command, stdin=slave, stdout=slave, stderr=slave,
            env=dict(os.environ, TERM="xterm-256color", COLORTERM="truecolor"),
        )
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

        def send(data, wait=0.4):
            os.write(master, data)
            drain(wait)

        def wait_for(predicate, timeout=15):
            deadline = time.monotonic() + timeout
            while time.monotonic() < deadline:
                if predicate():
                    return
                drain(0.2)
            raise AssertionError("Timed out waiting for application state")

        def address(path):
            return subprocess.check_output(["solana-keygen", "pubkey", str(path)], text=True).strip()

        def balance(pubkey):
            payload = json.dumps({"jsonrpc": "2.0", "id": 1, "method": "getBalance", "params": [pubkey, {"commitment": "confirmed"}]}).encode()
            request = urllib.request.Request(args.local_rpc, payload, {"Content-Type": "application/json"})
            with urllib.request.urlopen(request, timeout=3) as response:
                return json.load(response)["result"]["value"]

        try:
            drain(0.8)
            send(b"n")
            send(b"\x15buyer\r")
            buyer = root / ".solte/keys/buyer.json"
            wait_for(buyer.exists)
            drain(0.5)
            assert len(json.loads(buyer.read_text())) == 64
            assert buyer.stat().st_mode & 0o777 == 0o600

            send(b"\x1b[<0;4;28M")
            send(b"\x15seller\r")
            seller = root / ".solte/keys/seller.json"
            wait_for(seller.exists)
            drain(0.5)
            send(b"t")
            assert b"Choose theme" in output
            send(b"j\x1b")
            assert "glacier" not in (root / ".solte/config.toml").read_text()
            send(b"tj\r")
            send(b"tjj\r")
            send(b"m")
            assert b"Choose motion" in output
            send(b"k\r")
            if args.local_rpc:
                recipient = address(seller)
                payer = address(buyer)
                send(b"]")
                send(b"f")
                send(b"\r")
                wait_for(lambda: balance(payer) >= 1_000_000_000)
                drain(3)
                send(b"s")
                send(recipient.encode() + b"\t\x150.1\r", 2)
                assert b"Review SOL transfer" in output
                send(b"\r")
                wait_for(lambda: balance(recipient) == 100_000_000)
                drain(3)
                send(b"r", 2)
                send(b"3")
                send(b"\x1b[<0;10;13M", 1)
                assert b"Transaction inspector" in output
                send(b"\x1b")
            else:
                send(b"p")
                send(b"\x1b[B\r", 0.8)
                assert "selected_profile = 1" in (root / ".solte/config.toml").read_text()
            send(b"p")
            for columns, rows in [(100, 24), (99, 23), (80, 10), (60, 10), (45, 8), (60, 10), (160, 48), (140, 42)]:
                fcntl.ioctl(master, termios.TIOCSWINSZ, struct.pack("HHHH", rows, columns, 0, 0))
                process.send_signal(signal.SIGWINCH)
                drain(0.3)
                assert process.poll() is None, "Resizing a dialog stopped the application"
            send(b"\x1b")
            for columns, rows in [(120, 14), (90, 22), (80, 10), (79, 10), (60, 10), (45, 8), (140, 42)]:
                fcntl.ioctl(master, termios.TIOCSWINSZ, struct.pack("HHHH", rows, columns, 0, 0))
                process.send_signal(signal.SIGWINCH)
                drain(0.3)
                assert process.poll() is None, "Resizing the workspace stopped the application"
            assert b"\x1b[6n" not in output, "Fullscreen resizing must not query the cursor"
            send(b"q")
            process.wait(timeout=5)
            assert process.returncode == 0
            assert b"panicked" not in output
            assert 'theme = "neon"' in (root / ".solte/config.toml").read_text()
            assert "reduced_motion = false" in (root / ".solte/config.toml").read_text()
            print("PASS: keyboard and mouse creation, settings persistence, and clean exit")
            if args.local_rpc:
                print("PASS: TUI funding, simulation review, transfer, and mouse transaction inspection")
        finally:
            if process.poll() is None:
                process.terminate()
                process.wait(timeout=5)
            os.close(master)


if __name__ == "__main__":
    main()
