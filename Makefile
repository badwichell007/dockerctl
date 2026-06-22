.PHONY: build release run test check fmt install uninstall

build:
	cargo build

release:
	cargo build --release

run:
	cargo run --

test:
	cargo test --all-targets

check:
	cargo check --all-targets

fmt:
	cargo fmt

install:
	bash "./scripts/install-cli.sh"

uninstall:
	bash "./scripts/uninstall-cli.sh"
