# Installations and releases

Development prereleases are available for macOS and Linux. Use the version-specific installer in the [README](../README.md#install), or [build from source](#build-from-source) for changes that have not been released yet. A stable release has not been published.

## Install a published binary

For the current development prerelease, use the exact version-specific command in the README. After the first stable release, the following command will install without Rust or a repository checkout:

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/abhineet-biju/solte/releases/latest/download/solte-installer.sh | sh
```

The generated asset is named `solte-installer.sh`. It installs into `~/.local/bin` without sudo and prints PATH instructions. Start a new terminal, or load the generated setup file:

```sh
. "${XDG_CONFIG_HOME:-$HOME/.config}/solte/env.sh"
solte --version
cd your-project
solte
```

For a development prerelease, copy its exact version-specific command from its release notes. For example, replace `DEV_TAG` with a published tag such as `v0.1.1-dev.17.1.g1bb23564f7cd`. That example is a local test version, not a published release.

```sh
DEV_TAG='REPLACE_WITH_PUBLISHED_DEVELOPMENT_TAG'
curl --proto '=https' --tlsv1.2 -LsSf "https://github.com/abhineet-biju/solte/releases/download/$DEV_TAG/solte-installer.sh" | sh
```

To choose a directory and manage PATH yourself, pass `SOLTE_INSTALL_DIR` and `SOLTE_NO_MODIFY_PATH=1` to `sh`:

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/abhineet-biju/solte/releases/latest/download/solte-installer.sh |
  SOLTE_INSTALL_DIR="$HOME/bin" SOLTE_NO_MODIFY_PATH=1 sh
```

Installation replaces only the executable and manages its own PATH setup and installation receipt. It does not import keys, run wallet migrations, or modify project wallets and `.solte` data.

## Platforms and direct downloads

Each release includes these archives, individual `.sha256` files, and a combined `sha256.sum`:

| Platform | Archive | Compatibility target |
| --- | --- | --- |
| Apple Silicon | `solte-aarch64-apple-darwin.tar.xz` | macOS 11+ |
| Intel Mac | `solte-x86_64-apple-darwin.tar.xz` | macOS 11+ |
| Linux ARM64 | `solte-aarch64-unknown-linux-gnu.tar.xz` | glibc 2.35+ |
| Linux x86-64 | `solte-x86_64-unknown-linux-gnu.tar.xz` | glibc 2.35+ |

Linux builds use Ubuntu 22.04 and check the binary's required glibc symbols and shared libraries. SQLite is bundled; TLS uses Rust dependencies. Unexpected external libraries fail the packaging check. Alpine/musl and Windows are not supported by these packages.

macOS builds set `MACOSX_DEPLOYMENT_TARGET=11.0` and inspect the resulting load commands and system-library linkage. CI runs on macOS 15, so execution on macOS 11 itself remains unverified. The binaries are not Developer ID signed or notarized. Linux installation tests run on Ubuntu 22.04 for both architectures. These checks establish a compatibility target, not proof of operation on every distribution or older OS release.

Download the archive and its `.sha256` file from the desired [GitHub release](https://github.com/abhineet-biju/solte/releases). In a new temporary directory, verify and extract it. Substitute the appropriate target:

```sh
# macOS; use sha256sum -c instead on Linux
shasum -a 256 -c solte-aarch64-apple-darwin.tar.xz.sha256
tar -xJf solte-aarch64-apple-darwin.tar.xz
mkdir -p "$HOME/.local/bin"
install -m 755 solte-aarch64-apple-darwin/solte "$HOME/.local/bin/solte"
```

The shell installer verifies the archive checksum automatically. GitHub builds also attest the release assets. Once published, verify an archive's provenance with the GitHub CLI:

```sh
gh attestation verify solte-aarch64-apple-darwin.tar.xz --repo abhineet-biju/solte
```

## Update or uninstall

Quit Solte and rerun the installer for the desired release. Development installations use an explicit version URL; the stable URL always selects GitHub's latest stable release. There is no built-in updater or Homebrew distribution.

To uninstall a default installation, remove `~/.local/bin/solte`. If you used a custom directory, remove that executable instead. You may also remove the installer's `solte-receipt.json` and `env.sh`/`env.fish` files under `${XDG_CONFIG_HOME:-$HOME/.config}/solte`, then remove their PATH setup lines from your shell profile. Inspect those files before removing anything else. Keep project `.solte` directories and all keyfiles; they contain your wallet data.

## Publish a development build

1. Push your reviewed commits to `main`. Ordinary branch pushes and pull requests run checks and packaging tests without publishing releases.
2. Open the repository's **Actions** tab and select **Release binaries**.
3. Click **Run workflow**, select the **main** branch, and click the green **Run workflow** button. Manual runs from other branches are skipped.
4. The workflow checks, packages, and tests the exact commit selected when the run was dispatched. Only after every job succeeds does it publish a development prerelease.
5. Open **Releases** and copy the version-specific installer command from the new prerelease's notes.

The manual button becomes available after this workflow is pushed to the default branch. No version bump is needed for development releases. With an authenticated GitHub CLI, the equivalent explicit command is:

```sh
gh workflow run release.yml --ref main --repo abhineet-biju/solte
```

There is no release schedule to maintain. Publish a prerelease when you have a useful set of changes for testers.

For a committed package version of `0.1.0`, development versions have the form `0.1.1-dev.RUN.ATTEMPT.gSHA12`. Run and attempt numbers make every build unique, including reruns. The full source commit is recorded in release notes and `release-build.json`.

Only the root package version in `Cargo.toml` and `Cargo.lock` is stamped inside disposable build checkouts. Dependencies stay locked. No version change is committed back to `main`. GitHub's automatic source archive refers to the original commit, so reproducing a development binary also requires the recorded version stamp.

Development releases explicitly use `prerelease: true` and `make_latest: false`. They never replace the latest stable release. The same workflow builds and publishes them directly; it does not depend on a workflow being triggered by a tag created with `GITHUB_TOKEN`.

## Publish a stable release deliberately

Stable publication requires an exact `vMAJOR.MINOR.PATCH` tag matching the committed Cargo package version. A push to `main` alone runs checks and never publishes a release.

For a first stable `v0.2.0`:

1. Change the package version in `Cargo.toml` to `0.2.0`. Run `cargo check` to update the root entry in `Cargo.lock`, then review that dependencies did not change.
2. Run the checks below, commit both version files, and push the reviewed commit to `main`. This push runs checks without publishing.
3. Deliberately create and push the stable tag on that commit:

```sh
git tag -a v0.2.0 -m "Release v0.2.0"
git push origin v0.2.0
```

The workflow reruns checks and packaging from the tagged commit. Mismatched tags fail before publication. A successful stable release becomes eligible for GitHub's latest stable selection. After verifying the installer from that published release, make the stable install command the primary method in the README.

All publication happens in a final job with `contents: write`, `id-token: write`, and `attestations: write`. Other jobs have read-only repository access. It uses the built-in `GITHUB_TOKEN`; no PAT or repository secret is required. GitHub Actions, the listed hosted runners, and these token permissions must be allowed by the repository or organization. No repository settings are changed by this setup.

Assets are uploaded to a draft before publication. Existing releases and drafts are never overwritten automatically. If publication fails after draft creation, inspect the failed run and draft before retrying. A development rerun creates a new unique version; retrying a stable release with an existing draft requires explicit maintainer cleanup or recovery.

## Build from source

Install Rust through rustup and clone the repository. The committed toolchain file selects Rust 1.97.1. Linux source builds also need a C/C++ compiler and standard build tools; macOS needs Xcode Command Line Tools.

```sh
git clone https://github.com/abhineet-biju/solte.git
cd solte
cargo install --path . --locked
solte --demo --theme neon
```

Cargo normally installs into `~/.cargo/bin`; follow rustup's PATH setup instructions. Run `solte` from any project to discover its wallets.

## Validate packaging locally

Python 3.11+ is needed for release scripts; CI uses 3.13. Run application and release checks:

```sh
python3 -m venv .venv
.venv/bin/python -m pip install --require-hashes -r scripts/requirements-test.txt
. .venv/bin/activate
cargo fmt --check
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
cargo build --locked --bin solte
python3 scripts/terminal_smoke.py --artifact-dir target/terminal-smoke/wallet
python3 scripts/token_smoke.py --artifact-dir target/terminal-smoke/tokens
python3 -m unittest discover -s scripts/tests -v
```

Use a clean disposable checkout for development stamping. `prepare.py` validates the full source SHA and matching stable tag, and `--channel development` stamps a temporary version. Never stamp your working checkout merely to publish a development build.

For local native packaging from the committed version, install the checksum-pinned cargo-dist 0.33.0 into a temporary directory, then plan and build. This example targets Apple Silicon; substitute your native target and current package tag:

```sh
DIST_TOOLS="$(mktemp -d)"
python3 scripts/release/bootstrap_dist.py --directory "$DIST_TOOLS"
export CARGO="$PWD/scripts/release/cargo-locked.sh"
export MACOSX_DEPLOYMENT_TARGET=11.0
"$DIST_TOOLS/dist" plan --tag v0.1.0
"$DIST_TOOLS/dist" build --artifacts=local --target aarch64-apple-darwin --tag v0.1.0 --output-format=json > local-dist-manifest.json
cp local-dist-manifest.json target/distrib/aarch64-apple-darwin-dist-manifest.json
"$DIST_TOOLS/dist" build --artifacts=global --tag v0.1.0
python3 scripts/release/test_install.py --target aarch64-apple-darwin --version 0.1.0
```

The Cargo shim adds `--locked`, which cargo-dist 0.33.0 does not add itself. cargo-dist owns archive and installer generation; authored reusable workflows handle channel policy and native installation tests. Its generated CI is disabled to keep one workflow path for both channels.

The installer test uses a loopback HTTP mirror, a temporary home, and sentinel project data. It checks replacement, reinstallation, custom paths, checksum rejection, installed version, demo rendering, keyboard/mouse terminal smoke, and preservation of wallets/configuration. It runs the binary on the host architecture. The packaging workflow repeats this on all four native runners and combines the resulting checksums into one installer.

Every push and pull request runs Rust tests, Clippy, Python tooling tests, and both offline terminal smoke tests on Linux and macOS. Formatting runs once on Linux. The smoke scripts share a PTY harness and wait for the current rendered screen. Failures save the visible screen and terminal output as CI artifacts, including failures against packaged binaries.

Ordinary CI skips the four-platform packaging pipeline when only `README.md`, `AGENTS.md`, the release or usage guide, or verification reports change. Other paths, including the packaged key-handling guide, trigger the complete packaging and installation checks. Pushes compare against the branch's last successful CI run so failed or cancelled code changes remain covered; pull requests compare against their base commit. An unavailable comparison triggers all packaging checks. Superseded CI runs are cancelled. Tagged and manual releases always run the complete checks, native installation matrix, and final installer assembly before publication.

Local packaging checks do not exercise GitHub publication, artifact attestations, the latest-release redirect, or downloads from a published release.
