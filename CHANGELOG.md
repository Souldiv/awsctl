# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [1.2.4](https://github.com/Souldiv/awsctl/releases/tag/v1.2.4) - 2026-11-02

### Added

- add an All choice to sso -s and a --refresh flag ([#26](https://github.com/Souldiv/awsctl/pull/26))
- switch [default] and list written profiles from sso ([#24](https://github.com/Souldiv/awsctl/pull/24))
- hand the SSO token to the AWS CLI during sso -s ([#20](https://github.com/Souldiv/awsctl/pull/20))
- write SSO profiles to ~/.aws/config and select multiple accounts ([#19](https://github.com/Souldiv/awsctl/pull/19))
- add --region flag to sso subcommand ([#16](https://github.com/Souldiv/awsctl/pull/16))
- awsctl, a CLI for AWS SSO, EC2, RDS, SSM and EKS

### Fixed

- let release-plz publish awsctl to crates.io ([#32](https://github.com/Souldiv/awsctl/pull/32))
- keep the account's last region when setting up [default] ([#27](https://github.com/Souldiv/awsctl/pull/27))

### Other

- release binaries with dist and publish to crates.io ([#31](https://github.com/Souldiv/awsctl/pull/31))
- release v1.2.3 ([#30](https://github.com/Souldiv/awsctl/pull/30))
- add release-plz release PRs and tagging ([#29](https://github.com/Souldiv/awsctl/pull/29))
- move macOS jobs to macos-15 ([#28](https://github.com/Souldiv/awsctl/pull/28))
- *(deps)* bring every direct dependency current ([#25](https://github.com/Souldiv/awsctl/pull/25))
- *(deps)* refresh Cargo.lock via cargo update ([#15](https://github.com/Souldiv/awsctl/pull/15))
- add MIT license ([#14](https://github.com/Souldiv/awsctl/pull/14))
- *(deps)* bump actions/create-github-app-token from 2 to 3 ([#5](https://github.com/Souldiv/awsctl/pull/5))
- *(deps)* bump actions/upload-artifact from 6 to 7 ([#4](https://github.com/Souldiv/awsctl/pull/4))
- *(deps)* bump actions/download-artifact from 7 to 8 ([#1](https://github.com/Souldiv/awsctl/pull/1))

## [1.2.3](https://github.com/Souldiv/awsctl/compare/v1.0.5...v1.2.3) - 2026-11-02

### Other

- add release-plz release PRs and tagging ([#29](https://github.com/Souldiv/awsctl/pull/29))
- move macOS jobs to macos-15 ([#28](https://github.com/Souldiv/awsctl/pull/28))
