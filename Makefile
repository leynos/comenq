.PHONY: help all clean test test-cov test-cov-lcov test-workflow-contracts build release lint typecheck fmt check-fmt markdownlint nixie spelling

APP ?= comenq
CARGO ?= cargo
BUILD_JOBS ?=
CLIPPY_FLAGS ?= --workspace --all-targets --all-features -- -D warnings
MDLINT ?= markdownlint-cli2
# `make fmt` and `make check-fmt` call mdtablefix directly. `--git` selects the
# Markdown files Git tracks and `--include-untracked` adds the untracked files
# Git does not ignore, so a new document is formatted before it is staged.
# Both modes need mdtablefix 0.6.0 or later; CI pins the version at the
# install-mdtablefix step.
MDTABLEFIX ?= mdtablefix
MDTABLEFIX_SELECT = --git --include-untracked
MDTABLEFIX_RULES = --wrap --renumber --breaks --ellipsis --fences
WHITAKER ?= whitaker
UV ?= uv
UV_ENV = UV_CACHE_DIR=.uv-cache UV_TOOL_DIR=.uv-tools
NIXIE_VERSION ?= 1.1.0
NIXIE = $(UV_ENV) $(UV) tool run --python 3.14 \
	--from nixie-cli@$(NIXIE_VERSION) nixie
TYPOS_CONFIG_BUILDER_VERSION ?= v0.1.1
TYPOS_CONFIG_BUILDER = $(UV_ENV) $(UV) tool run --python 3.14 --from \
	"git+https://github.com/leynos/typos-config-builder.git@$(TYPOS_CONFIG_BUILDER_VERSION)" \
	typos-config-builder
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

all: release spelling ## Default target builds release binary and checks spelling

clean: ## Remove build artefacts
	$(CARGO) clean
	@command -v cargo-llvm-cov >/dev/null && $(CARGO) llvm-cov clean --workspace || true
	rm -rf coverage

test: ## Run tests with warnings treated as errors
	RUSTFLAGS="-D warnings" $(CARGO) nextest run --workspace --all-targets --all-features $(BUILD_JOBS)
	RUSTFLAGS="-D warnings" $(CARGO) test --workspace --all-features --test cucumber $(BUILD_JOBS)

test-cov: ## Run workspace-wide tests with coverage; set COV_MIN to enforce a threshold
	$(CHECK_CARGO_LLVM_COV)
	RUSTFLAGS="-D warnings" $(CARGO) llvm-cov nextest --workspace --all-features --summary-only --text --fail-under-lines $(COV_MIN) $(BUILD_JOBS)
	RUSTFLAGS="-D warnings" $(CARGO) llvm-cov --no-clean --workspace --all-features --test cucumber --summary-only --text --fail-under-lines $(COV_MIN) $(BUILD_JOBS)

test-cov-lcov: ## Run workspace-wide tests with coverage and write LCOV to coverage/lcov.info
	$(CHECK_CARGO_LLVM_COV)
	mkdir -p coverage
	RUSTFLAGS="-D warnings" $(CARGO) llvm-cov nextest --workspace --all-features --lcov --output-path coverage/lcov.info --fail-under-lines $(COV_MIN) $(BUILD_JOBS)
	RUSTFLAGS="-D warnings" $(CARGO) llvm-cov --no-clean --workspace --all-features --test cucumber --lcov --output-path coverage/lcov-cucumber.info --fail-under-lines $(COV_MIN) $(BUILD_JOBS)

test-workflow-contracts: ## Validate the mutation-testing caller contract
	uv run --with 'pytest>=8' --with 'pyyaml>=6' pytest tests/workflow_contracts -q

target/%/$(APP): ## Build binary in debug or release mode
	$(CARGO) build $(BUILD_JOBS) $(if $(findstring release,$(@)),--release) --bin $(APP)

lint: ## Run Clippy and the Whitaker Dylint suite with warnings denied
	$(CARGO) clippy $(CLIPPY_FLAGS)
	RUSTFLAGS="-D warnings" $(WHITAKER) --all -- --all-targets --all-features

typecheck: ## Check all Rust targets and features
	RUSTFLAGS="-D warnings" $(CARGO) check --workspace --all-targets --all-features $(BUILD_JOBS)

fmt: ## Format Rust and Markdown sources
	$(CARGO) fmt --all
	$(MDTABLEFIX) --in-place $(MDTABLEFIX_SELECT) $(MDTABLEFIX_RULES)
	@unset FORCE_COLOR; $(MDLINT) --fix "**/*.md"

check-fmt: ## Verify formatting
	$(CARGO) fmt --all -- --check
	$(MDTABLEFIX) --check $(MDTABLEFIX_SELECT) $(MDTABLEFIX_RULES)

markdownlint: spelling ## Lint Markdown files and enforce spelling
	$(MDLINT) "**/*.md"

spelling: ## Enforce en-GB-oxendict spelling in Markdown prose
	$(TYPOS_CONFIG_BUILDER) gate --repository .

nixie: ## Validate Mermaid diagrams
	$(NIXIE) --no-sandbox --max-concurrency 1

help: ## Show available targets
	@grep -E '^[a-zA-Z_-]+:.*?##' $(MAKEFILE_LIST) | \
	awk 'BEGIN {FS=":"; printf "Available targets:\n"} {printf "  %-20s %s\n", $$1, $$2}'
