BIN := $(HOME)/.local/bin/trm

.PHONY: test build install clean

test:
	cargo test

build:
	cargo build --release

install: test build
	mkdir -p $(HOME)/.local/bin
	rm -f $(BIN)
	cp target/release/trm $(BIN)
	@echo "Installed $(BIN)"

clean:
	cargo clean
