#!/usr/bin/env python3
"""Exercise token inspection, export, forms, mouse input, and resizing in a PTY."""
import argparse
import json
from pathlib import Path
import tempfile

from terminal_session import TerminalSession


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=Path("target/debug/solte"))
    parser.add_argument("--artifact-dir", type=Path, help="Save terminal state on failure")
    args = parser.parse_args()
    with tempfile.TemporaryDirectory(prefix="solte-token-terminal-") as temporary:
        root = Path(temporary)
        command = [str(args.binary.resolve()), "--demo", "--view", "tokens", "--project", str(root), "--reduced-motion"]
        with TerminalSession(command, 120, 32, artifact_dir=args.artifact_dir) as terminal:
            terminal.wait_text("Token accounts")
            terminal.send(b"j\r", "Token account inspector")
            terminal.send(b"y")
            account = terminal.clipboard()
            terminal.send(b"M")
            mint = terminal.clipboard()
            assert account != mint
            terminal.click_text("Name [L]", "Name mint & account locally")
            terminal.send(b"Demo USD\tBuyer balance\r", "Token account inspector")
            terminal.wait_text("Demo USD")
            terminal.send(b"M")
            assert terminal.clipboard() == mint, "Local names must preserve the copied mint address"
            terminal.escape("Token account inspector")
            terminal.send(b"/", "Filter token accounts")
            terminal.send(b"Buyer balance\r", "Filter: Buyer balance")
            terminal.send(b"\r", "Token account inspector")
            terminal.send(b"y")
            assert terminal.clipboard() == account, "Account names should find the same account"
            terminal.send(b"L", "Name mint & account locally")
            terminal.send(b"\x15\t\x15\r", "Token account inspector")
            terminal.wait_for(lambda: "Demo USD" not in terminal.text, "cleared local names")
            terminal.escape("Token account inspector")
            terminal.send(b"x")
            terminal.send(b"/", "Filter token accounts")
            terminal.send(b"Token-2022\r", "Filter: Token-2022")
            terminal.send(b"\r", "Token account inspector")
            terminal.send(b"E", "Export token account")
            terminal.send(b"\x15token.json\r")
            exported = root / "token.json"
            terminal.wait_for(exported.exists, "token JSON export")
            data = json.loads(exported.read_text())
            assert data["program"] == "TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb"
            assert "mint_info" in data and "account" in data
            terminal.send(b"s", "Send tokens")
            for width, height in [(60, 10), (80, 20), (120, 32)]:
                terminal.resize(width, height)
                terminal.send(b"\t")
                terminal.wait_text("Send tokens")
            terminal.escape("Send tokens")
            terminal.send(b"a", "Create associated token account")
            terminal.escape("Create associated token account")
            terminal.send(b"x")
            terminal.wait_for(lambda: "Filter:" not in terminal.text, "cleared token filter")
            terminal.click_text(mint[:6], "Token account inspector", max_column=55)
            terminal.send(b"M")
            assert terminal.clipboard() == mint, "Mouse should inspect the intended token row"
            terminal.escape("Token account inspector")
            terminal.send(b"c", "Associated token account")
            terminal.send(b"\r", "Create token mint")
            terminal.resize(88, 22)
            terminal.wait_text("more below")
            terminal.send(b"\x1b[<65;40;12M")
            terminal.wait_text("more above")
            for width, height in [(60, 10), (80, 20), (120, 32)]:
                terminal.resize(width, height)
                for _ in range(6):
                    terminal.send(b"\t")
                terminal.wait_text("Create token mint")
            terminal.escape("Create token mint")
            terminal.send(b"c", "Associated token account")
            terminal.send(b"j\r", "Create associated token account")
            terminal.escape("Create associated token account")
            terminal.send(b"v", "Project mints")
            terminal.send(b"j\r", "Mint inspector")
            terminal.wait_text("Mint more [m]")
            terminal.wait_text("View ATA [t]")
            assert "Create ATA [a]" not in terminal.text, "An existing ATA should offer inspection"
            terminal.send(b"y")
            project_mint = terminal.clipboard()
            assert len(project_mint) >= 32
            terminal.send(b"t", "Token account inspector")
            terminal.send(b"y")
            assert terminal.clipboard() != project_mint, "View account should inspect the active wallet's token account"
            terminal.send(b"m", "Mint tokens")
            terminal.wait_text("Requires a loaded mint authority")
            terminal.escape("Mint tokens")
            terminal.send(b"c", "Associated token account")
            terminal.click_text("Token mint", "Create token mint")
            terminal.escape("Create token mint")
            terminal.send(b"q")
            terminal.process.wait(timeout=5)
            assert terminal.process.returncode == 0
            assert not (root / ".solte").exists(), "Demo token workflow must not create project wallet data"
            print("PASS: token inspection, local names, copying, filtering, JSON export, mint creation, existing ATA actions, mouse, resizing, and exit")


if __name__ == "__main__":
    main()
