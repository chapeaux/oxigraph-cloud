# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.7.4] - 2026-03-23

### Changed
- Upgrade tikv-client 0.3.0 → 0.4.0
- Unpin rudof crates to allow semver-compatible updates
- TiKV container base image: ubi-micro → ubi-minimal (fixes gRPC DNS EBUSY)
- Install TiKV runtime deps via microdnf instead of copying .so files
- Add TiKV connection retry with exponential backoff (5 attempts)

## [0.7.3] - 2026-03-23

### Fixed
- Containerfile.tikv: copy libssl, libcrypto, libz to runtime image (tikv-client links OpenSSL dynamically)

### Changed
- Updated CLAUDE.md with full HTTP API table, container image variant details, cargo features
- Updated README.md with transactions, changelog, and container variant documentation

## [0.7.2] - 2026-03-22

### Changed
- Container images only built on tag releases (not on every push to main)
- Fix release workflow binary name (oxigraph-cloud, not oxigraph-server)

## [0.7.1] - 2026-03-22

### Fixed
- CI: scope tests and fmt checks to our crates, skip TiKV tests without live cluster
- Fix dead_code warning on TiKvConfig::with_scan_batch_size
- Remove unused import in backend_tests

## [0.7.0] - 2026-03-22

### Added
- HTTP transaction API (begin/commit/rollback with buffered multi-step writes)
- Changelog with undo support (opt-in via `--changelog`, stored in named graph)
- OpenTelemetry observability behind `otel` cargo feature (Prometheus `/metrics`, OTLP traces)
- CI/CD matrix builds for all 4 image variants (default, tikv, otel, tikv-otel) pushed to quay.io
- `EXTRA_FEATURES` build arg in Containerfiles for composable feature sets

### Changed
- Pin rudof crates to exact versions to avoid CI breakage from semver-compatible updates

## [0.6.0] - 2026-03-21

### Changed
- Enable SHACL validation in container images (both RocksDB and TiKV variants)
- Use stable Rust toolchain in Containerfiles instead of pinned 1.87.0

## [0.5.6] - 2026-03-18

### Added
- StorageBackend trait abstraction (Phase 1)
- TiKV distributed storage backend (Phase 2)
- SHACL validation via rudof with enforce/warn/off modes (Phase 3)
- Coprocessor pushdown for scan, filter, aggregate, bloom filter (Phase 4)
- Kubernetes manifests and Helm chart (Phase 5)
- OpenShift Route and SecurityContext support (Phase 6)
- Developer Sandbox deployment values (Phase 7)
- Write authentication via API key
- Health and readiness endpoints
- Docker Compose for local development
- Comprehensive documentation suite

### Security
- Write operations require Bearer token authentication
- Non-root container (UID 1001)
- UBI 9 minimal base image
