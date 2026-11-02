# Developer Guide

## Building

```bash
cargo build            # debug binary at target/debug/awsctl
cargo build --release  # release binary at target/release/awsctl
```

The locked dependencies need a recent stable Rust toolchain; the AWS SDK crates
in `Cargo.lock` currently require 1.94.1 or newer.

## Releasing

Releases go through release-plz (`.github/workflows/release-plz.yml`). Every
push to `main` opens or updates a release PR that bumps the version in
`Cargo.toml` and adds the changes since the last tag to `CHANGELOG.md`. The
bump follows the conventional commit prefixes: `fix:` gives a patch release,
`feat:` a minor one, and a breaking change a major one. Edit the PR to change
the version or the changelog.

Merging the release PR pushes the version tag (for example `v1.2.3`).
`.github/workflows/release.yml` then builds the macOS arm64 and Linux x86_64
binaries and publishes a GitHub Release with both archives attached. Nothing is
published to crates.io.

The workflow needs a `RELEASE_PLZ_TOKEN` secret: a fine-grained personal access
token for this repository with Contents and Pull requests set to read and write.
A tag pushed with the default `GITHUB_TOKEN` would not start `release.yml`.

### Homebrew tap (optional)

The release workflow can also render `packaging/homebrew/awsctl.rb` and push it
to a Homebrew tap. That job is skipped unless the repository defines:

- variable `HOMEBREW_TAP_REPO` — the tap repository's name, under the same owner
  as this repository (for example `homebrew-tap`)
- variable `APP_ID` and secret `PRIVATE_KEY` — a GitHub App installed on that
  tap with `contents: write`

Without them the release still publishes; only the tap update is skipped.
