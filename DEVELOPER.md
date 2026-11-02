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

Merging the release PR publishes the crate to crates.io and pushes the version
tag (for example `v1.2.3`). The tag starts `.github/workflows/release.yml`,
which [dist](https://github.com/axodotdev/cargo-dist) generates from
`dist-workspace.toml`. It builds archives for macOS (arm64 and x86_64) and
Linux (x86_64 and arm64), creates the GitHub Release with them and a shell
installer attached, and pushes the Homebrew formula to `Souldiv/homebrew-tap`.

Do not edit `release.yml` by hand. Change `dist-workspace.toml`, then run
`dist generate` with the dist version it names. `dist plan` lists what a
release would build, and `dist build` builds the archive for your own machine.

The release path needs three secrets:

- `RELEASE_PLZ_TOKEN`: a fine-grained personal access token for this repository
  with Contents and Pull requests set to read and write. A tag pushed with the
  default `GITHUB_TOKEN` would not start `release.yml`.
- `CARGO_REGISTRY_TOKEN`: a crates.io API token with the `publish-update` scope.
- `HOMEBREW_TAP_TOKEN`: a fine-grained personal access token with Contents set
  to read and write on `Souldiv/homebrew-tap`. Without it the release still
  publishes; only the formula update fails.
