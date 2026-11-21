.PHONY: build release test clean

build:
	cargo build --locked

release:
	cargo build --release --locked --features vendored-tls

test:
	cargo test --locked
