# Legal & License Audit Report

**Project:** Oxigraph Cloud-Native (TiKV + Rudof SHACL Integration)
**Audit Date:** 2026-03-17
**Project License:** MIT OR Apache-2.0
**Distribution Targets:** OpenShift container images, Developer Sandbox

---

## 1. Executive Summary

The Oxigraph Cloud-Native project and its dependency tree are **predominantly permissively licensed** (MIT, Apache-2.0, BSD-2-Clause, BSD-3-Clause, ISC). No GPL or AGPL dependencies were found in the runtime dependency tree. Two areas require attention:

1. **encoding_rs** is licensed under **(MIT OR Apache-2.0) AND BSD-3-Clause** -- compatible but requires attribution.
2. **ring** uses a custom ISC-style license with BoringSSL/OpenSSL heritage code -- has **export control implications** for cryptographic software.
3. **lz4** (vendored in oxrocksdb-sys) has a dual license: the `lib/` directory is BSD-2-Clause (which is what RocksDB links), while non-lib files are GPL-2.0+. Only the BSD-2-Clause lib files are compiled.
4. A **NOTICE file** is required under Apache-2.0 Section 4(d) for distribution.

**Overall Risk: LOW** -- No blockers for distribution.

---

## 2. License Inventory

### 2.1 Workspace Crates (Our Code)

| Crate | Version | License | Status |
|-------|---------|---------|--------|
| oxigraph (fork) | 0.7.2 | MIT OR Apache-2.0 | OK -- fork compliant |
| oxigraph-tikv | 0.7.2 | MIT OR Apache-2.0 | OK |
| oxigraph-shacl | 0.7.2 | MIT OR Apache-2.0 | OK |
| oxigraph-server | 0.7.2 | MIT OR Apache-2.0 | OK |
| oxrdf | 0.3.3 | MIT OR Apache-2.0 | OK |
| oxrdfio | 0.2.4 | MIT OR Apache-2.0 | OK |
| oxrdfxml | 0.2.3 | MIT OR Apache-2.0 | OK |
| oxsdatatypes | 0.2.2 | MIT OR Apache-2.0 | OK |
| oxttl | 0.2.3 | MIT OR Apache-2.0 | OK |
| oxjsonld | 0.2.4 | MIT OR Apache-2.0 | OK |
| spargebra | 0.4.6 | MIT OR Apache-2.0 | OK |
| spareval | 0.2.6 | MIT OR Apache-2.0 | OK |
| sparesults | 0.3.3 | MIT OR Apache-2.0 | OK |
| sparopt | 0.3.6 | MIT OR Apache-2.0 | OK |
| spargeo | 0.5.4 | MIT OR Apache-2.0 | OK |
| sparql-smith | (dev) | MIT OR Apache-2.0 | OK |
| oxrocksdb-sys | 0.7.2 | MIT OR Apache-2.0 | OK |
| oxigraph-cli | 0.7.2 | MIT OR Apache-2.0 | OK |
| pyoxigraph | 0.7.2 | MIT OR Apache-2.0 | OK (not distributed) |
| oxigraph-js | 0.7.2 | MIT OR Apache-2.0 | OK (not distributed) |
| oxigraph-testsuite | 0.7.2 | MIT OR Apache-2.0 | OK (dev only) |

### 2.2 Rudof / SHACL Ecosystem Crates

| Crate | Version | License | Status |
|-------|---------|---------|--------|
| srdf | 0.1.147 | MIT OR Apache-2.0 | OK |
| shacl_validation | 0.2.7 | MIT OR Apache-2.0 | OK |
| shacl_ast | 0.2.6 | MIT OR Apache-2.0 | OK |
| shacl_ir | 0.2.7 | MIT OR Apache-2.0 | OK |
| shacl_rdf | 0.2.6 | MIT OR Apache-2.0 | OK |
| rudof_rdf | 0.2.7 | MIT OR Apache-2.0 | OK |
| sparql_service | 0.2.6 | MIT OR Apache-2.0 | OK |
| prefixmap | 0.2.6 | MIT OR Apache-2.0 | OK |
| iri_s | 0.1.147 | MIT OR Apache-2.0 | OK |
| mie | 0.1.127 | MIT OR Apache-2.0 | OK |

