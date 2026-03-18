# Security Audit Report

**Date:** 2026-03-18
**Tool:** cargo-audit (RustSec Advisory Database, 951 advisories loaded)
**Workspace:** oxigraph-cloud (553 crate dependencies scanned)

---

## Vulnerability Summary

| Severity | Count |
|----------|-------|
| Vulnerabilities | 1 |
| Warnings (unmaintained) | 3 |

---

## Vulnerabilities

### RUSTSEC-2024-0437: protobuf - Crash due to uncontrolled recursion

- **Crate:** `protobuf` v2.28.0
- **Severity:** Vulnerability
- **Fix:** Upgrade to >= 3.7.2
- **URL:** https://rustsec.org/advisories/RUSTSEC-2024-0437
- **Impact:** A crafted protobuf message can cause a stack overflow via uncontrolled recursion during parsing.
- **Dependency chain:** `protobuf` <- `prometheus` <- `tikv-client` <- `oxigraph-tikv` / `oxigraph`
- **Mitigation:** This is a transitive dependency pulled in by `tikv-client 0.3.0`. The fix requires `tikv-client` to update its `prometheus` dependency (which uses `protobuf` v2.x). Monitor the tikv-client repository for updates. In the interim, the risk is mitigated by the fact that protobuf messages are received from the TiKV cluster (trusted infrastructure), not from external user input.

---

## Warnings (Unmaintained Crates)

### RUSTSEC-2023-0089: atomic-polyfill (unmaintained)

- **Crate:** `atomic-polyfill` v1.0.3
- **Status:** Unmaintained since 2023-07-11
- **URL:** https://rustsec.org/advisories/RUSTSEC-2023-0089
- **Dependency chain:** `atomic-polyfill` <- `heapless` <- `rstar` <- `geo-types` <- `spargeo`
- **Impact:** Low. This crate provides atomic operation polyfills for platforms without native atomics. On x86_64/aarch64 targets used in this project, the polyfill code paths are not exercised.
- **Mitigation:** Will be resolved when upstream `rstar`/`heapless` crates remove this dependency.

### RUSTSEC-2024-0436: paste (unmaintained)

- **Crate:** `paste` v1.0.15
- **Status:** Unmaintained since 2024-10-07
- **URL:** https://rustsec.org/advisories/RUSTSEC-2024-0436
- **Dependency chain:** `paste` <- `rudof_rdf` <- `shacl_validation` / `shacl_rdf` / `shacl_ir` <- `oxigraph-shacl`
- **Impact:** Low. `paste` is a procedural macro crate used at compile time only; it does not execute at runtime.
- **Mitigation:** Monitor rudof project for updates removing the `paste` dependency.

### RUSTSEC-2025-0134: rustls-pemfile (unmaintained)

- **Crate:** `rustls-pemfile` v1.0.4
- **Status:** Unmaintained since 2025-11-28
- **URL:** https://rustsec.org/advisories/RUSTSEC-2025-0134
- **Dependency chain:** `rustls-pemfile` <- `tonic` <- `tikv-client`
- **Impact:** Low. PEM file parsing for TLS certificates. The crate is stable and functional but no longer receives updates.
- **Mitigation:** Will be resolved when `tikv-client` updates its `tonic` dependency to a newer version that uses `rustls-pemfile` v2.x+.

---

## Dependency License Check

### Workspace License

The workspace itself is dual-licensed under **MIT OR Apache-2.0** (see `Cargo.toml` `[workspace.package]`).

### Key Dependencies by License

| License | Notable Crates |
|---------|---------------|
| MIT OR Apache-2.0 | tokio, serde, clap, tracing, rayon, thiserror, anyhow, rand, regex |
| Apache-2.0 | tikv-client, tonic, prost, protobuf, prometheus |
| MIT | dashmap, siphasher, ryu-js |
| BSD-2-Clause / BSD-3-Clause | Various small utility crates |
| MIT OR Apache-2.0 | rudof crates (srdf, shacl_validation, shacl_ast, etc.) |
| Zlib OR Apache-2.0 OR MIT | flate2 |

### License Findings

- **No copyleft (GPL/LGPL/AGPL) licenses detected** in the direct dependency tree based on workspace Cargo.toml analysis.
- All direct dependencies use permissive licenses: MIT, Apache-2.0, BSD-2-Clause, BSD-3-Clause, ISC, or Zlib.
- The `tikv-client` dependency tree uses Apache-2.0 licensed crates (protobuf, grpcio ecosystem), which is compatible with the workspace's dual MIT/Apache-2.0 license.

### Recommendation

Run `cargo license --tsv` periodically (and in CI) to generate a complete machine-readable license manifest of all transitive dependencies. Consider adding a `deny.toml` configuration with [cargo-deny](https://github.com/EmbarkStudios/cargo-deny) for automated license policy enforcement.

---

## Recommended Actions

1. **High Priority:** Track `tikv-client` updates for the `protobuf` v2 -> v3 migration to resolve RUSTSEC-2024-0437.
2. **Medium Priority:** Add `cargo-deny` to CI for continuous license and advisory monitoring.
3. **Low Priority:** Monitor upstream crates for removal of `atomic-polyfill`, `paste`, and `rustls-pemfile` v1 dependencies.

---

## CI Integration

Security auditing is now integrated into the CI pipeline via `.github/workflows/ci.yml` in the `security` job, which runs `cargo audit` on every push to `main` and on all pull requests.
