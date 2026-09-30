#!/usr/bin/env python3
"""Exercise Solte's actual keyboard and mouse input through a pseudo-terminal."""

import argparse
import base64
import json
from pathlib import Path
import struct
import subprocess
import tempfile
import urllib.request

from terminal_session import TerminalSession


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=Path("target/debug/solte"))
    parser.add_argument("--local-rpc", help="Optional loopback RPC to test funding and transfer")
    parser.add_argument("--local-ws", help="Matching loopback WebSocket endpoint")
    parser.add_argument("--format", choices=["auto", "legacy", "v0", "v1"], default="auto")
    parser.add_argument("--artifact-dir", type=Path, help="Save terminal state on failure")
    args = parser.parse_args()
    if args.local_rpc:
        from urllib.parse import urlparse

        for endpoint in [args.local_rpc, args.local_ws]:
            assert endpoint and urlparse(endpoint).hostname in {"127.0.0.1", "localhost", "::1"}, "Live tests require loopback RPC and WebSocket endpoints"

    with tempfile.TemporaryDirectory(prefix="solte-terminal-") as directory:
        root = Path(directory)
        command = [str(args.binary.resolve()), "--project", str(root), "--reduced-motion"]
        if not args.local_rpc:
            command.append("--offline")
        else:
            (root / ".solte").mkdir(mode=0o700)
            (root / ".solte/config.toml").write_text(
                '[[profiles]]\nname = "Local test"\n'
                f'http = "{args.local_rpc}"\nwebsocket = "{args.local_ws}"\n'
            )
        def address(path):
            return subprocess.check_output(["solana-keygen", "pubkey", str(path)], text=True).strip()

        def rpc(method, params):
            payload = json.dumps({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}).encode()
            request = urllib.request.Request(args.local_rpc, payload, {"Content-Type": "application/json"})
            with urllib.request.urlopen(request, timeout=3) as response:
                return json.load(response)["result"]

        def balance(pubkey):
            return rpc("getBalance", [pubkey, {"commitment": "confirmed"}])["value"]

        def latest_signature(pubkey):
            return rpc("getSignaturesForAddress", [pubkey, {"limit": 1, "commitment": "confirmed"}])[0]["signature"]

        with TerminalSession(command, 140, 42, artifact_dir=args.artifact_dir) as terminal:
            terminal.wait_text("Create wallet [n]")
            terminal.send(b"n", "Create development wallet")
            terminal.send(b"\x15buyer\r")
            buyer = root / ".solte/keys/buyer.json"
            terminal.wait_for(lambda: buyer.exists() and "Create development wallet" not in terminal.text and "buyer" in terminal.text, "created buyer wallet")
            assert len(json.loads(buyer.read_text())) == 64
            assert buyer.stat().st_mode & 0o777 == 0o600

            terminal.click_text("New wallet [n]", "Create development wallet")
            terminal.send(b"\x15seller\r")
            seller = root / ".solte/keys/seller.json"
            terminal.wait_for(lambda: seller.exists() and "Create development wallet" not in terminal.text and "seller" in terminal.text, "created seller wallet")
            config = root / ".solte/config.toml"
            saved_selection = config.read_text()
            terminal.send(b"2jk\r", "Wallet details")
            terminal.send(b"y")
            assert terminal.clipboard(), "Copy must emit an OSC 52 clipboard request"
            assert config.read_text() == saved_selection, "Preview/copy must not activate a wallet"
            terminal.escape("Wallet details")
            terminal.send(b"6j\r", "Log details")
            terminal.send(b"y")
            assert terminal.clipboard()
            terminal.escape("Log details")
            terminal.send(b"1")
            terminal.send(b"y")
            assert terminal.clipboard()
            terminal.send(b"t", "Choose theme")
            terminal.send(b"k")
            terminal.escape("Choose theme")
            assert config.read_text() == saved_selection, "Cancelling theme selection must not persist a change"
            terminal.send(b"t", "Choose theme")
            terminal.send(b"kk\r")
            terminal.wait_for(lambda: 'theme = "glacier"' in config.read_text(), "saved Glacier theme")
            terminal.send(b"t", "Choose theme")
            terminal.send(b"jj\r")
            terminal.wait_for(lambda: 'theme = "neon"' in config.read_text(), "saved Neon theme")
            terminal.send(b"m", "Choose motion")
            terminal.send(b"k\r")
            terminal.wait_for(lambda: "reduced_motion = false" in config.read_text(), "saved motion setting")
            if args.local_rpc:
                recipient = address(seller)
                payer = address(buyer)
                terminal.send(b"]")
                terminal.wait_text("● buyer")
                terminal.send(b"f", "Amount · SOL")
                terminal.send(b"\r")
                terminal.wait_for(lambda: balance(payer) >= 1_000_000_000, "funded buyer wallet")
                terminal.wait_text(f"Confirmed {latest_signature(payer)[:7]}")
                terminal.send(b"s", "Recipient address")
                terminal.send(recipient.encode() + b"\t\x150.1\t" + b"\x1b[C" * ["auto", "legacy", "v0", "v1"].index(args.format) + b"\r", "Review SOL transfer")
                terminal.send(b"\r")
                terminal.wait_for(lambda: balance(recipient) == 100_000_000, "recipient transfer balance")
                signature = latest_signature(recipient)
                terminal.wait_text(f"Confirmed {signature[:7]}")
                terminal.send(b"4", "Import tx [I]")
                terminal.send(b"r")
                terminal.click_text(f"{signature[:7]}…{signature[-6:]}", "Transaction inspector")
                terminal.send(b"y")
                assert terminal.clipboard() == signature, "Mouse should inspect the intended transaction"
                terminal.escape("Transaction inspector")

                blockhash = rpc("getLatestBlockhash", [{"commitment": "confirmed"}])["value"]["blockhash"]
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
                terminal.send(b"I", "Import transaction")
                terminal.send(str(source).encode() + b"\t\t\x15" + str(destination).encode() + b"\r", "Review imported transaction")
                terminal.send(b"\r")
                terminal.wait_for(destination.exists, "signed transaction export")
                terminal.wait_text("Signed transaction exported")
                assert balance(recipient) == 100_000_000, "Export must not submit"
                exported = base64.b64decode(destination.read_bytes())
                assert exported[65:] == message, "Import must preserve the exact message"
                assert exported[1:65] != bytes(64), "Export must include the selected signature"
                terminal.send(b"I", "Import transaction")
                terminal.send(str(destination).encode() + b"\t\x1b[C\r", "Review imported transaction")
                terminal.send(b"\r")
                terminal.wait_for(lambda: balance(recipient) == 200_000_000, "imported transfer balance")
                terminal.send(b"6")
                terminal.wait_text(f"Confirmed {latest_signature(recipient)[:7]}")
                terminal.wait_for(lambda: "Waiting for transaction confirmation" not in terminal.text, "finished imported submission")
                terminal.send(b"4", "Import tx [I]")
            else:
                terminal.send(b"p", "RPC profiles")
                terminal.send(b"\x1b[B\r")
                terminal.wait_for(lambda: "selected_profile = 1" in config.read_text(), "saved RPC profile")
            terminal.send(b"p", "RPC profiles")
            for columns, rows in [(100, 24), (99, 23), (80, 10), (60, 10), (45, 8), (60, 10), (160, 48), (140, 42)]:
                terminal.resize(columns, rows)
                assert terminal.process.poll() is None, "Resizing a dialog stopped the application"
            terminal.escape("RPC profiles")
            for columns, rows in [(120, 14), (90, 22), (80, 10), (79, 10), (60, 10), (45, 8), (140, 42)]:
                terminal.resize(columns, rows)
                assert terminal.process.poll() is None, "Resizing the workspace stopped the application"
            terminal.send(b"q")
            terminal.process.wait(timeout=5)
            while terminal.read(0):
                pass
            assert b"\x1b[6n" not in terminal.output, "Fullscreen resizing must not query the cursor"
            assert terminal.process.returncode == 0
            assert b"panicked" not in terminal.output
            assert 'theme = "neon"' in config.read_text()
            assert "reduced_motion = false" in config.read_text()
            print("PASS: keyboard and mouse creation, settings persistence, and clean exit")
            if args.local_rpc:
                print(f"PASS: {args.format} TUI funding/transfer, inspection, import/export without submission, and imported submission")


if __name__ == "__main__":
    main()
