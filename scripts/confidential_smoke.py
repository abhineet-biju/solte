#!/usr/bin/env python3
"""Exercise confidential setup, balances and withdrawal in a real terminal on loopback."""
import argparse
import json
from pathlib import Path
import sqlite3
import subprocess
import tempfile
from urllib.parse import urlparse

from terminal_session import TerminalSession


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, default=Path('target/debug/solte'))
    parser.add_argument('--rpc', default='http://127.0.0.1:19899')
    parser.add_argument('--ws', default='ws://127.0.0.1:19900')
    parser.add_argument('--solana-tools', type=Path, required=True)
    parser.add_argument('--artifact-dir', type=Path)
    args = parser.parse_args()
    assert urlparse(args.rpc).hostname in ('localhost', '127.0.0.1', '::1')
    assert urlparse(args.ws).hostname in ('localhost', '127.0.0.1', '::1')
    with tempfile.TemporaryDirectory(prefix='solte-confidential-pty-') as temporary:
        root = Path(temporary)
        (root / 'keys').mkdir()
        (root / '.solte').mkdir()
        key = root / 'keys/alice.json'
        subprocess.run([str(args.solana_tools / 'solana-keygen'), 'new', '--no-bip39-passphrase', '--silent', '--outfile', str(key)], check=True, capture_output=True)
        address = subprocess.check_output([str(args.solana_tools / 'solana-keygen'), 'pubkey', str(key)], text=True).strip()
        subprocess.run([str(args.solana_tools / 'solana'), 'airdrop', '5', address, '--url', args.rpc, '--keypair', str(key)], check=True, capture_output=True)
        (root / '.solte/config.toml').write_text(f'selected_profile = 0\nreduced_motion = true\n[[profiles]]\nname = "Confidential local test"\nhttp = "{args.rpc}"\nwebsocket = "{args.ws}"\n')
        def confirmed():
            with sqlite3.connect(root / '.solte/history.sqlite') as db:
                return sum(json.loads(row[0])['message'].startswith('Confirmed:') for row in db.execute('SELECT payload FROM logs'))
        command = [str(args.binary.resolve()), '--project', str(root), '--view', 'tokens', '--reduced-motion']
        with TerminalSession(command, 120, 36, artifact_dir=args.artifact_dir) as terminal:
            terminal.wait_text('Token accounts')
            terminal.wait_text('Updated')
            terminal.send(b'c', 'Create')
            terminal.click_text('Confidential token mint', 'Create confidential token mint')
            terminal.send(b'\t\t\t\t\x15100')
            def sign():
                before = confirmed()
                terminal.send(b'\r', 'Review token operation')
                assert 'SIMULATION FAILED' not in terminal.text
                terminal.send(b'\r')
                terminal.wait_for(lambda: confirmed() > before, 'confirmed confidential operation', timeout=40)
                terminal.wait_for(lambda: 'Waiting for transaction confirmation' not in terminal.text and 'Signing and submitting' not in terminal.text, 'completed operation', timeout=10)
            sign()
            terminal.wait_text('Confidential balances enabled')
            terminal.send(b't', 'Token account inspector')
            terminal.send(b'c', 'Confidential balances')
            terminal.resize(60, 10)
            terminal.resize(120, 36)
            terminal.click_text('Configure account', 'Configure account')
            sign()
            def menu():
                terminal.send(b'3j')
                terminal.send(b'\r', 'Token account inspector')
                terminal.send(b'c', 'Confidential balances')
            menu()
            terminal.click_text('Deposit public tokens', 'Deposit public tokens')
            terminal.send(b'\x1510')
            sign()
            menu()
            terminal.click_text('Reveal balances')
            terminal.wait_text('Available 0')
            terminal.wait_text('Pending 10')
            terminal.click_text('Hide balances')
            terminal.wait_text('Available Locked')
            terminal.click_text('Apply pending balance', 'Apply pending balance')
            sign()
            menu()
            terminal.click_text('Reveal balances')
            terminal.wait_text('Available 10')
            terminal.click_text('Hide balances')
            terminal.click_text('Withdraw to public', 'Withdraw to public')
            terminal.send(b'1')
            sign()
            menu()
            terminal.click_text('Reveal balances')
            terminal.wait_text('Available 9')
            terminal.wait_text('Public 91')
            terminal.resize(60, 10)
            terminal.resize(80, 20)
            terminal.resize(120, 36)
            terminal.escape('Confidential balances')
            terminal.send(b'q')
            terminal.wait_for(lambda: terminal.process.poll() == 0, 'clean exit')
            print('Confidential terminal workflow passed: creation, setup, deposit, reveal/hide, apply, withdrawal, mouse/keyboard and resize')


if __name__ == '__main__':
    main()
