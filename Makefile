# =============================================================================
# Oxigraph Cloud-Native — Makefile
# =============================================================================
# Works on Linux and macOS. Prefers podman; falls back to docker.
# =============================================================================

VERSION   ?= $(shell grep '^version' Cargo.toml | head -1 | sed 's/.*"\(.*\)"/\1/')
IMAGE     ?= oxigraph-cloud
REGISTRY  ?= ghcr.io
REPO      ?= $(shell git remote get-url origin 2>/dev/null | sed 's|.*github.com[:/]||;s|\.git||')
HELM_DIR  := deploy/helm/oxigraph-cloud

# Detect container runtime: prefer podman, fall back to docker
CONTAINER_RUNTIME := $(shell command -v podman 2>/dev/null || command -v docker 2>/dev/null)

.PHONY: build test lint container helm-lint helm-package deploy-sandbox clean help

## build: Compile release binary
build:
	cargo build --release -p oxigraph-server

## test: Run all workspace tests
test:
	cargo test --workspace

## lint: Run clippy and format check
lint:
	cargo clippy --workspace --all-targets -- -D warnings
	cargo fmt --all -- --check

## container: Build container image
container:
	$(CONTAINER_RUNTIME) build -t $(IMAGE):$(VERSION) -t $(IMAGE):latest -f Containerfile .

## helm-lint: Lint Helm chart with all value files
helm-lint:
	helm lint $(HELM_DIR)
	@for f in $(wildcard $(HELM_DIR)/values-*.yaml); do \
		echo "--- Linting with $$f ---"; \
		helm lint $(HELM_DIR) -f $$f; \
	done

## helm-package: Package Helm chart
helm-package: helm-lint
	helm package $(HELM_DIR) --destination dist/

## deploy-sandbox: Install chart with Developer Sandbox values
deploy-sandbox:
	helm upgrade --install oxigraph-cloud $(HELM_DIR) \
		-f $(HELM_DIR)/values-sandbox.yaml \
		--wait --timeout 5m

## clean: Remove build artifacts
clean:
	cargo clean
	rm -rf dist/

## help: Show this help
help:
	@grep -E '^## ' $(MAKEFILE_LIST) | sed 's/## //' | column -t -s ':'
