# Solte usage

A terminal wallet for Solana development. Manage project identities, fund wallets, inspect transactions, and keep network diagnostics beside your editor.

The preview uses demo accounts and transactions. Try it with `solte --demo --theme neon`.

## Run

Requires Rust 1.97.1 or newer and a terminal with mouse support. Solte targets macOS and Linux. A C toolchain is needed when building the bundled SQLite dependency from source.

```sh
cargo build --release --locked
./target/release/solte --project /path/to/your/solana-project
```

To install the executable in your Cargo bin directory:

```sh
cargo install --path . --locked
cd /path/to/your/solana-project
solte
```

Devnet is the default. Press `p` or click **RPC profiles** to switch to Localnet or add a custom HTTP and WebSocket endpoint. Mainnet can be inspected, but funding and signing are disabled based on the endpoint's genesis hash.

Useful launch options:

```sh
solte --demo                     # Explore labeled fixtures; no signing or network calls
solte --offline                  # Browse captured local records
solte --profile Localnet         # Select a saved profile
solte --check                    # Read-only RPC diagnostic
solte --reduced-motion           # Disable panel transitions
solte --demo --snapshot view.svg # Render the actual terminal buffer to SVG
```

## Wallets and transactions

Solte discovers the nearest project through `.solte/config.toml`, `Anchor.toml`, or `.git`. It reads the Anchor provider wallet and valid keypair JSON files in the project root, `keys/`, `wallets/`, `.solte/keys/`, and `target/deploy/`. Program keypairs in `target/deploy/` appear as read-only identities.

Create a wallet with `n` or **New identity**. New keys use standard Solana JSON in `.solte/keys/`; existing names are never overwritten. Imported keys stay at their original paths. Both remain usable with Anchor and the Solana CLI.

In **Wallets**, moving the selection previews an identity; clicking it or pressing Enter opens its details at any supported terminal size. Use **Copy address [y]** without changing the active wallet. **Use wallet [Enter]** explicitly activates the inspected identity. The `y` shortcut in the Wallets view also copies the highlighted identity directly. Overview wallet shortcuts and `]` still provide quick activation.

Transaction colors describe the selected wallet's SOL balance change: **Received** is green, **Sent** is red, and failed transactions are red with a failure marker. Fee-only and other program activity use the theme's heading color. The inspector retains the underlying instruction details. Search accepts both instruction names and the displayed action labels.

Logs use red for errors, the theme accent for warnings, neutral text for informational entries, and muted text for debug/trace. The Logs view shows separate entries, newest first. Navigate with `j`/`k`, arrows, Page Up/Down, or the mouse wheel; click an entry or press Enter for its full text. Inspecting or navigating logs pauses following to keep the selection stable; `F` resumes at the newest entry. Log details offer **Copy log [y]** and, when a full transaction signature is present, **Signature [s]** and **Explorer [o]**. If a message contains several signatures, those actions use the first valid signature; copying the full log preserves them all.

On Devnet, `f` opens a funding menu: the official Solana faucet with your public address prefilled, Quicknode with address copying, or a direct request through the current RPC. Web requests require you to complete the faucet's verification in your browser. Solte refreshes on terminal focus and continues monitoring your balance. A rejected direct Devnet request offers the browser choices again. Localnet uses direct RPC funding.

Use `s` to enter a recipient and amount, inspect an unsigned simulation, then explicitly sign and submit. The review shows the network, amount, fee, compute usage, and available program logs. A failed simulation cannot be submitted. When confirmation is uncertain, Solte keeps the signature so you can check it before retrying.

Click a transaction row or press Enter to inspect its error, logs, instructions, inner instructions, and token balance metadata. Explorer and copy actions are available by mouse and keyboard. Copy uses the terminal's OSC 52 clipboard support. Custom RPC URLs that may contain credentials are not sent to an external explorer.

## Navigation

