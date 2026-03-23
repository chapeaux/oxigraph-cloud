# Security & Compliance Assessment: Oxigraph Cloud-Native

> **Date**: 2026-03-17 | **Assessor**: Security & Compliance Agent
> **Scope**: Full-stack assessment covering dependencies, container, deployment, data security, SPARQL endpoint, and operational security.
> **Project Version**: 0.7.2 | **Rust Toolchain**: stable

---

## Table of Contents

1. [Executive Summary](#1-executive-summary)
2. [Security Findings Table](#2-security-findings-table)
3. [Detailed Findings](#3-detailed-findings)
4. [Compliance Checklist](#4-compliance-checklist)
5. [Risk Matrix](#5-risk-matrix)
6. [Remediation Roadmap](#6-remediation-roadmap)

---

## 1. Executive Summary

This assessment covers the Oxigraph Cloud-Native project across eight security domains: dependency vulnerabilities, container security, supply chain integrity, data security (at rest and in transit), OWASP Top 10 for the SPARQL endpoint, OpenShift deployment security, secrets management, and logging/audit trail.

**Key findings**:
- 1 Critical finding: No authentication/authorization on the SPARQL endpoint (including write operations)
- 2 High findings: Known vulnerability in `protobuf` crate; no data-at-rest encryption configuration
- 5 Medium findings: Missing NetworkPolicy, no rate limiting, no structured logging/audit trail, CORS wildcard, no query timeout per request
- 4 Low/Informational findings: Unmaintained transitive dependencies, missing `cargo-deny` at workspace root, image tag `latest` in values.yaml, no seccomp profile

The container image and Helm chart demonstrate strong security posture with non-root user, read-only root filesystem, dropped capabilities, and proper SecurityContext. The most critical gap is the complete absence of authentication on the SPARQL endpoint, which allows unauthenticated writes and deletes.

---

## 2. Security Findings Table

| ID | Category | Severity | Description | Remediation |
|----|----------|----------|-------------|-------------|
| SEC-01 | SPARQL Endpoint | **Critical** | No authentication or authorization on any endpoint, including `/update` (writes), `/store` (bulk load), `/shacl/shapes` (DELETE), and `/shacl/mode` (PUT). Any network-reachable client can modify or delete data. | Add authentication middleware (OAuth2/OIDC, mTLS client certs, or API key). At minimum, separate read and write endpoints with distinct auth requirements. |
| SEC-02 | Dependencies | **High** | `protobuf 2.28.0` has known vulnerability RUSTSEC-2024-0437 (uncontrolled recursion causing crash). Pulled in via `tikv-client 0.3.0 -> prometheus 0.13.4`. | Upgrade `tikv-client` to a version that depends on `protobuf >= 3.7.2`, or patch via `[patch.crates-io]` in workspace `Cargo.toml`. |
| SEC-03 | Data Security | **High** | No TiKV Transparent Data Encryption (TDE) configuration documented or implemented. The TiKV operations guide (`docs/tikv-operations-guide.md`) contains no mention of encryption at rest, TLS, or mTLS. Data stored on PersistentVolumes is unencrypted by default. | Add `[security]` section to `tikv.toml` with TDE configuration. Document encryption-at-rest requirements. Use StorageClass-level encryption (e.g., LUKS) as defense in depth. |
| SEC-04 | Network Security | **Medium** | No NetworkPolicy resource is included in the Helm chart templates. Only the testing/hardening plan doc proposes one, but it is not shipped. All pods can communicate freely within the namespace and potentially across namespaces. | Add `networkpolicy.yaml` to `helm/oxigraph-cloud/templates/` based on the policy defined in `docs/testing-hardening-plan.md` section 4.5. |
| SEC-05 | SPARQL Endpoint | **Medium** | No rate limiting or request throttling. The server uses `available_parallelism() * 128` max concurrent connections with a flat 60-second timeout. A single client can exhaust server resources with expensive SPARQL queries. | Implement per-client rate limiting. Add configurable query timeout (currently global 60s). Consider a query complexity estimator to reject overly broad patterns. |
| SEC-06 | Logging & Audit | **Medium** | No structured logging framework. Logging is limited to `eprintln!` statements for startup messages and internal server errors. No SPARQL query logging, no mutation audit trail, no access logs, no security event logging. | Integrate `tracing` crate with structured JSON output. Log all write operations (UPDATE, store POST, SHACL shape changes) with timestamps, source IP, and operation details. |
| SEC-07 | SPARQL Endpoint | **Medium** | CORS is configured with wildcard `Access-Control-Allow-Origin: *` when `--cors` flag is set. The Containerfile CMD includes `--cors` by default. This allows any web origin to issue authenticated requests if auth is later added. | Remove `--cors` from default CMD. When CORS is needed, support configurable allowed origins rather than wildcard. |
| SEC-08 | SPARQL Endpoint | **Medium** | No per-query timeout configuration. The global HTTP timeout is 60 seconds, which may be insufficient to prevent resource exhaustion from complex CONSTRUCT or property path queries that could run for minutes. | Add a `--query-timeout` CLI flag. Pass it to `SparqlEvaluator` to enforce per-query execution limits. |
| SEC-09 | Dependencies | **Low** | Three unmaintained transitive dependencies: `atomic-polyfill 1.0.3` (RUSTSEC-2023-0089), `paste 1.0.15` (RUSTSEC-2024-0436), `rustls-pemfile 1.0.4` (RUSTSEC-2025-0134). None have known exploitable vulnerabilities, but they will not receive security patches. | Monitor upstream for replacements. `rustls-pemfile` and `paste` are transitive via `tikv-client` and `rudof_rdf`; upgrading those should resolve it. |
| SEC-10 | Supply Chain | **Low** | `cargo-deny` configuration exists at `oxigraph/deny.toml` (upstream fork) but not at the workspace root `/home/ldary/rh/oxigraph-k8s/deny.toml`. The cloud-native extension crates are not covered by license/advisory/ban checks. RUSTSEC-2018-0015 is explicitly ignored in the upstream config. | Create a workspace-level `deny.toml` that covers all crates including `oxigraph-tikv`, `oxigraph-shacl`, and `oxigraph-server`. Review whether RUSTSEC-2018-0015 ignore is still justified. |
| SEC-11 | Container | **Low** | Default Helm values use `image.tag: latest`, which is not reproducible and may pull different images over time. | Pin to a specific digest or semantic version tag. Use `image.tag: "0.5.6"` or a SHA256 digest. |
| SEC-12 | Deployment | **Info** | No seccomp profile specified in the pod or container SecurityContext. OpenShift defaults may apply, but explicit configuration is preferred for compliance. | Add `seccompProfile: { type: RuntimeDefault }` to `podSecurityContext` in `values.yaml`. |
| SEC-13 | Data Security | **Info** | No mTLS configuration between Oxigraph and TiKV is implemented. The testing-hardening plan documents the approach (section 4.6) but it is not wired into the Helm chart or server code. | Implement mTLS support in `oxigraph-tikv` crate using `tikv_client::Config::with_security()`. Add TLS secret volume mounts to the Helm chart. |
| SEC-14 | SPARQL Endpoint | **Info** | The `/store` POST endpoint does not enforce body size limits through `limited_body()` -- it reads directly from `request.body_mut()` without the `MAX_SPARQL_BODY_SIZE` check. A malicious client could upload arbitrarily large RDF files. | Route `/store` POST through `limited_body()` or implement a separate configurable limit for bulk loads. |
| SEC-15 | Secrets Mgmt | **Info** | The backup CronJob in `docs/tikv-operations-guide.md` includes a Secret manifest with placeholder credentials (`REPLACE_WITH_ACCESS_KEY`). While this is documentation, committing secret templates with placeholder values risks accidental use. | Use `ExternalSecret` or `SealedSecret` patterns. Remove placeholder values from documentation manifests. |

---

## 3. Detailed Findings

### 3.1 Dependency Vulnerability Audit

**Tool**: `cargo audit` against `Cargo.lock` (551 crate dependencies)

| Advisory | Crate | Version | Severity | Status |
|----------|-------|---------|----------|--------|
| RUSTSEC-2024-0437 | `protobuf` | 2.28.0 | Vulnerability (crash via recursion) | **Action required** |
| RUSTSEC-2023-0089 | `atomic-polyfill` | 1.0.3 | Unmaintained | Monitor |
| RUSTSEC-2024-0436 | `paste` | 1.0.15 | Unmaintained | Monitor |
| RUSTSEC-2025-0134 | `rustls-pemfile` | 1.0.4 | Unmaintained | Monitor |

The `protobuf` vulnerability is the most concerning. It is reachable via `tikv-client -> prometheus -> protobuf`. The `protobuf` crate is used for Prometheus metric serialization within the TiKV client. An attacker who can send crafted protobuf messages could cause a stack overflow crash. The fix requires `protobuf >= 3.7.2`, but `tikv-client 0.3.0` pins to `protobuf 2.x`. This is blocked on upstream `tikv-client` release.

**Cargo.lock**: Both `Cargo.lock` files are committed (workspace root and `oxigraph/` subdirectory), which is correct for reproducible builds.

**Dependency pinning**: The workspace `Cargo.toml` uses semver ranges for most dependencies (e.g., `anyhow = "1.0.72"`, `tokio = "1.29"`). Internal crates use exact version pins (`= "0.5.6"`). The semver range approach is standard for applications but means `cargo update` can pull in new minor/patch versions. This is acceptable given the lockfile is committed.

### 3.2 Container Security Review

**File**: `/home/ldary/rh/oxigraph-k8s/Containerfile`

| Check | Status | Details |
|-------|--------|---------|
| Multi-stage build | PASS | Builder (UBI 9) + Runtime (UBI 9 minimal) |
| Minimal base image | PASS | `ubi9/ubi-minimal` -- Red Hat certified, minimal attack surface |
| Non-root user | PASS | `USER 1001`, created with `useradd -r -u 1001 -g 0` |
| OpenShift UID compatibility | PASS | Group `0` (root group) with `chmod -R g=u` for arbitrary UID support |
| No secrets baked in | PASS | No credentials, tokens, or keys in the image |
| Binary stripped | PASS | `strip /usr/local/bin/oxigraph-cloud` reduces binary size and removes debug symbols |
| Layer hygiene | PASS | `dnf clean all` / `microdnf clean all` in each RUN layer |
| HTTPS-only Rust install | PASS | `curl --proto '=https' --tlsv1.2` |
| Pinned Rust version | PASS | `--default-toolchain 1.87.0` |
| No unnecessary packages | PASS | Runtime installs only `shadow-utils` (for useradd) and `libstdc++` |
| Labels | PASS | Proper OCI/OpenShift labels (`io.k8s.display-name`, `io.openshift.tags`) |

**Concern**: The builder stage installs `git` which could be used for supply chain attacks during build if the network is not controlled. Consider using `--mount=type=cache` for cargo registry instead.

### 3.3 Supply Chain Security

| Check | Status | Details |
|-------|--------|---------|
| `Cargo.lock` committed | PASS | Present at workspace root and `oxigraph/` |
| `cargo-deny` configured | PARTIAL | Exists at `oxigraph/deny.toml` but not at workspace root |
| License allow-list | PASS (upstream) | Covers Apache-2.0, MIT, BSD-3-Clause, ISC, MPL-2.0, etc. |
| Unknown registry denied | PASS | `unknown-registry = "deny"` in deny.toml |
| Reproducible builds | PASS | Pinned Rust toolchain (1.87.0), committed lockfile, LTO enabled |
| Release profile hardening | PASS | `lto = true`, `codegen-units = 1`, `panic = "abort"` |
| Unsafe code lint | PASS | `unsafe_code = "warn"` in workspace lints |

### 3.4 Data Security Assessment

| Domain | Status | Details |
|--------|--------|---------|
| Data at rest (TiKV TDE) | **NOT CONFIGURED** | The TiKV operations guide contains no `[security]` section for encryption. TiKV supports TDE via RocksDB encryption, but it must be explicitly configured. |
| Data at rest (K8s volumes) | **DEPENDS ON CLUSTER** | StorageClass may or may not provide encryption. Not specified in Helm values. |
| Data in transit (Oxigraph to TiKV) | **NOT IMPLEMENTED** | The testing-hardening plan documents mTLS setup (section 4.6) with cert-manager, but neither the `oxigraph-tikv` crate nor the Helm chart implements it. All gRPC traffic between Oxigraph and TiKV is plaintext. |
| Data in transit (client to Oxigraph) | **PARTIAL** | The OpenShift Route template supports TLS termination (`edge`), but it is disabled by default (`route.enabled: false`). When enabled, TLS terminates at the router; traffic from router to pod is HTTP. |
| SPARQL endpoint auth | **NONE** | See SEC-01. No authentication mechanism exists. |

### 3.5 OWASP Top 10 Assessment for SPARQL Endpoint

**File**: `/home/ldary/rh/oxigraph-k8s/crates/oxigraph-server/src/main.rs`

| OWASP Category | Risk Level | Assessment |
|----------------|-----------|------------|
| A01: Broken Access Control | **Critical** | No authentication or authorization. All endpoints (read + write + admin) are fully open. |
| A02: Cryptographic Failures | **High** | No TLS between Oxigraph and TiKV. Route TLS is optional. No encryption at rest. |
| A03: Injection | **Low** | SPARQL is parsed via `spargebra` into a typed AST before evaluation. No string concatenation with user data. Content-Type is validated. SPARQL injection is effectively mitigated by the parser. |
| A04: Insecure Design | **Medium** | No separation between read and write endpoints at the authorization level. The SHACL management endpoints (`/shacl/mode PUT`, `/shacl/shapes DELETE`) are admin operations exposed without protection. |
| A05: Security Misconfiguration | **Medium** | CORS wildcard enabled by default in Containerfile CMD. Server name header exposes version (`OxigraphCloud/0.5.6`). |
| A06: Vulnerable Components | **High** | `protobuf 2.28.0` has a known DoS vulnerability. Three unmaintained transitive deps. |
| A07: Identification & Auth Failures | **Critical** | No authentication mechanism at all. |
| A08: Software & Data Integrity | **Low** | Cargo.lock is committed. Build uses pinned Rust version. Image uses Red Hat UBI base. |
| A09: Security Logging & Monitoring | **High** | No access logging, no query logging, no audit trail. Only `eprintln!` for errors. No integration with centralized logging. |
| A10: SSRF | **Low** | No SPARQL `SERVICE` clause handling observed in `main.rs`. Federated query support in upstream Oxigraph could be an SSRF vector if enabled -- would need review of `SparqlEvaluator` configuration to confirm whether SERVICE is restricted. |

**Body size limits**: The SPARQL query/update endpoints enforce a 128 MB body limit via `limited_body()`. However, the `/store` POST endpoint reads directly from `request.body_mut()` via `store.load_from_reader()` without size enforcement (SEC-14).

**XSS in HTML responses**: The root page (`/`) returns static HTML with no user-controlled content. Query results are returned as application/json or application/sparql-results+xml, not HTML. XSS risk is minimal.

### 3.6 OpenShift Deployment Security

**File**: `/home/ldary/rh/oxigraph-k8s/helm/oxigraph-cloud/templates/statefulset.yaml`

| Check | Status | Details |
|-------|--------|---------|
| `runAsNonRoot` | PASS | `podSecurityContext.runAsNonRoot: true` |
| `runAsUser` | PASS | `1001` (non-root) |
| `allowPrivilegeEscalation` | PASS | `false` |
| `capabilities.drop` | PASS | `ALL` capabilities dropped |
| `readOnlyRootFilesystem` | PASS | `true` -- writable `/tmp` via emptyDir, `/data` via PVC |
| Resource limits | PASS | Both `requests` and `limits` defined (100m/256Mi to 1/1Gi) |
| Resource requests | PASS | CPU and memory requests set |
| Liveness probe | PASS | HTTP GET `/` with sensible timings |
| Readiness probe | PASS | HTTP GET `/` (actually probes store accessibility via `/ready` would be better) |
| Config checksum annotation | PASS | Forces pod restart on ConfigMap changes |
| Volume mounts | PASS | Data on PVC, config as read-only ConfigMap, `/tmp` as emptyDir |
| RBAC | **MISSING** | No ServiceAccount, Role, or RoleBinding templates in the Helm chart |
| NetworkPolicy | **MISSING** | No NetworkPolicy template (SEC-04) |
| PodDisruptionBudget | **MISSING** | No PDB template for high availability |
| Seccomp profile | **MISSING** | No seccompProfile in SecurityContext (SEC-12) |

**Readiness probe concern**: The readiness probe checks `/` which returns static HTML. The `/ready` endpoint exists and actually probes the store, but it is not used by the probe. If the store is unhealthy, the readiness probe would still pass.

### 3.7 Secrets Management

| Secret | Current State | Recommended |
|--------|--------------|-------------|
| TiKV PD endpoints | CLI argument / Helm value (not secret) | ConfigMap -- OK |
| TiKV mTLS certificates | Not implemented | Kubernetes Secret + cert-manager auto-rotation |
| S3 backup credentials | Kubernetes Secret (documented with placeholder values) | ExternalSecret (Vault/AWS Secrets Manager) or SealedSecret |
| Image pull secrets | Optional Helm value, empty by default | Kubernetes Secret -- OK |
| SPARQL endpoint auth tokens | Not implemented | Kubernetes Secret when auth is added |

No external secrets management (HashiCorp Vault, AWS Secrets Manager, etc.) is integrated. When mTLS and authentication are implemented, a proper secrets rotation strategy will be needed.

### 3.8 Logging and Audit Trail

| What | Logged? | Details |
|------|---------|---------|
| Server startup | Yes | `eprintln!` with bind address and backend info |
| SPARQL queries (SELECT) | **No** | No query text or metadata logged |
| SPARQL mutations (UPDATE) | **No** | No mutation logging at all |
| Bulk data loads (/store POST) | **No** | No logging of data ingestion |
| SHACL shape changes | **No** | No logging of shape uploads, deletes, or mode changes |
| Internal server errors | Partial | `eprintln!` in `internal_server_error()` -- no structured format |
| Access logs (IP, method, path, status) | **No** | No HTTP access logging |
| Authentication events | N/A | No auth implemented |
| TiKV connection events | **No** | No connection pool or reconnect logging |

The absence of logging means there is no forensic capability for security incidents, no ability to detect anomalous query patterns, and no compliance with audit requirements that mandate data access logging.

---

## 4. Compliance Checklist

### 4.1 General Security Standards

| Control | Standard | Status | Notes |
|---------|----------|--------|-------|
| Authentication required for data access | OWASP, SOC 2, NIST 800-53 (IA-2) | FAIL | No auth on any endpoint |
| Authorization/RBAC for write operations | OWASP, SOC 2, NIST 800-53 (AC-3) | FAIL | No authorization controls |
| Encryption at rest | SOC 2, HIPAA, NIST 800-53 (SC-28) | FAIL | TiKV TDE not configured |
| Encryption in transit | SOC 2, HIPAA, NIST 800-53 (SC-8) | PARTIAL | TLS on Route only; no mTLS to TiKV |
| Audit logging | SOC 2, HIPAA, NIST 800-53 (AU-2) | FAIL | No meaningful logging |
| Vulnerability management | SOC 2, NIST 800-53 (RA-5) | PARTIAL | cargo audit available but not in CI; known vuln in protobuf |
| Least privilege | NIST 800-53 (AC-6) | PARTIAL | Container runs as non-root; no RBAC in Helm chart |
| Network segmentation | NIST 800-53 (SC-7) | FAIL | No NetworkPolicy deployed |
| Incident response logging | SOC 2, NIST 800-53 (IR-4) | FAIL | Insufficient logging for incident response |
| Backup and recovery | SOC 2, NIST 800-53 (CP-9) | PASS | TiKV BR documented with CronJob, PITR, and verification steps |
| Container hardening | CIS Benchmarks | PASS | Non-root, read-only FS, dropped caps, minimal base |
| Supply chain integrity | SLSA Level 1 | PARTIAL | Lockfile committed; no SBOM generation or provenance attestation |

### 4.2 GDPR Considerations

If RDF data contains personal information (PII):
- **Right to erasure**: SPARQL UPDATE supports `DELETE WHERE` but there is no mechanism to ensure complete deletion across all 9 index copies and the `id2str` dictionary. TiKV's MVCC means old versions persist until GC.
- **Data minimization**: No built-in mechanism to restrict stored data to what is necessary.
- **Consent tracking**: No audit trail of who accessed what data.
- **Data breach notification**: No logging means breaches may go undetected.

### 4.3 FedRAMP / HIPAA Notes

The project is not currently suitable for FedRAMP or HIPAA environments due to the absence of authentication, encryption at rest, and audit logging. These would need to be addressed before hosting regulated data.

---

## 5. Risk Matrix

Top 5 risks ranked by **Likelihood x Impact** (each scored 1-5):

| Rank | Risk | Likelihood | Impact | Score | Finding |
|------|------|-----------|--------|-------|---------|
| 1 | **Unauthorized data modification or deletion** via unauthenticated SPARQL UPDATE/DELETE endpoint | 5 (certain if exposed) | 5 (data loss/corruption) | **25** | SEC-01 |
| 2 | **Data exfiltration** via unauthenticated SPARQL queries on sensitive RDF data | 5 (certain if exposed) | 4 (data breach) | **20** | SEC-01 |
| 3 | **Denial of service** via resource-exhausting SPARQL queries (no rate limiting, no per-query timeout, protobuf crash vuln) | 4 (likely) | 3 (service outage) | **12** | SEC-05, SEC-08, SEC-02 |
| 4 | **Data exposure via network sniffing** -- plaintext gRPC between Oxigraph and TiKV, plaintext data on disk | 3 (moderate in shared K8s cluster) | 4 (data breach) | **12** | SEC-03, SEC-13 |
| 5 | **Undetected security incident** due to absence of logging and audit trail | 4 (likely) | 3 (delayed response, compliance failure) | **12** | SEC-06 |

---

## 6. Remediation Roadmap

### Phase 1: Critical (Week 1-2)

| Priority | Finding | Action | Effort |
|----------|---------|--------|--------|
| P0 | SEC-01 | Add authentication middleware to the SPARQL server. Options: (a) OAuth2/OIDC token validation via middleware, (b) mTLS client certificates, (c) API key header validation. Separate read-only and read-write roles. | 3-5 days |
| P0 | SEC-01 | As an immediate mitigation, ensure the SPARQL endpoint is not exposed externally. Verify the OpenShift Route is disabled by default (it is). Add a warning to deployment documentation. | 1 day |

### Phase 2: High (Week 2-4)

| Priority | Finding | Action | Effort |
|----------|---------|--------|--------|
| P1 | SEC-02 | Investigate `tikv-client` upgrade path for `protobuf >= 3.7.2`. If blocked upstream, evaluate `[patch.crates-io]` workaround or pinning a fixed version. | 1-2 days |
| P1 | SEC-03 | Configure TiKV TDE in `tikv.toml`: add `[security.encryption]` section with AES-256-CTR. Document key management approach (file-based, KMS, or Vault). | 2-3 days |
| P1 | SEC-13 | Implement mTLS for Oxigraph-to-TiKV communication. Wire `Config::with_security()` in `oxigraph-tikv` crate. Add cert-manager Certificate resources to Helm chart. | 3-5 days |
| P1 | SEC-06 | Integrate `tracing` crate with `tracing-subscriber` (JSON formatter). Add spans for SPARQL queries, mutations, SHACL operations, and TiKV interactions. Export to stdout for Kubernetes log collection. | 2-3 days |

### Phase 3: Medium (Week 4-6)

| Priority | Finding | Action | Effort |
|----------|---------|--------|--------|
| P2 | SEC-04 | Add `networkpolicy.yaml` to Helm chart templates, based on the policy in `docs/testing-hardening-plan.md` section 4.5. Make it toggleable via `values.yaml`. | 1 day |
| P2 | SEC-05 | Add rate limiting middleware (e.g., token bucket per source IP). Add `--query-timeout` CLI flag and wire to `SparqlEvaluator`. | 2-3 days |
| P2 | SEC-07 | Remove `--cors` from default Containerfile CMD. Add `--cors-origins` flag accepting a comma-separated list of allowed origins. | 1 day |
| P2 | SEC-08 | Add configurable per-query timeout separate from HTTP timeout. | 1 day |
| P2 | SEC-14 | Apply body size limit to `/store` POST endpoint, or make it configurable separately from SPARQL body limit. | 0.5 days |

### Phase 4: Low / Hardening (Week 6-8)

| Priority | Finding | Action | Effort |
|----------|---------|--------|--------|
| P3 | SEC-10 | Create workspace-level `deny.toml` covering all crates. Add `cargo deny check` to CI pipeline. | 0.5 days |
| P3 | SEC-11 | Change default `image.tag` in `values.yaml` from `latest` to the current release version. | 0.5 hours |
| P3 | SEC-12 | Add `seccompProfile: { type: RuntimeDefault }` to `podSecurityContext` in `values.yaml`. | 0.5 hours |
| P3 | SEC-09 | Monitor upstream for `tikv-client`, `rudof_rdf`, and `geo-types` updates that resolve unmaintained transitive deps. | Ongoing |
| P3 | SEC-15 | Replace placeholder secret values in documentation with references to ExternalSecret or SealedSecret patterns. | 0.5 days |
| P3 | -- | Add RBAC templates (ServiceAccount, Role, RoleBinding) to Helm chart with least-privilege permissions. | 1 day |
| P3 | -- | Add PodDisruptionBudget template to Helm chart. | 0.5 hours |
| P3 | -- | Fix readiness probe to use `/ready` endpoint instead of `/` for actual store health checking. | 0.5 hours |
| P3 | -- | Generate SBOM (Software Bill of Materials) during CI build for supply chain transparency. | 1 day |

---

## Appendix A: Files Reviewed

| File | Purpose |
|------|---------|
| `Cargo.toml` (workspace root) | Dependency versions, lints, profiles |
| `Cargo.lock` (workspace root) | Locked dependency versions |
| `oxigraph/deny.toml` | cargo-deny license/advisory configuration |
| `Containerfile` | Container image build definition |
| `crates/oxigraph-server/src/main.rs` | SPARQL HTTP server implementation |
| `crates/oxigraph-server/Cargo.toml` | Server crate dependencies and features |
| `crates/oxigraph-tikv/Cargo.toml` | TiKV backend crate dependencies |
| `crates/oxigraph-shacl/Cargo.toml` | SHACL validation crate dependencies |
| `helm/oxigraph-cloud/values.yaml` | Default Helm values |
| `helm/oxigraph-cloud/templates/statefulset.yaml` | Kubernetes StatefulSet definition |
| `helm/oxigraph-cloud/templates/route.yaml` | OpenShift Route definition |
| `docs/tikv-operations-guide.md` | TiKV operational configuration |
| `docs/testing-hardening-plan.md` | Security audit plan and network policies |

## Appendix B: Tools Used

- `cargo audit` (RustSec advisory database, 951 advisories loaded)
- Manual code review of `main.rs` (914 lines)
- Helm template analysis
- Containerfile static analysis
