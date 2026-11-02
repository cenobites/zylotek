.PHONY: setup prek prek-all prek-install build  \
        test test-unit test-integration test-known-bugs \
        cov cov-unit cov-integration cov-html \
        bench bench-check fuzz fuzz-build \
        clean help

# =============================================================================
# Setup
# =============================================================================

setup: prek-install
	cargo install --locked prek sccache cargo-llvm-cov cargo-fuzz cargo-insta \
	cargo-audit cargo-outdated
	rustup toolchain install nightly --profile minimal  # cargo-fuzz needs it

# =============================================================================
# Pre-commit hooks
# =============================================================================

# Run prek on staged files
prek:
	prek run

# Run prek on all files
prek-all:
	prek run --all-files

# Install prek git hooks
prek-install:
	prek install

# =============================================================================
# Helpers
# =============================================================================

migrate:
	cargo run -p zylotek-cli -- db migrate

# =============================================================================
# Tests
# =============================================================================

# Files that are not measured, and why:
#   testing.rs, test_support.rs  test support, not product code
#   models.rs                    derive-generated Toasty schema, no logic
#   server.rs, migrations.rs     process entry points (fixed port, OS signals, cwd);
#                                measured by the integration gate instead
#   zylotek-cli, main.rs         thin binaries, exercised through the compiled binary
COMMON_IGNORE := (testing|test_support)\.rs|/tests/|/benches/|/src/main\.rs$$
UNIT_IGNORE := $(COMMON_IGNORE)|persistence/woods/models\.rs|/(server|migrations)\.rs|crates/zylotek-cli/
INTEGRATION_IGNORE := $(COMMON_IGNORE)

# Every test: unit, integration and doc tests
test:
	cargo test --workspace

# Unit tests: fast, no network, no ports
test-unit:
	cargo test --workspace --lib

# Integration tests: real SQLite, real sockets, the compiled CLI
test-integration:
	cargo test --workspace --test '*'

# Known bugs recorded as #[ignore]d tests; these are expected to fail until fixed
test-known-bugs:
	cargo test --workspace -- --ignored

# Unit coverage gate: every line of the measured files, bar one that is unreachable
# (basic_information.rs: the `else` of `ElementRef::wrap` on a `div > a > img` grandparent)
cov-unit:
	cargo llvm-cov --workspace --lib --ignore-filename-regex '$(UNIT_IGNORE)' --fail-uncovered-lines 1

# Integration coverage gate: at least 80% of lines, from tests/ alone
cov-integration:
	cargo llvm-cov --workspace --test '*' --ignore-filename-regex '$(INTEGRATION_IGNORE)' --fail-under-lines 80

cov: cov-unit cov-integration

# Browsable report of unit + integration coverage
cov-html:
	cargo llvm-cov --workspace --tests --ignore-filename-regex '$(INTEGRATION_IGNORE)' --html --open

# =============================================================================
# Benchmarks (criterion; reports in target/criterion)
# =============================================================================

bench:
	cargo bench --workspace

# Runs every benchmark once, to prove they still work
bench-check:
	cargo bench --workspace --benches -- --test

# =============================================================================
# Fuzzing (cargo-fuzz, nightly)
# =============================================================================

FUZZ_TARGETS := parse_wood parse_open_data wood_request_json value_objects http_requests
FUZZ_TIME ?= 60

fuzz-build:
	cd fuzz && cargo +nightly fuzz build

# make fuzz [FUZZ_TIME=seconds per target] [FUZZ_TARGETS="parse_wood"]
fuzz:
	@for target in $(FUZZ_TARGETS); do \
		echo "==> fuzzing $$target for $(FUZZ_TIME)s"; \
		mkdir -p fuzz/corpus/$$target; \
		(cd fuzz && cargo +nightly fuzz run $$target corpus/$$target $$(test -d seeds/$$target && echo seeds/$$target) -- -max_total_time=$(FUZZ_TIME) -rss_limit_mb=4096) || exit 1; \
	done

# =============================================================================
# Build targets
# =============================================================================

# Development build (fast compile, debug symbols)
build:
	cargo build -p zylotek-cli

# =============================================================================
# Help
# =============================================================================

help:
	@echo "Zylotek System"
	@echo ""
	@echo "Setup:"
	@echo "  make setup             - Initial setup"
	@echo ""
	@echo "Build targets:"
	@echo "  make build             - Development build (fast)"
	@echo ""
	@echo "Tests:"
	@echo "  make test              - Unit, integration and doc tests"
	@echo "  make test-unit         - Unit tests only"
	@echo "  make test-integration  - Integration tests only"
	@echo "  make test-known-bugs   - Run the #[ignore]d tests that document open bugs"
	@echo "  make cov               - Coverage gates: unit (~100%) and integration (80%)"
	@echo "  make cov-html          - HTML coverage report"
	@echo "  make bench             - Criterion benchmarks"
	@echo "  make bench-check       - Run each benchmark once"
	@echo "  make fuzz              - Fuzz every target (FUZZ_TIME=60 seconds each)"
	@echo ""
