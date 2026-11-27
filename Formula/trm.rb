# Homebrew formula for `brew tap misbeliever/tap && brew install trm`
class Trm < Formula
  desc "Fast, zero-dependency Rust CLI that trims shell output for AI coding agents"
  homepage "https://github.com/misbeliever/shell-trim"
  version "0.2.0"
  license "MIT"

  on_macos do
    on_arm do
      url "https://github.com/misbeliever/shell-trim/releases/download/v#{version}/trm-macos-arm64.tar.gz"
      sha256 "8fa6d915b9e889b0a8addd70c09efcbfabd473ca391bbfa0439a93dffc8ff611"
    end
    on_intel do
      url "https://github.com/misbeliever/shell-trim/releases/download/v#{version}/trm-macos-x86_64.tar.gz"
      sha256 "20a308a4374b0aae3bb1e124eb8cd426d618640e416438289ad0613cb591ad4b"
    end
  end

  on_linux do
    url "https://github.com/misbeliever/shell-trim/releases/download/v#{version}/trm-linux-x86_64.tar.gz"
    sha256 "e37f12154552aed1a1e8dc6a78ae2365608b8bc96df296d8591ea64211f7e82f"
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