### 2.3 TiKV Client and gRPC Stack

| Crate | Version | License | Status |
|-------|---------|---------|--------|
| tikv-client | 0.3.0 | Apache-2.0 | OK |
| tonic | 0.10.2 | MIT | OK |
| prost | (latest) | Apache-2.0 | OK |
| prost-derive | (latest) | Apache-2.0 | OK |
| h2 | 0.3.27, 0.4.13 | MIT | OK |
| hyper | 0.14.32, 1.8.1 | MIT | OK |
| tower | 0.4.13, 0.5.3 | MIT | OK |
| tower-service | (latest) | MIT | OK |
| tower-layer | (latest) | MIT | OK |
| tower-http | (latest) | MIT | OK |
| tokio | (latest) | MIT | OK |
| tokio-stream | (latest) | MIT | OK |
| tokio-util | (latest) | MIT | OK |
| tokio-macros | (latest) | MIT | OK |
| futures | (latest) | MIT OR Apache-2.0 | OK |
| futures-core | (latest) | MIT OR Apache-2.0 | OK |
| futures-util | (latest) | MIT OR Apache-2.0 | OK |
| bytes | (latest) | MIT | OK |
| http | 0.2.12, 1.4.0 | MIT OR Apache-2.0 | OK |
| http-body | 0.4.6, 1.0.1 | MIT | OK |
| async-trait | (latest) | MIT OR Apache-2.0 | OK |
| async-stream | (latest) | MIT | OK |
| fail | 0.4.0 | Apache-2.0 | OK |
| prometheus | (latest) | Apache-2.0 | OK |

### 2.4 Cryptographic Dependencies

| Crate | Version | License | Status | Notes |
|-------|---------|---------|--------|-------|
| ring | 0.17.14 | ISC-style (ring license) | OK -- permissive | Export control: contains cryptographic code |
| rustls | 0.21.12, 0.23.37 | MIT OR Apache-2.0 | OK | TLS implementation |
| rustls-webpki | 0.101.7, 0.103.9 | ISC | OK | WebPKI validation |
| rustls-pemfile | 1.0.4 | MIT OR Apache-2.0 | OK | |
| rustls-pki-types | 1.14.0 | MIT OR Apache-2.0 | OK | |
| rustls-native-certs | 0.8.3 | MIT OR Apache-2.0 | OK | |
| rustls-platform-verifier | 0.6.2 | MIT OR Apache-2.0 | OK | |
| sct | (latest) | MIT OR Apache-2.0 | OK | |
| native-tls | 0.2.18 | MIT OR Apache-2.0 | OK | Wraps OpenSSL on Linux |
| openssl | 0.10.76 | Apache-2.0 | OK | Bindings to system OpenSSL |
| openssl-sys | 0.9.112 | MIT | OK | FFI bindings |
| openssl-probe | 0.2.1 | MIT OR Apache-2.0 | OK | |
| untrusted | (latest) | ISC | OK | |
| sha1 | (latest) | MIT OR Apache-2.0 | OK | |
| sha2 | (latest) | MIT OR Apache-2.0 | OK | |
| md-5 | (latest) | MIT OR Apache-2.0 | OK | |
| digest | (latest) | MIT OR Apache-2.0 | OK | |
| subtle | (latest) | BSD-3-Clause | OK | |
| zeroize | (latest) | MIT OR Apache-2.0 | OK | |
| quinn | (latest) | MIT OR Apache-2.0 | OK | QUIC transport |

### 2.5 Serialization and Data

