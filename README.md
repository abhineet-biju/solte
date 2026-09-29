# Solte

Solte is a developer-first, TUI-based Solana wallet built for the terminal.

> **Development use only.** Solte has not been thoroughly security-tested or independently audited for use with real mainnet funds. It is intended for development and testing. Do not use it to store or manage real funds.

![Solte in the Neon theme with demo data](docs/preview.png)

## Built for developers

- *Project-aware wallets*: Discover existing Solana and Anchor keypairs when you run `solte` in a project.
- *Multiple test identities*: Create, import, and switch wallets. You can also inspect/copy any wallet.
- *Development networks*: Use Devnet, Localnet, or custom RPC profiles, with funding options available directly in the terminal.
- *Legacy, v0, and v1 support*: Choose a transfer format or import an existing transaction with automatic format detection and lookup-table resolution.
- *Review before signing*: Inspect simulation errors, fees, resource limits, accounts, and instructions before submitting.
- *Partial signing*: Preserve co-signatures and export signed transactions without broadcasting them.
- *Useful diagnostics*: Search captured history and failures. Inspect logs, copy signatures, and open Explorer directly from the wallet.
- *Live network context*: Watch RPC health, latency, slots, and wallet activity alongside your work.
- You can also navigate with Vim keys, arrows, or clicks, with responsive layouts and built-in themes.

## Install

Prebuilt installers are pending the first release. For now, [build from source](docs/releases.md#build-from-source), then run `solte` in your project. macOS and Linux supported.

Try `solte --demo --theme neon` without using real keys.

[Usage](docs/usage.md) · [Installation and releases](docs/releases.md) · [Key handling](docs/key-handling.md) · [MIT license](LICENSE)
