#!/usr/bin/env python3
"""Exercise Solte's actual keyboard and mouse input through a pseudo-terminal."""

import argparse
import base64
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
    parser.add_argument("--format", choices=["auto", "legacy", "v0", "v1"], default="auto")
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

            send(b"\x1b[<0;4;29M")
            send(b"\x15seller\r")
            seller = root / ".solte/keys/seller.json"
            wait_for(seller.exists)
            drain(0.5)
            saved_selection = (root / ".solte/config.toml").read_text()
            send(b"2jk\r")
            assert b"Wallet details" in output
            send(b"y")
            assert (root / ".solte/config.toml").read_text() == saved_selection, "Preview/copy must not activate a wallet"
            send(b"\x1b")
            send(b"6j\r")
            assert b"Log details" in output
            send(b"y")
            send(b"\x1b")
            send(b"1")
            send(b"y")
            assert b"\x1b]52;" in output, "Copy must emit an OSC 52 clipboard request"
            send(b"t")
            assert b"Choose theme" in output
            send(b"k\x1b")
            assert "glacier" not in (root / ".solte/config.toml").read_text()
            send(b"tkk\r")
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
                send(recipient.encode() + b"\t\x150.1\t" + b"\x1b[C" * ["auto", "legacy", "v0", "v1"].index(args.format) + b"\r", 2)
                assert b"Review SOL transfer" in output
                send(b"\r")
                wait_for(lambda: balance(recipient) == 100_000_000)
                drain(3)
                send(b"r", 2)
                send(b"3")
                send(b"\x1b[<0;10;13M", 1)
                assert b"Transaction inspector" in output
                send(b"\x1b")

                payload = json.dumps({"jsonrpc":"2.0", "id":1, "method":"getLatestBlockhash", "params":[{"commitment":"confirmed"}]}).encode()
                request = urllib.request.Request(args.local_rpc, payload, {"Content-Type":"application/json"})
                with urllib.request.urlopen(request, timeout=3) as response:
                    blockhash = json.load(response)["result"]["value"]["blockhash"]
                alphabet = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz"
                number = 0
                for char in blockhash:
                    number = number * 58 + alphabet.index(char)
                hash_bytes = number.to_bytes(32, "big")
                payer_bytes = bytes(json.loads(buyer.read_text())[32:])
                recipient_bytes = bytes(json.loads(seller.read_text())[32:])
                message = b"\x01\x00\x01\x03" + payer_bytes + recipient_bytes + bytes(32) + hash_bytes
                message += b"\x01\x02\x02\x00\x01\x0c" + struct.pack("<IQ", 2, 100_000_000)
                source = root / "import.base64"
                destination = root / "signed.base64"
                source.write_bytes(base64.b64encode(b"\x01" + bytes(64) + message))
                send(b"I")
                send(str(source).encode() + b"\t\t\x15" + str(destination).encode() + b"\r", 2)
                assert b"Review imported transaction" in output
                send(b"\r")
                wait_for(destination.exists)
                assert balance(recipient) == 100_000_000, "Export must not submit"
                exported = base64.b64decode(destination.read_bytes())
                assert exported[65:] == message, "Import must preserve the exact message"
                assert exported[1:65] != bytes(64), "Export must include the selected signature"
                send(b"I")
                send(str(destination).encode() + b"\t\x1b[C\r", 2)
                send(b"\r")
                wait_for(lambda: balance(recipient) == 200_000_000)
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
                print(f"PASS: {args.format} TUI funding/transfer, inspection, import/export without submission, and imported submission")
        finally:
            if process.poll() is None:
                process.terminate()
                process.wait(timeout=5)
            os.close(master)


if __name__ == "__main__":
    main()
