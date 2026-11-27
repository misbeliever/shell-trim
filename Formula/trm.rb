# Homebrew formula for `brew tap misbeliever/tap && brew install trm`
class Trm < Formula
  desc "Fast, zero-dependency Rust CLI that trims shell output for AI coding agents"
  homepage "https://github.com/misbeliever/shell-trim"
  version "0.1.0"
  license "MIT"

  on_macos do
    on_arm do
      url "https://github.com/misbeliever/shell-trim/releases/download/v#{version}/trm-macos-arm64.tar.gz"
      sha256 "15b1f0ed28c818d0879d3d0283cb7b4fbdaa5c8dd8a2698619f8bdaf98a2cfc0"
    end
    on_intel do
      url "https://github.com/misbeliever/shell-trim/releases/download/v#{version}/trm-macos-x86_64.tar.gz"
      sha256 "91c12f7b36728b650cfd20cfcd6dd64ea9a173fdea6e3530d703dd6ea32cb737"
    end
  end

  on_linux do
    url "https://github.com/misbeliever/shell-trim/releases/download/v#{version}/trm-linux-x86_64.tar.gz"
    sha256 "65774092ea496110b7a39fa230d1d2fbb19e9ff1bca79a5cc9cdf52ea87900ac"
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
