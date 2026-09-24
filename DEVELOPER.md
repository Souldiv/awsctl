# Developer Guide

## Building

```bash
cargo build            # debug binary at target/debug/awsctl
cargo build --release  # release binary at target/release/awsctl
```

The locked dependencies need a recent stable Rust toolchain; the AWS SDK crates
in `Cargo.lock` currently require 1.94.1 or newer.

## Releasing

Releases are tag-driven: the tag is the source of truth for the version, so no
version-bump commit is needed. Create a version tag and push it.

```bash
git tag -a v1.0.6 -m "Release v1.0.6"
git push origin v1.0.6
```

`.github/workflows/release.yml` then syncs `Cargo.toml` to the tag, builds the
macOS arm64 and Linux x86_64 binaries, and publishes a GitHub Release with both
archives attached. Release notes are generated from the commits since the
previous tag, which is why this repository keeps no hand-maintained changelog.

### Homebrew tap (optional)

The release workflow can also render `packaging/homebrew/awsctl.rb` and push it
to a Homebrew tap. That job is skipped unless the repository defines:

- variable `HOMEBREW_TAP_REPO` — the tap repository's name, under the same owner
  as this repository (for example `homebrew-tap`)
- variable `APP_ID` and secret `PRIVATE_KEY` — a GitHub App installed on that
  tap with `contents: write`

Without them the release still publishes; only the tap update is skipped.
