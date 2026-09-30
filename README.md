# Solte

Solte is a terminal-based Solana wallet built for developers.

> **Development use only.** Solte has not been thoroughly security-tested or independently audited for use with real mainnet funds. It is intended for development and testing. Do not use it to store or manage real funds.

![Solte in the Neon theme with demo data](docs/preview.png)

## Built for developers

- *Project-aware wallets*: Discover existing Solana and Anchor keypairs when you run `solte` in a project.
- *Multiple test identities*: Create, import, and switch wallets. You can also inspect/copy any wallet.
- *Development networks*: Use Devnet, Localnet, or custom RPC profiles, with funding options available directly in the terminal.
- *Token account tools*: Inspect SPL Token and Token-2022 accounts, give mints and accounts local names, copy/export public details, create associated accounts, and review token transfers.
- *Confidential test transfers*: Discover confidential Token-2022 accounts, configure test wallets, and deposit, send, or withdraw tokens with encrypted balances.
- *Test token minting*: Create SPL Token or Token-2022 mints, track them in your project, and issue tokens to test wallets.
- *Legacy, v0, and v1 support*: Choose a transfer format or import an existing transaction with automatic format detection and lookup-table resolution.
- *Review before signing*: Inspect simulation errors, fees, resource limits, accounts, and instructions before submitting.
- *Partial signing*: Preserve co-signatures and export signed transactions without broadcasting them.
- *Useful diagnostics*: Search captured history and failures. Inspect logs, copy signatures, and open Explorer directly from the wallet.
- *Live network context*: Watch RPC health, latency, slots, and wallet activity alongside your work.
- You can also navigate with Vim keys, arrows, or clicks, with responsive layouts and built-in themes.

## Install

Prebuilt binaries are available for macOS and Linux (prerelease).

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/abhineet-biju/solte/releases/download/v0.1.1-dev.3.1.gb206996f0152/solte-installer.sh | sh
```

Run `solte` in your project.

Try `solte --demo --theme neon` without using real keys.

[Usage](docs/usage.md) · [Installation and releases](docs/releases.md) · [Key handling](docs/key-handling.md) · [MIT license](LICENSE)
