# Template for the awsctl Homebrew formula.
#
# The release workflow renders the placeholders below from the tag and the
# .sha256 sidecars published with each release, then pushes the result to the
# tap repository named by the HOMEBREW_TAP_REPO variable. See
# .github/workflows/release.yml.
class Awsctl < Formula
  desc "CLI for AWS SSO, EC2, RDS, SSM and EKS operations"
  homepage "https://github.com/REPO_PLACEHOLDER"
  version "VERSION_PLACEHOLDER"

  on_macos do
    url "https://github.com/REPO_PLACEHOLDER/releases/download/v#{version}/awsctl-aarch64-apple-darwin.tar.xz"
    sha256 "MAC_ARM64_SHA256_PLACEHOLDER"
  end

  on_linux do
    url "https://github.com/REPO_PLACEHOLDER/releases/download/v#{version}/awsctl-x86_64-unknown-linux-gnu.tar.xz"
    sha256 "LINUX_X86_64_SHA256_PLACEHOLDER"
  end

  def install
    bin.install "awsctl"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/awsctl --version")
  end
end
