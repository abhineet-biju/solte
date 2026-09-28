# Verification pass, 28 September 2026

The tested workflows passed after fixing stale network telemetry. This is a macOS verification of the current release, not a guarantee that every terminal, RPC provider, or network condition behaves identically.

## Finding and fix

Stopping the local validator made Connection show Offline, but the Network view continued to show its previously cached Healthy status, latency, and slot. Network panels now suppress those measurements while disconnected. The verified network identity remains in application state for signing guards. The updated release was exercised against an unavailable endpoint and displayed dashes and a failed-refresh indicator correctly.

## Results

| Area | Verification | Result |
| --- | --- | --- |
| Build | Formatting, locked Rust tests, Clippy with warnings denied, release build | Passed; 57 unit tests and 4 mocked RPC tests |
| Real transactions | Isolated validator integration test and terminal funding, unsigned simulation, explicit submission, recipient balance, mouse inspection | Passed |
| Transfer rejection | Invalid recipient in TUI; insufficient funds in TUI; Enter on failed review; mocked expired review and changed network | Passed; invalid reviews did not submit |
| Mainnet protection | Genesis-based rejection even with a Devnet profile name | Passed in mocked RPC test |
| Wallets | Keyboard and mouse creation, invalid name, duplicate name, missing import path, valid external import, selection persistence | Passed; source import file retained |
| Project discovery and key files | Existing discovery/keyfile tests, owner-only generated key/config permissions, unchanged-key requirement before signing | Passed |
| RPC profiles | Invalid URL, custom profile creation, duplicate name, saved profile selection, CLI diagnostic | Passed |
| Monitoring | WebSocket Live state, external CLI transactions, classic SPL and Token-2022 account discovery, explicit refresh | Passed; both token account types were detected |
| Network failure | Stop validator, refresh failure, restart same ledger, recovery of HTTP state and WebSocket monitoring | Passed; stale display issue fixed |
| History | Real transaction details and failed-simulation logs after reopening; paging, exhaustion, scope isolation, and cursor persistence tests | Passed |
| Transactions | Search, errors filter, mouse inspector, metadata, Home/End, inspector resizing | Passed |
| Logs | Independent search, pause/resume, clear, chronological cache merge, frozen paused entries | Passed through TUI and regression tests |
| Offline mode | Funding/sending rejection, cached refresh completion, creation/import, settings | Passed |
| Clipboard | OSC 52 request emitted by the actual terminal process | Passed; host clipboard acceptance remains terminal-dependent |
| Explorer and faucets | Cluster-aware URL generation, credential exclusion, official faucet public-address parameter, rejected-airdrop recovery | Passed in code tests; external browser fulfillment not exercised |
| Navigation | View shortcuts, arrow/Vim behavior, focus routing, keyboard/mouse action equivalence, modal isolation | Passed in regression suite and terminal flows |
| Appearance | Theme/motion dialogs and persistence; 100 demo snapshots covering all 4 themes, all 5 views, and 5 sizes | Passed |
| Resizing | Workspace and dialogs through compact, wide, and below-minimum sizes; no cursor-query race or crash | Passed |
| CLI | Help/version, invalid theme/profile/dimensions, offline/demo diagnostic rejection, local RPC diagnostic | Passed |
| Demo | Snapshot matrix and absence of project-state writes | Passed |

The real-validator integration test adds one passing test beyond the 61 standard tests. Both offline and local-validator terminal smoke runs passed. A clipboard-emission assertion was added and verified in the offline smoke run. The local transfer smoke run preceded the display-only disconnect fix; the final release's disconnected display was then checked interactively.

## Reproduction

```sh
cargo fmt --check
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
cargo build --release --locked
python3 scripts/terminal_smoke.py --binary target/release/solte
```

With an isolated validator on HTTP 18899 and WebSocket 18900:

```sh
SOLTE_TEST_RPC=http://127.0.0.1:18899 \
SOLTE_TEST_WS=ws://127.0.0.1:18900 \
cargo test --test localnet -- --ignored --nocapture

python3 scripts/terminal_smoke.py --binary target/release/solte \
  --local-rpc http://127.0.0.1:18899 \
  --local-ws ws://127.0.0.1:18900
```

## Limits

- No live Devnet faucet request, CAPTCHA, browser approval, or external explorer page was completed. URL construction and recovery behavior were tested; provider availability is outside this result.
- Terminal interaction used a real pseudo-terminal and screen parser on macOS. Native GUI font zoom, terminal-specific clipboard permissions, and Linux were not independently exercised in this pass. CI is configured for macOS and Linux; its remote results were not checked here.
- Token monitoring was tested with classic SPL and Token-2022 account creation. Every token extension and third-party program instruction was not exercised.
- Transaction confirmation timeouts, severe packet loss, and unavailable transaction metadata are not exhaustively reproduced by this pass.
- This is a functional verification, not a formal security audit. Existing keyfile validation and memory-lifetime safeguards were retained.

Disposable test wallets and a loopback validator were used. No user wallet was funded or used to sign. All test terminal processes and the validator were stopped after testing.