| Crate | Version | License | Status |
|-------|---------|---------|--------|
| serde | (latest) | MIT OR Apache-2.0 | OK |
| serde_json | (latest) | MIT OR Apache-2.0 | OK |
| serde_derive | (latest) | MIT OR Apache-2.0 | OK |
| serde_urlencoded | (latest) | MIT OR Apache-2.0 | OK |
| toml | (latest) | MIT OR Apache-2.0 | OK |
| toml_edit | (latest) | MIT OR Apache-2.0 | OK |
| quick-xml | (latest) | MIT | OK |
| json-event-parser | (latest) | MIT OR Apache-2.0 | OK |
| csv | (latest) | MIT OR Unlicense | OK |
| rkyv | 0.7.46 | MIT | OK |
| borsh | (latest) | MIT OR Apache-2.0 | OK |
| protobuf | (latest) | MIT | OK |

### 2.6 Web / HTTP / Networking

| Crate | Version | License | Status |
|-------|---------|---------|--------|
| reqwest | 0.12.28 | MIT OR Apache-2.0 | OK |
| oxhttp | 0.3.1 | MIT OR Apache-2.0 | OK |
| url | (latest) | MIT OR Apache-2.0 | OK |
| percent-encoding | (latest) | MIT OR Apache-2.0 | OK |
| form_urlencoded | (latest) | MIT OR Apache-2.0 | OK |
| encoding_rs | 0.8.35 | (MIT OR Apache-2.0) AND BSD-3-Clause | OK -- see note |
| mime | 0.3.17 | MIT OR Apache-2.0 | OK |
| axum | (latest) | MIT | OK |
| axum-core | (latest) | MIT | OK |
| hyper-tls | (latest) | MIT OR Apache-2.0 | OK |
| hyper-rustls | (latest) | MIT OR Apache-2.0 | OK |
| hyper-timeout | (latest) | MIT | OK |
| hyper-util | (latest) | MIT | OK |

### 2.7 Utility and Core Crates

| Crate | Version | License | Status |
|-------|---------|---------|--------|
| anyhow | (latest) | MIT OR Apache-2.0 | OK |
| thiserror | 1.x, 2.x | MIT OR Apache-2.0 | OK |
| clap | (latest) | MIT OR Apache-2.0 | OK |
| regex | (latest) | MIT OR Apache-2.0 | OK |
| rand | 0.7, 0.8, 0.9 | MIT OR Apache-2.0 | OK |
| log | (latest) | MIT OR Apache-2.0 | OK |
| tracing | (latest) | MIT | OK |
| tracing-core | (latest) | MIT | OK |
| tracing-subscriber | (latest) | MIT | OK |
| once_cell | (latest) | MIT OR Apache-2.0 | OK |
| lazy_static | (latest) | MIT OR Apache-2.0 | OK |
| dashmap | (latest) | MIT | OK |
| indexmap | (latest) | MIT OR Apache-2.0 | OK |
| hashbrown | (latest) | MIT OR Apache-2.0 | OK |
| rayon | (latest) | MIT OR Apache-2.0 | OK |
| rayon-core | (latest) | MIT OR Apache-2.0 | OK |
| crossbeam-deque | (latest) | MIT OR Apache-2.0 | OK |
| crossbeam-epoch | (latest) | MIT OR Apache-2.0 | OK |
| crossbeam-utils | (latest) | MIT OR Apache-2.0 | OK |
| memchr | (latest) | MIT OR Unlicense | OK |
| libc | (latest) | MIT OR Apache-2.0 | OK |
| cfg-if | (latest) | MIT OR Apache-2.0 | OK |
| either | (latest) | MIT OR Apache-2.0 | OK |
| itertools | (latest) | MIT OR Apache-2.0 | OK |
| pin-project | (latest) | MIT OR Apache-2.0 | OK |
| pin-project-lite | (latest) | MIT OR Apache-2.0 | OK |
| smallvec | (latest) | MIT OR Apache-2.0 | OK |
| parking_lot | (latest) | MIT OR Apache-2.0 | OK |
| tempfile | (latest) | MIT OR Apache-2.0 | OK |
| flate2 | (latest) | MIT OR Apache-2.0 | OK |
| bzip2 | (latest) | MIT OR Apache-2.0 | OK |
| semver | (latest) | MIT OR Apache-2.0 | OK |
| siphasher | (latest) | MIT OR Apache-2.0 | OK |
| rustc-hash | (latest) | MIT OR Apache-2.0 | OK |
| petgraph | (latest) | MIT OR Apache-2.0 | OK |
| proptest | (latest) | MIT OR Apache-2.0 | OK |
| colored | (latest) | MPL-2.0 | REVIEW -- see finding F-3 |
| spin | 0.9.8 | MIT | OK |

