# Parakeet Local tasks. Run `just` to list them.
# Everything goes through scripts/cargo so the pinned Rust toolchain is used.

# Library used by `just run` and `just dev`; override with `just data=/path run`.
data := ".local/app"

[private]
default:
    @just --list

# Format all Rust code
fmt:
    scripts/cargo fmt --all

# Check formatting without changing files
fmt-check:
    scripts/cargo fmt --all --check

# Lint with clippy; warnings are errors
lint:
    scripts/cargo clippy --workspace --all-targets --quiet -- -D warnings

# Run all tests
test:
    scripts/cargo test --workspace --quiet

# Type-check without building binaries (fast)
check:
    scripts/cargo check --workspace --all-targets --quiet

# Everything CI-like: format, lint, test
verify: fmt-check lint test

# Build the app bundle (debug)
build:
    scripts/build-app

# Build the optimized app bundle
release:
    scripts/build-app --release

# Build the optimized app bundle with UI performance instrumentation
performance:
    scripts/build-app --release --performance

# Launch the last built app on a library (build first)
run:
    scripts/run-desktop {{data}}

# Build the debug app, then launch it
dev: build run

# Run the command-line tool, e.g. `just cli models list`
cli *args:
    scripts/cargo run --quiet -p parakeet-cli -- {{args}}

# Regenerate the SQLx query metadata after changing SQL or migrations
sql:
    scripts/prepare-sql

# Remove build output (keeps your library data)
clean:
    scripts/cargo clean
    rm -rf "target/Parakeet Local.app"
