# Homebrew formula for `brew tap misbeliever/tap && brew install trm`
class Trm < Formula
  desc "Fast, zero-dependency Rust CLI that trims shell output for AI coding agents"
  homepage "https://github.com/misbeliever/shell-trim"
  version "0.2.1"
  license "MIT"

  on_macos do
    on_arm do
      url "https://github.com/misbeliever/shell-trim/releases/download/v#{version}/trm-macos-arm64.tar.gz"
      sha256 "854d7457ae3fdc43992835344c514c569cd482515406629d2f02a19aae666c62"
    end
    on_intel do
      url "https://github.com/misbeliever/shell-trim/releases/download/v#{version}/trm-macos-x86_64.tar.gz"
      sha256 "445ae698d9ed46b542883086cda6944a5edc06c0406d4d306234d473391da569"
    end
  end

  on_linux do
    url "https://github.com/misbeliever/shell-trim/releases/download/v#{version}/trm-linux-x86_64.tar.gz"
    sha256 "1e3cb93e73603bf75f5cc36303c654fbaba811a868493f778fdf7d5e9ec94a63"
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
