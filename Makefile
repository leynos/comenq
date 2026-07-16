.PHONY: help all clean test test-cov test-cov-lcov test-workflow-contracts build release lint fmt check-fmt markdownlint nixie spelling spelling-config spelling-config-write spelling-phrase-check spelling-helper-test

APP ?= comenq
CARGO ?= cargo
BUILD_JOBS ?=
CLIPPY_FLAGS ?= --workspace --all-targets --all-features -- -D warnings
MDLINT ?= markdownlint-cli2
WHITAKER ?= whitaker
UV ?= uv
UV_ENV = UV_CACHE_DIR=.uv-cache UV_TOOL_DIR=.uv-tools
RUFF_VERSION ?= 0.15.12
PATHSPEC_VERSION ?= 1.1.1
TYPOS_VERSION ?= 1.48.0
NIXIE_VERSION ?= 1.1.0
NIXIE = $(UV_ENV) $(UV) tool run --python 3.14 \
	--from nixie-cli@$(NIXIE_VERSION) nixie
TYPOS_CONFIG_BUILDER_COMMIT := b604f198797fdd36a567dd0f8f07b13f9539b241
TYPOS_CONFIG_BUILDER_SOURCE := git+https://github.com/leynos/typos-config-builder.git@$(TYPOS_CONFIG_BUILDER_COMMIT)
TYPOS_CONFIG_BUILDER := $(UV_ENV) $(UV) tool run --python 3.14 \
	--from "$(TYPOS_CONFIG_BUILDER_SOURCE)" typos-config-builder
SPELLING_PY_SRCS := \
	scripts/typos_rollout_check.py scripts/tests/test_typos_rollout_check.py
SPELLING_PY_TESTS := scripts/tests/test_typos_rollout_check.py
SPELLING_COVERAGE_ARGS := --cov=typos_rollout_check --cov-fail-under=90
SPELLING_PY_ENV := PYTHONDONTWRITEBYTECODE=1
SPELLING_COVERAGE_FILE ?= /tmp/$(APP)-spelling-helper.coverage
SPELLING_HELPER_PYTEST = PYTHONPATH=scripts $(SPELLING_PY_ENV) \
	COVERAGE_FILE=$(SPELLING_COVERAGE_FILE) $(UV_ENV) $(UV) run --no-project \
	--python 3.14 --with pathspec==$(PATHSPEC_VERSION) --with pytest==9.0.2 \
	--with pytest-cov==7.0.0 python -m pytest
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

fmt: ## Format Rust and Markdown sources
	$(CARGO) fmt --all
	mdformat-all

check-fmt: ## Verify formatting
	$(CARGO) fmt --all -- --check

markdownlint: spelling ## Lint Markdown files and enforce spelling
	$(MDLINT) "**/*.md"

spelling: spelling-phrase-check ## Enforce en-GB-oxendict spelling in Markdown prose
	@git ls-files -z '*.md' | \
		xargs -0 -r env $(UV_ENV) $(UV) tool run typos@$(TYPOS_VERSION) \
		--config typos.toml --force-exclude

spelling-phrase-check: spelling-config ## Reject prohibited spelling phrases
	@PYTHONPATH=scripts $(SPELLING_PY_ENV) $(UV_ENV) $(UV) run --no-project --python 3.14 \
		--with pathspec==$(PATHSPEC_VERSION) scripts/typos_rollout_check.py \
		--repository .

spelling-config: spelling-helper-test ## Verify generated spelling configuration
	@git ls-files --error-unmatch typos.toml >/dev/null
	@$(TYPOS_CONFIG_BUILDER) --repository . --check

spelling-config-write: spelling-helper-test ## Generate spelling configuration
	@$(TYPOS_CONFIG_BUILDER) --repository .

spelling-helper-test: ## Validate the shared spelling-policy integration
	@$(UV_ENV) $(UV) tool run ruff@$(RUFF_VERSION) format --isolated --target-version py313 --check $(SPELLING_PY_SRCS)
	@$(UV_ENV) $(UV) tool run ruff@$(RUFF_VERSION) check --isolated --target-version py313 $(SPELLING_PY_SRCS)
	@$(SPELLING_HELPER_PYTEST) $(SPELLING_PY_TESTS) -c /dev/null --rootdir=. -p no:cacheprovider $(SPELLING_COVERAGE_ARGS)

nixie: ## Validate Mermaid diagrams
	$(NIXIE) --no-sandbox --max-concurrency 1

help: ## Show available targets
	@grep -E '^[a-zA-Z_-]+:.*?##' $(MAKEFILE_LIST) | \
	awk 'BEGIN {FS=":"; printf "Available targets:\n"} {printf "  %-20s %s\n", $$1, $$2}'
