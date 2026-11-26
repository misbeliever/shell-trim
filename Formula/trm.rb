# Homebrew formula template for a future `brew tap misbeliever/tap && brew install trm`
# After tagging a GitHub release (e.g. `git tag v0.1.0 && git push --tags`),
# update `url` and `sha256` from the release `.sha256` assets.
class Trm < Formula
  desc "Fast, zero-dependency Rust CLI that trims shell output for AI coding agents"
  homepage "https://github.com/misbeliever/shell-trim"
  version "0.1.0"
  license "MIT"

  on_macos do
    on_arm do
      url "https://github.com/misbeliever/shell-trim/releases/download/v#{version}/trm-macos-arm64.tar.gz"
      sha256 "REPLACE_WITH_ARM64_SHA256"
    end
    on_intel do
      url "https://github.com/misbeliever/shell-trim/releases/download/v#{version}/trm-macos-x86_64.tar.gz"
      sha256 "REPLACE_WITH_X86_64_SHA256"
    end
  end

  on_linux do
    url "https://github.com/misbeliever/shell-trim/releases/download/v#{version}/trm-linux-x86_64.tar.gz"
    sha256 "REPLACE_WITH_LINUX_SHA256"
  end

  depends_on "ripgrep"

  def install
    bin.install "trm"
  end

  def caveats
    <<~'EOS'
       _
      | |_ _ __ _ __ ___
      | __| '__| '_ ` _ \
      | |_| |  | | | | | |
       \__|_|  |_| |_| |_|  cut shell noise. keep the signal.

      Next steps:
        trm install    # install always-on rules for your AI coding agents
        trm doctor     # verify PATH, rules, config & telemetry
        trm ui         # open live savings dashboard
    EOS
  end

  test do
    assert_match "trm #{version}", shell_output("#{bin}/trm --version")
  end
end
