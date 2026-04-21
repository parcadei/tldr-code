.PHONY: build test lint fmt clean release install

build:
	cargo build --release

test:
	cargo test -p tldr-core --lib
	cargo test -p tldr-cli --lib

lint:
	cargo clippy --workspace -- -D warnings

fmt:
	cargo fmt --check

clean:
	cargo clean

install: build
	cp target/release/tldr ~/.local/bin/tldr

# Run all checks (CI equivalent)
check: fmt lint test

# Quick dev build
dev:
	cargo build
