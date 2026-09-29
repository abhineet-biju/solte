# Development key handling

Solte uses standard unencrypted Solana keypair JSON files. It is intended for development keys, not as a mainnet custody wallet.

- Persistent application state contains wallet names, public addresses, and file paths. It does not retain signing keys.
- Reads are capped at 4 KiB. File contents and decoded key bytes use `zeroize::Zeroizing`, including error paths. Parsing fills a guarded fixed-size array rather than an intermediate secret vector.
- Key creation serializes guarded bytes directly into a new file. It does not create an additional JSON string containing the secret. Existing files are never overwritten; new key files use Unix mode `0600` and the private directory uses `0700`.
- The pinned Solana keypair implementation wraps `ed25519-dalek::SigningKey`. Its enabled `zeroize` feature wipes the stored secret when the signing key drops. The signer is scoped to the signing call and is dropped before submission or confirmation awaits.
- Mint creation holds a temporary mint keypair through simulation and review. Closing the review drops and zeroizes it. After approval it signs the reviewed message, then drops before submission or confirmation awaits. Only public mint addresses and creation signatures enter project storage. Loaded authority wallets are reread and verified at signing time.
- Signing rejects changed key files and read-only program identities. Key parsing errors do not echo file contents.

These measures reduce secret lifetime in application memory. They do not guarantee removal of every compiler, allocator, operating-system, or cryptographic-library temporary copy. Standard key files remain unencrypted on disk. Imported files retain their existing permissions. Solte does not add memory locking, an encrypted vault, or password prompts.
