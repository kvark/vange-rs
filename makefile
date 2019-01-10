RUSTC_FLAGS=-C prefer-dynamic
# May be --release
CARGO_FLAGS=

# Classic static build
static: Cargo.toml
	cargo build ${CARGO_FLAGS}

# Use nightly- toolchain to build dependencies with Rust 2018 edition
static-nightly:
	cargo +nightly build ${CARGO_FLAGS}

# Shared build to produce thin binaries
shared:
	env RUSTFLAGS="${RUSTC_FLAGS}" cargo build ${CARGO_FLAGS}

# Shared build with nightly toolchain
shared-nightly:
	env RUSTFLAGS="${RUSTC_FLAGS}" cargo +nightly build ${CARGO_FLAGS}

