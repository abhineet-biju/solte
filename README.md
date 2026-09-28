# Solte

A terminal wallet for Solana development. Project wallets, Devnet funding, transaction inspection, and network diagnostics in one mouse- and keyboard-accessible interface.

Solte targets macOS and Linux. It uses standard Solana keypair files and defaults to Devnet. Localnet and custom development RPC profiles are supported; mainnet signing is outside the first release.

## Development

```sh
cargo run
cargo test
cargo fmt --check
cargo clippy --all-targets -- -D warnings
```

The application is being built in incremental commits. See the command-line help for available options.