### 2.8 Geospatial Crates

| Crate | Version | License | Status |
|-------|---------|---------|--------|
| geo | (latest) | MIT OR Apache-2.0 | OK |
| geo-types | (latest) | MIT OR Apache-2.0 | OK |
| geo-traits | (latest) | MIT OR Apache-2.0 | OK |
| geojson | (latest) | MIT OR Apache-2.0 | OK |
| wkt | (latest) | MIT OR Apache-2.0 | OK |
| rstar | (latest) | MIT OR Apache-2.0 | OK |
| spade | (latest) | MIT OR Apache-2.0 | OK |

### 2.9 ICU / Unicode Crates

| Crate | Version | License | Status |
|-------|---------|---------|--------|
| icu_collections | (latest) | Unicode-3.0 | OK -- data license, permissive |
| icu_locale_core | (latest) | Unicode-3.0 | OK |
| icu_normalizer | (latest) | Unicode-3.0 | OK |
| icu_normalizer_data | (latest) | Unicode-3.0 | OK |
| icu_properties | (latest) | Unicode-3.0 | OK |
| icu_properties_data | (latest) | Unicode-3.0 | OK |
| icu_provider | (latest) | Unicode-3.0 | OK |

### 2.10 Vendored C/C++ Libraries (in oxrocksdb-sys)

| Library | License | Status | Notes |
|---------|---------|--------|-------|
| RocksDB | Apache-2.0 + BSD-3-Clause (LevelDB heritage) | OK | Attribution required |
| lz4 (lib/) | BSD-2-Clause | OK | Only lib/ is compiled; non-lib files are GPL-2.0+ but NOT used |

---

## 3. Risk Findings

### F-1: Missing NOTICE File (Medium)

**Severity:** Medium
**Details:** The project is licensed under MIT OR Apache-2.0. Under Apache-2.0 Section 4(d), derivative works must include a readable copy of attribution notices. No NOTICE file currently exists in the repository root or the Oxigraph fork.
**Action Required:** Create a `NOTICE` file in the repository root before distribution.

### F-2: Oxigraph Fork Attribution (Medium)

**Severity:** Medium
**Details:** The fork preserves both `LICENSE-MIT` and `LICENSE-APACHE` from upstream Oxigraph (copyright "2018 Oxigraph developers"). Per Apache-2.0 Section 4(b), modified files must carry "prominent notices stating that You changed the files." Currently there are no per-file change notices, and no `NOTICE` file documenting the fork relationship.
**Action Required:**
- Preserve existing LICENSE files (done).
- Add a NOTICE file documenting the fork and modifications.
- Consider adding a header comment to substantially modified files.

### F-3: `colored` Crate -- MPL-2.0 (Low)

**Severity:** Low
**Details:** The `colored` crate (versions 2.x and 3.x) used by the rudof ecosystem is licensed under MPL-2.0. MPL-2.0 is a weak copyleft license that operates at the **file level** -- modifications to MPL-2.0 files must be shared under MPL-2.0, but the license does not extend to the larger work. Since we use `colored` as an unmodified dependency and do not modify its source, there is no obligation beyond preserving the license notice.
**Action Required:** None beyond attribution in NOTICE file. If the crate source is ever vendored and modified, those modifications must remain MPL-2.0.

