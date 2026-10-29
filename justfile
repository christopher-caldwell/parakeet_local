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
verify: fmt-check lint test native-test

# Compile and check microphone routing logic without opening an audio device
native-test:
    scripts/build-native
    target/native/parakeet-bridge microphone-route-self-test
    target/native/parakeet-bridge microphone-format-self-test .local/scratchpad/native-tests
    target/native/parakeet-bridge microphone-stall-self-test .local/scratchpad/native-tests

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

# Compile the Swift helpers (audio bridge, model runtime); skipped when their sources are unchanged
native:
    scripts/build-native
    scripts/build-model-runtime

# Fast dev loop: rebuild and restart the app whenever Rust code changes. Runs the bare binary
# (helpers come from target/native), so macOS microphone/dictation permissions need `just build run`.
dev: native
    root=$(cd "{{data}}" && pwd) && PARAKEET_DATA_ROOT="$root" scripts/cargo watch --quiet --clear -w crates -w Cargo.toml -x 'run --quiet -p parakeet-desktop'

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
