# ADR-001: Fork Strategy for Oxigraph

## Status: Accepted

## Context

We need to modify Oxigraph's storage layer to introduce a `StorageBackend` trait abstraction, replacing the hardcoded RocksDB/in-memory enum dispatch. This is deep surgery: the `oxigraph/src/storage/` module, the `Store` struct, and potentially the query evaluation pipeline all need changes. We must decide how to manage our relationship with upstream Oxigraph given the depth of these modifications and our need to track upstream bug fixes, SPARQL compliance improvements, and new features.

## Options Considered

### (a) Git Submodule

Add upstream Oxigraph as a git submodule and apply our changes on top.

- **Pros**: Clear separation between upstream and our modifications; easy to see what we changed; `git submodule update` pulls upstream.
- **Cons**: Submodules are notoriously painful in practice (detached HEAD states, CI complexity, developer friction). Merge conflicts during upstream sync will be frequent because we are modifying core modules, not just adding new code alongside. Submodules do not support patching -- we would need to maintain a fork within the submodule anyway, negating the benefit.

### (b) Patch Crate / Cargo `[patch]`

Use Cargo's `[patch.crates-io]` to override the `oxigraph` crate with a local or git-hosted modified version.

- **Pros**: Clean Cargo integration; downstream crates that depend on `oxigraph` automatically get our version; no workspace restructuring needed.
- **Cons**: Only works if we can keep the crate's public API surface identical, which is unlikely given the generic/dispatch changes to `Store`. The `[patch]` mechanism is designed for small, targeted fixes, not for refactoring an entire module. Upstream version bumps require re-applying all patches manually. No structured merge workflow.

### (c) Full Fork

Fork `oxigraph/oxigraph` into our own repository (or directory within this repo) and maintain it as a first-class dependency.

- **Pros**: Complete control over the codebase. We can restructure modules, add the `StorageBackend` trait, change `Store`'s type signature, and modify internal APIs freely. Upstream tracking is handled via standard git: add upstream as a remote, periodically merge or cherry-pick. This is the established pattern for projects that make deep architectural changes (e.g., how CockroachDB forked RocksDB into Pebble's early development, or how many TiKV components fork upstream Rust crates).
- **Cons**: Merge burden when pulling upstream changes. Risk of divergence if we don't maintain a disciplined merge cadence. We must own any bugs we introduce.

## Decision

**Full fork (option c).**

The modifications we are making are not superficial patches -- we are refactoring the entire storage abstraction, changing `Store`'s type signature, and potentially modifying the query evaluation pipeline for async support. Neither submodules nor cargo patches are designed for this level of modification.

The fork will be managed as follows:

1. Fork from a pinned stable Oxigraph release tag.
2. Add `upstream` as a git remote for periodic merges.
3. Keep our changes in clearly separated modules where possible (e.g., `storage/backend.rs` for the trait, `storage/tikv.rs` for the TiKV implementation) to minimize merge conflicts with upstream storage changes.
4. Establish a quarterly (or per-release) upstream merge cadence, with the conformance test suite (PLAN task 1.5) as the regression gate.
5. Engage upstream via the existing discussion (#1487) to explore whether the `StorageBackend` trait could eventually be upstreamed, which would eliminate the fork long-term.

## Consequences

- We own the Oxigraph codebase and must maintain it. CI must build and test our fork, not upstream.
- The Cargo workspace will reference our forked Oxigraph crates as path dependencies, not crates.io versions.
- Upstream SPARQL compliance fixes and performance improvements require explicit merge effort. The conformance test suite (task 1.5) and W3C SPARQL test suite (task 8.1) serve as regression safety nets.
- If the `StorageBackend` trait is accepted upstream, we can transition from a fork back to a dependency, but this is a long-term possibility, not a near-term expectation.
- Phase 0 task 0.1 should fork from a specific release tag (not `main`) to provide a stable baseline.