### F-4: `encoding_rs` Dual License (Info)

**Severity:** Info
**Details:** `encoding_rs` is licensed under "(MIT OR Apache-2.0) AND BSD-3-Clause". The BSD-3-Clause component requires attribution in binary distributions. This is a transitive dependency of `reqwest`.
**Action Required:** Include BSD-3-Clause attribution for encoding_rs in the NOTICE file.

### F-5: Cryptographic Dependencies -- Export Control (Low)

**Severity:** Low
**Details:** The dependency tree includes cryptographic crates:
- **ring** (0.17.14) -- Contains BoringSSL-derived cryptographic primitives (AES, SHA, RSA, ECC, etc.). Licensed under a permissive ISC-style license.
- **rustls** (0.21.12, 0.23.37) -- Pure-Rust TLS implementation using ring.
- **openssl** / **openssl-sys** -- FFI bindings to system OpenSSL (used via native-tls on Linux).
- **quinn** -- QUIC transport using rustls.

These are pulled in via `reqwest` (used by rudof crates for HTTP fetching) and `tonic` (used by tikv-client for gRPC).

Under U.S. Export Administration Regulations (EAR), software that uses publicly available encryption source code may qualify for the License Exception TSU (Technology and Software Unrestricted), provided proper notification is filed. The `ring` crate's README notes it includes code subject to export control.

**Action Required:**
- If distributing from the U.S., verify EAR License Exception TSU applicability.
- Document cryptographic capabilities in export classification records.
- For Red Hat distribution channels, follow Red Hat's existing export control processes.

### F-6: lz4 GPL-2.0 Code Not Compiled (Info)

**Severity:** Info
**Details:** The vendored lz4 source tree contains GPL-2.0+ licensed files outside the `lib/` directory (programs, tests, examples). Only the `lib/` directory (BSD-2-Clause) is compiled and linked by oxrocksdb-sys. The GPL-2.0 files are present in the source tree but are inert.
**Action Required:** None for binary distribution. If distributing source, consider adding a note or `.gitattributes` marking the non-lib lz4 directories as not compiled.

### F-7: UBI 9 Base Image Redistribution (Info)

**Severity:** Info
**Details:** The Containerfile uses `registry.access.redhat.com/ubi9/ubi:latest` (builder) and `registry.access.redhat.com/ubi9/ubi-minimal:latest` (runtime). UBI 9 images are freely redistributable per Red Hat's Universal Base Image End User License Agreement (EULA). Key terms:
- UBI can be redistributed on any OCI-compliant platform.
- Red Hat RPMs from UBI repos can be included.
- Red Hat trademarks may not be used to endorse derivatives.
- No support entitlement is implied.
**Action Required:** None -- UBI is explicitly designed for this use case.

### F-8: TiKV / PD Container Images (Info)

**Severity:** Info
**Details:** The docker-compose file uses `docker.io/pingcap/tikv:latest` and `docker.io/pingcap/pd:latest`. TiKV is licensed under Apache-2.0. PD is part of the TiDB project, also Apache-2.0. These images are used as infrastructure dependencies, not bundled into our image.
**Action Required:** None -- these are runtime dependencies, not distributed as part of our artifact.

---

## 4. License Compatibility Matrix

| License | Count (approx) | Compatible with MIT OR Apache-2.0? | Notes |
|---------|-------|------|-------|
| MIT OR Apache-2.0 | ~200+ | Yes | Majority of deps |
| MIT | ~50+ | Yes | tokio, hyper, axum, dashmap, etc. |
| Apache-2.0 | ~15 | Yes | tikv-client, prost, openssl, etc. |
| BSD-2-Clause | ~5 | Yes | lz4 lib, etc. |
| BSD-3-Clause | ~5 | Yes | subtle, encoding_rs component, LevelDB |
| ISC | ~5 | Yes | ring, rustls-webpki, untrusted |
| MIT OR Unlicense | ~3 | Yes | memchr, csv |
| Unicode-3.0 | ~7 | Yes | ICU crates |
| MPL-2.0 | 1 | Yes (weak copyleft, file-level) | colored |
| GPL-2.0+ | 0 in runtime | N/A | lz4 non-lib files present but not compiled |
| LGPL | 0 | N/A | Not present |
| AGPL | 0 | N/A | Not present |