| Action | Keys |
|---|---|
| Move focus within the current view | Tab / Shift-Tab |
| Overview / Wallets / Transactions / Network / Logs | 1 / 2 / 3 / 4 / 5 |
| Previous / next main view | Left / Right arrows |
| Navigate options within the focused panel | h / j / k / l, Up / Down arrows |
| Scroll the current list or view | Page Up / Page Down |
| Activate the focused option | Enter |
| Create / import / cycle identity | n / i / ] |
| Funding options / send SOL | f / s |
| RPC profiles / refresh | p / r or R |
| Search current Transactions or Logs view / transaction failures | / / e |
| Fetch or load older records | b |
| Open transaction explorer / copy wallet address | o / y |
| Clear transaction search | x |
| Follow logs / clear visible logs | F / C |
| Choose theme / motion | t / m |
| Open focused summary / return to Overview | z |
| Help / close dialog / quit | ? / Escape / q |

The top bar switches between distinct views. Overview combines wallet, activity, network, and log summaries; Wallets, Transactions, Network, and Logs use the workspace for their own content. Use the main tabs or z to visit a summary’s full view. At narrower widths Overview reduces the number of summaries while all five views remain accessible. The minimum size is 60 columns by 10 rows.

The highlighted control has keyboard focus. Left/right arrows switch main views directly. Use k/up at the top of a page to reach the view selector, h/l to switch views there, and j/down or Enter to enter the page. In dialogs, left/right do not change the underlying view; in text fields they move the cursor. Tab and Shift-Tab move between visible focus regions without changing views. Clicks and Enter invoke the same actions. Forms accept normal text input, arrow keys move the cursor, and Tab switches fields. Each view preserves its navigation state; Transactions and Logs have separate search filters.

Use `solte --view network` to open a particular view. `--view` also works with `--demo` and `--snapshot` for reproducible previews.

## Local data and monitoring

Solte keeps project data in an ignored `.solte/` directory:

- `config.toml`: identity labels and paths, network profiles, theme, and motion preference.
- `keys/`: newly generated standard keypair files.
- `history.sqlite`: captured transactions, per-account paging cursors, and operation logs.
- `diagnostics.log`: internal application diagnostics.

New keyfiles and configuration files use owner-only permissions on macOS/Linux. Keyfiles and custom RPC credentials are plaintext local files; the first release does not include an encrypted vault. Private keys are not stored in the history database.

Wallet log subscriptions provide live notifications. HTTP refreshes recover available history every 15 seconds and poll up to 32 discovered classic SPL and Token-2022 accounts. The interface reports incomplete coverage. Failed simulations performed by Solte are retained in the operation log.

History is separated by endpoint, wallet, and genesis hash. The interface initially loads 1,000 captured records; **Older history** expands the window in 250-record steps up to 10,000 and fetches older RPC pages when needed. Additional captured records remain in SQLite. Recent operation logs are bounded to 1,000 entries per wallet/endpoint.

Current limits:

- Monitoring runs while Solte is open. Reopening recovers data the RPC still serves; it cannot reconstruct pruned history or historical closed token accounts it never observed.
- Transactions submitted by another application are visible if they reach the chain and involve a watched address. Rejected approvals and preflight failures from other applications require a future integration.
- Transaction reading and inspection support legacy, v0, and v1. Unavailable metadata and RPC errors remain explicit in the inspector.
- The built-in composer sends native SOL in legacy, v0, or v1 format. Imported transactions can contain other program instructions. Dedicated token tools, browser pairing, and hardware wallets are outside this release.
- Devnet airdrops depend on faucet availability and rate limits. Custom development chains must expose compatible Solana RPC methods.

## Development and verification

```sh
cargo fmt --check
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
cargo build --locked
python3 scripts/terminal_smoke.py
```

The regular tests cover exact amount parsing, file protection, project discovery, history isolation and paging, layout bounds, input handling, modal mouse isolation, and mocked RPC behavior. The terminal smoke test creates disposable wallets through both keyboard and mouse input.

To verify real funding and transfers, run an isolated local validator and provide its endpoints:

```sh
SOLTE_TEST_RPC=http://127.0.0.1:8899 \
SOLTE_TEST_WS=ws://127.0.0.1:8900 \
cargo test --test localnet -- --ignored

python3 scripts/terminal_smoke.py \
  --local-rpc http://127.0.0.1:8899 \
  --local-ws ws://127.0.0.1:8900
```

