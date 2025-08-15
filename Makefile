.PHONY: help all clean test test-cov test-cov-lcov build release lint fmt check-fmt markdownlint nixie

APP ?= comenq
CARGO ?= cargo
BUILD_JOBS ?=
CLIPPY_FLAGS ?= --all-targets --all-features -- -D warnings
MDLINT ?= markdownlint
NIXIE ?= nixie
COV_MIN ?= 0 # Minimum line coverage percentage for coverage targets

define CHECK_CARGO_LLVM_COV
        @command -v cargo-llvm-cov >/dev/null || { \
        echo "error: cargo-llvm-cov not found. Install with: cargo install cargo-llvm-cov" >&2; \
        exit 127; \
        }
        @command -v rustup >/dev/null || { \
        echo "error: rustup not found. Install from: https://rustup.rs" >&2; \
        exit 127; \
        }
        @rustup component list --installed | grep -q '^llvm-tools' || { \
        echo "error: rustup component llvm-tools-preview not found. Install with: rustup component add llvm-tools-preview" >&2; \
        exit 127; \
        }
endef

build: target/debug/$(APP) ## Build debug binary
release: target/release/$(APP) ## Build release binary

all: release ## Default target builds release binary

clean: ## Remove build artefacts
	$(CARGO) clean
	@command -v cargo-llvm-cov >/dev/null && $(CARGO) llvm-cov clean --workspace || true
	rm -rf coverage

test: ## Run tests with warnings treated as errors
	RUSTFLAGS="-D warnings" $(CARGO) test --all-targets --all-features $(BUILD_JOBS)

test-cov: ## Run workspace-wide tests with coverage; set COV_MIN to enforce a threshold
	$(CHECK_CARGO_LLVM_COV)
	RUSTFLAGS="-D warnings" $(CARGO) llvm-cov --workspace --all-features --doctests --summary-only --text --fail-under-lines $(COV_MIN) $(BUILD_JOBS)

test-cov-lcov: ## Run workspace-wide tests with coverage and write LCOV to coverage/lcov.info
	$(CHECK_CARGO_LLVM_COV)
	mkdir -p coverage
	RUSTFLAGS="-D warnings" $(CARGO) llvm-cov --workspace --all-features --doctests --lcov --output-path coverage/lcov.info --fail-under-lines $(COV_MIN) $(BUILD_JOBS)

target/%/$(APP): ## Build binary in debug or release mode
	$(CARGO) build $(BUILD_JOBS) $(if $(findstring release,$(@)),--release) --bin $(APP)

lint: ## Run Clippy with warnings denied
	$(CARGO) clippy $(CLIPPY_FLAGS)

fmt: ## Format Rust and Markdown sources
	$(CARGO) fmt --all
	mdformat-all

check-fmt: ## Verify formatting
	$(CARGO) fmt --all -- --check

markdownlint: ## Lint Markdown files
	find . -type f -name '*.md' -not -path './target/*' -print0 | xargs -0 $(MDLINT)

nixie: ## Validate Mermaid diagrams
	find . -type f -name '*.md' -not -path './target/*' -print0 | xargs -0 $(NIXIE)

help: ## Show available targets
	@grep -E '^[a-zA-Z_-]+:.*?##' $(MAKEFILE_LIST) | \
	awk 'BEGIN {FS=":"; printf "Available targets:\n"} {printf "  %-20s %s\n", $$1, $$2}'