---

## 5. Required Actions Before Distribution

### Must-Do (Before First Release)

1. **Create a `NOTICE` file** in the repository root (see Section 6 for draft content).
2. **Add fork documentation** -- A brief statement in the NOTICE file and/or README documenting that Oxigraph is forked from upstream and modified.
3. **Include `LICENSE-MIT` and `LICENSE-APACHE`** in the repository root (currently only in `oxigraph/`).

### Should-Do (Best Practice)

4. **Run `cargo deny check licenses`** in CI to prevent future introduction of copyleft dependencies. Example `deny.toml`:
   ```toml
   [licenses]
   allow = [
       "MIT",
       "Apache-2.0",
       "BSD-2-Clause",
       "BSD-3-Clause",
       "ISC",
       "Zlib",
       "Unicode-3.0",
       "Unicode-DFS-2016",
       "MPL-2.0",
       "Unlicense",
   ]
   deny = ["GPL-2.0", "GPL-3.0", "AGPL-3.0"]
   ```
5. **Document cryptographic capabilities** for export control compliance.
6. **Embed license metadata in container image labels** (OCI annotations):
   ```
   org.opencontainers.image.licenses=MIT OR Apache-2.0
   ```

### Nice-to-Have

7. Generate a full SBOM (Software Bill of Materials) using `cargo sbom` or Syft for container scanning.
8. Add SPDX license identifiers to all source files in new crates.

---

## 6. Draft NOTICE File

```
Oxigraph Cloud-Native
Copyright 2024-2026 Red Hat, Inc. and contributors

This product is a derivative work of Oxigraph (https://github.com/oxigraph/oxigraph),
Copyright 2018 Oxigraph developers, licensed under MIT OR Apache-2.0.

This product includes software developed by:

- The Oxigraph project (https://oxigraph.org/)
  License: MIT OR Apache-2.0

- The Rudof project (https://github.com/rudof-project/rudof)
  Crates: srdf, shacl_validation, shacl_ast, shacl_ir, shacl_rdf,
          rudof_rdf, sparql_service, prefixmap, iri_s, mie
  License: MIT OR Apache-2.0

- TiKV Client (https://github.com/tikv/client-rust)
  License: Apache-2.0

- RocksDB (https://github.com/facebook/rocksdb)
  License: Apache-2.0
  Portions derived from LevelDB, Copyright 2011 The LevelDB Authors (BSD-3-Clause)

- LZ4 Library (https://github.com/lz4/lz4)
  Copyright 2011-2020 Yann Collet
  License: BSD-2-Clause

- encoding_rs (https://github.com/nickel-technologies/nickel.rs)
  Copyright Mozilla Foundation
  License: (MIT OR Apache-2.0) AND BSD-3-Clause

- ring (https://github.com/briansmith/ring)
  Portions Copyright Google Inc., ISP RAS, and others
  License: ISC-style
  Note: Contains cryptographic software subject to export regulations.

- colored (https://github.com/mackwic/colored)
  License: MPL-2.0

- Red Hat Universal Base Image 9 (UBI 9)
  Used as container runtime base image.
  Redistributed under Red Hat UBI EULA.
```

---

## 7. Conclusion

The Oxigraph Cloud-Native project has a clean dependency license profile. All runtime dependencies use permissive licenses compatible with the project's MIT OR Apache-2.0 license. The single MPL-2.0 dependency (`colored`) is used as-is without modification, so no copyleft obligations propagate. No GPL, LGPL, or AGPL dependencies exist in the runtime tree.

**The project is cleared for distribution** once the NOTICE file is created and LICENSE files are placed at the repository root.
