# Solte

Solte is a developer-first, TUI-based Solana wallet built for the terminal.

![Solte in the Neon theme with demo data](docs/preview.png)

## Built for developers

- **Project-aware wallets.** Discover existing Solana and Anchor keypairs when you run `solte` in a project.
- **Multiple test identities.** Create, import, and switch wallets; inspect and copy an inactive wallet without switching.
- **Development networks.** Use Devnet, Localnet, or custom RPC profiles, with funding options inside the terminal.
- **Legacy, v0, and v1.** Choose a transfer format or import an existing transaction with automatic format detection and lookup-table resolution.
- **Review before signing.** Inspect simulation errors, fees, resource limits, accounts, and instructions before submitting.
- **Partial signing.** Preserve co-signatures and export signed transactions without broadcasting them.
- **Useful diagnostics.** Search captured history and failures; inspect logs, copy signatures, and open Explorer.
- **Live network context.** Watch RPC health, latency, slots, and wallet activity alongside your work.
- **Keyboard and mouse.** Navigate with Vim keys, arrows, or clicks, with responsive layouts and built-in themes.

## Run

```sh
cargo install --path . --locked
cd your-project
solte
```

macOS and Linux. Requires Rust 1.97.1+. Try `solte --demo --theme neon` without using real keys.

[Usage and transaction support](docs/usage.md) · [Key-handling practices](docs/key-handling.md)
