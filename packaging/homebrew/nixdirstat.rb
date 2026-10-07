# typed: false
# frozen_string_literal: true

class Nixdirstat < Formula
  desc "Disk usage analyzer and cleanup assistant for Unix and Unix-like systems"
  homepage "https://github.com/meethune/nixdirstat"
  version "RELEASE_VERSION"
  license "MIT"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/meethune/nixdirstat/releases/download/v#{version}/nixdirstat-v#{version}-aarch64-apple-darwin.tar.gz"
      sha256 "SHA256_AARCH64_APPLE_DARWIN"
    else
      url "https://github.com/meethune/nixdirstat/releases/download/v#{version}/nixdirstat-v#{version}-x86_64-apple-darwin.tar.gz"
      sha256 "SHA256_X86_64_APPLE_DARWIN"
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/meethune/nixdirstat/releases/download/v#{version}/nixdirstat-v#{version}-aarch64-unknown-linux-musl.tar.gz"
      sha256 "SHA256_AARCH64_LINUX_MUSL"
    else
      url "https://github.com/meethune/nixdirstat/releases/download/v#{version}/nixdirstat-v#{version}-x86_64-unknown-linux-musl.tar.gz"
      sha256 "SHA256_X86_64_LINUX_MUSL"
    end
  end

  def install
    bin.install "nixdirstat"
    bash_completion.install "completions/nixdirstat.bash" => "nixdirstat"
    zsh_completion.install "completions/_nixdirstat"
    fish_completion.install "completions/nixdirstat.fish"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/nixdirstat --version")
  end
end
