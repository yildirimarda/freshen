class Freshen < Formula
  desc "One command to keep a developer machine current"
  homepage "https://github.com/yildirimarda/freshen"
  version "0.1.0"
  license "MIT"

  # The sha256 values below are placeholders. Replace each one with the
  # sha256 of the corresponding v0.1.0 release asset once the tag is pushed
  # and the GitHub Release created by ci-proposals/release-binaries.yml:
  #
  #   https://github.com/yildirimarda/freshen/releases/tag/v0.1.0
  #
  # Release asset naming (see ci-proposals/release-binaries.yml):
  #   freshen-<version>-<target>.tar.gz   (macOS, Linux)
  #   freshen-<version>-<target>.zip      (Windows)
  #
  # This file is the canonical Homebrew formula. It is copied into the
  # yildirimarda/homebrew-tap repository as `freshen.rb` so it can be
  # installed with `brew install yildirimarda/homebrew-tap/freshen`.
  # (The tap repository is created separately; see HUMAN-TASKS.md.)

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/yildirimarda/freshen/releases/download/v0.1.0/freshen-0.1.0-aarch64-apple-darwin.tar.gz"
      sha256 "0000000000000000000000000000000000000000000000000000000000000000"
    else
      url "https://github.com/yildirimarda/freshen/releases/download/v0.1.0/freshen-0.1.0-x86_64-apple-darwin.tar.gz"
      sha256 "0000000000000000000000000000000000000000000000000000000000000000"
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/yildirimarda/freshen/releases/download/v0.1.0/freshen-0.1.0-aarch64-unknown-linux-musl.tar.gz"
      sha256 "0000000000000000000000000000000000000000000000000000000000000000"
    else
      url "https://github.com/yildirimarda/freshen/releases/download/v0.1.0/freshen-0.1.0-x86_64-unknown-linux-musl.tar.gz"
      sha256 "0000000000000000000000000000000000000000000000000000000000000000"
    end
  end

  def install
    bin.install "freshen"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/freshen --version")
  end
end
