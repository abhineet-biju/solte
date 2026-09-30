# Confidential workflow verification, 30 September 2026

The confidential lifecycle and existing wallet workflows passed on macOS. Tests used disposable wallets and an isolated loopback validator; no user keypair or public network funds were used.

## Results

| Area | Evidence | Result |
| --- | --- | --- |
| Standard checks | Formatting, 99 regular Rust tests, Clippy with warnings denied, optimized build | Passed |
| Existing native flows | Three SOL/transaction-format/import tests and two token/mint lifecycle tests | Passed |
| Confidential native flows | Two validator tests covering manual approval with an auditor and automatic approval without one | Passed |
| Confidential state | Public/pending/available balances, deposit, apply, send, recipient apply, withdrawal | Passed |
| Review protection | Unsigned prepared messages, stale account rejection before submission, wrong-owner decryption, reconfiguration rejection | Passed |
| Mainnet protection | Confidential requests rejected by genesis before account reads or key loading | Passed |
| UI | Four themes, compact/wide form and menu layouts, keyboard/mouse action equivalence, reveal/hide, resize | Passed |
| Terminal workflows | Existing wallet and token smoke scripts plus the new confidential PTY walkthrough | Passed against the optimized binary |
| Release/test harness | Eight release tests and three terminal-session tests | Passed |

The native confidential test retained ordinary public transfers on the same mint. The PTY walkthrough created an automatically approved mint, configured the account, deposited ten tokens, revealed and hid the balances, applied the pending credits, and withdrew one token. It checked the resulting public balance of 91 and confidential available balance of 9.

## Environment

- Agave 4.3.0, Apple Silicon.
- Published Token-2022 11.1.0 program loaded into the disposable validator with `--bpf-program`.
- Upstream `solana-zk-sdk` 7.0.1 and confidential proof generation/extraction 0.6.1, pinned with the committed lockfile.
- Inline proofs in atomic v1 sends/withdrawals; Legacy account setup, approval, deposit and apply.

The validator's default bundled Token-2022 program accepted configuration but rejected deposits with `InvalidInstructionData`. Loading the published current program enabled the full lifecycle. Applications must validate the selected program/runtime capability; a validator version alone is insufficient.

The downloaded program's SHA-256 was `3dde54d202caa2d10514fd58a154df5f26f07643f9f5fd0402ff08084f0cb954`, checked against its GitHub release-asset digest. The Agave archive's SHA-256 was `0bfbd769a55e32f0a1fe1b92f76f360f01ee19475facf8c31cb90f99dcca7fe0`.

## Reproduction

```sh
cargo fmt --check
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
cargo build --release --locked

SOLTE_TEST_RPC=http://127.0.0.1:19899 SOLTE_TEST_WS=ws://127.0.0.1:19900 \
  cargo test --locked --test confidential_localnet --test token_localnet --test localnet -- --ignored

python3 scripts/terminal_smoke.py --binary target/release/solte
python3 scripts/token_smoke.py --binary target/release/solte
python3 scripts/confidential_smoke.py --binary target/release/solte \
  --solana-tools /path/to/solana-release/bin
python3 -m unittest discover -s scripts/tests -v
cargo run --locked --example confidential_preview
```

The preview example writes actual TUI buffer snapshots into a temporary directory. Pass an output directory to save them elsewhere.

## Limits

Live Devnet execution, Linux and Intel macOS execution, legacy/v0 confidential send plans, confidential fees, confidential mint/burn and custom-key import were not tested as supported workflows. There is no claim of a formal cryptographic or wallet security audit. Full available-balance recovery after concurrent apply races is limited to amounts the SDK can decrypt as a 32-bit value; larger recovery failures are explicit and require a compatible client.
