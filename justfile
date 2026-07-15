set dotenv-load := false

default:
    @just --list

# Run every local handoff check.
check: fmt-check lint test

# Format Rust sources.
fmt:
    cargo fmt --all

# Verify formatting without changing files.
fmt-check:
    cargo fmt --all --check

# Run Clippy across the workspace.
lint:
    cargo clippy --workspace --all-targets --all-features -- -D warnings

# Run the complete test suite.
test:
    cargo test --workspace --all-targets --all-features

# Run the CLI. Pass arguments after `--`, for example: just run -- guide agents
run *args:
    cargo run -- {{args}}

# Install the local development build in Cargo's bin directory.
install:
    cargo install --path crates/hostbraid-cli --locked