These checks only accept loopback RPC endpoints and create disposable keypairs. The terminal test also requires `solana-keygen` on PATH. The CI workflow runs the regular checks and offline terminal smoke test on macOS and Linux.

The source is organized around wallet operations, network monitoring, local storage, application state, rendering, and runtime coordination. Rendering does no network or disk I/O. A session identifier prevents stale responses from updating a newly selected wallet or profile.

Transaction inspectors support Home/End and bounded scrolling with a position indicator. Escape closes a dialog without changing the underlying view. In offline mode, funding and sending explain their unavailability before opening a form.

Interaction styling is shared across all themes: a solid accent background marks the focused control or editable field; muted backgrounds and accent text indicate selection without focus. Wallet activity, errors, and connection health retain explicit symbols or labels. Moving focus never changes the active wallet. In Wallets, opening details also preserves the active wallet until Use wallet is confirmed. Dialogs own focus while open, and closing them restores the underlying view and focus. In forms, the highlighted field receives typing; Enter submits the form.

Overview retains all four summaries from 80 columns by 20 rows, keeping Wallets on the left and Network on the right. Narrower sidebars and wrapped wallet actions preserve the same arrangement when zooming. Smaller windows prioritize a usable summary and keep every full view accessible. Transactions is abbreviated to Txns in the narrowest tab bar; `--view transactions` also accepts the previous `activity` spelling.

See [key-handling practices](key-handling.md) for memory lifetime, file permissions, and the limits of development-key protection.

Manual refresh displays a spinner while pending and a success check or failure indicator for four seconds after completion. Reduced motion keeps progress text static. Offline refresh reloads cached history; demo data does not issue RPC requests. Repeat refresh presses are coalesced while a request is pending.

Appearance includes Ember, Glacier, Orchid, and Neon. Neon uses a navy background, cyan headings, magenta focus, and mint balances. Choose it with `t` or launch with `solte --theme neon`. Theme selection changes colors only.

Terminal font zoom is handled as a change in available character columns and rows. Layouts reflow at their breakpoints, preserve view/selection state, and rebuild click targets. Below 60 × 10, Solte shows a size hint and recovers when enlarged; it cannot fit the complete interface below that minimum.

Shortcut hints use square brackets consistently, including tab numbers, buttons, dialog close labels, status messages, and Help. The header uses a three-line block wordmark when space permits and a compact spaced wordmark in shorter windows; both use the terminal’s existing font.

## Transaction formats and imports

The Send form has a Format choice with Auto, Legacy, v0, and v1. Use the left/right arrows or click the field arrows. Auto chooses legacy for native SOL transfers. Explicit formats never silently fall back. Newly built v1 transfers set resource limits, simulate, adjust the limits, and simulate the final message again. The review shows the actual format, fees, resource configuration, accounts, instructions, and simulation logs.

Press `I` or click **Import tx [I]** in Transactions to open a raw wire transaction or base64 file. Solte detects the format, resolves v0 lookup tables, validates existing signatures, and simulates the original message. Choose **Sign & export** to save a base64 transaction without submitting; choose **Sign & submit** to broadcast after review. Export is the default and never overwrites an existing file. The selected wallet must be a required signer; other valid signatures are preserved.

Signing does not change the original message, blockhash, lookup references, or format. All required signatures must be present before submission. An expired blockhash requires rebuilding and collecting signatures externally. Imported durable-nonce transactions are currently rejected explicitly. Imports require online simulation, including sign-and-export; this is not an offline signing mode. Mainnet signing remains disabled.

V1 requires support on the selected RPC and validator. Unsupported-format errors are shown without attempting a different format. Use an Agave 4.3+ local validator for the complete integration suite.

```sh
SOLTE_TEST_RPC=http://127.0.0.1:18899 \
SOLTE_TEST_WS=ws://127.0.0.1:18900 \
cargo test --test localnet -- --ignored

python3 scripts/terminal_smoke.py --binary target/release/solte --format v1 \
  --local-rpc http://127.0.0.1:18899 --local-ws ws://127.0.0.1:18900
```
