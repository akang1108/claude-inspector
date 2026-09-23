.DEFAULT_GOAL := help
.PHONY: help build release run list test check fmt lint install clean

help: ## Show this help
	@echo "Usage: make <target>"
	@echo
	@grep -E '^[a-z-]+:.*## ' $(MAKEFILE_LIST) | awk -F':.*## ' '{printf "  %-10s %s\n", $$1, $$2}'

build: ## Debug build
	cargo build

release: ## Optimized build
	cargo build --release

run: ## Run the TUI (pass ARGS="-c ~/.claude-me")
	cargo run --release -- $(ARGS)

list: ## Print plain-text summary, no TUI
	cargo run --release -- --list $(ARGS)

test: ## Run tests
	cargo test

check: ## Type-check without building
	cargo check --all-targets

fmt: ## Format code
	cargo fmt

lint: ## Clippy, warnings as errors
	cargo clippy --all-targets -- -D warnings

install: ## Install binary to ~/.cargo/bin
	cargo install --path .

clean: ## Remove build artifacts
	cargo clean
