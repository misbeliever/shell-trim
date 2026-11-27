BIN := $(HOME)/.local/bin/trm

.PHONY: test fmt lint build install uninstall clean

test:
	cargo test

fmt:
	cargo fmt --all

lint:
	cargo clippy --all-targets -- -D warnings

build:
	cargo build --release

install: test build
	mkdir -p $(HOME)/.local/bin
	install -m 755 target/release/trm $(BIN)
	@echo "Installed $(BIN)"

uninstall:
	rm -f $(BIN)
	@echo "Removed $(BIN)"

clean:
	cargo clean
