.PHONY: test build fmt lint check

test:   ## run unit tests, including the YieldBlox replay
	cargo test

build:  ## build the optimized wasm (soroban-sdk 29 requires stellar-cli)
	stellar contract build

fmt:    ## format
	cargo fmt

lint:   ## clippy with warnings as errors
	cargo clippy --all-targets -- -D warnings

check: fmt lint test build
