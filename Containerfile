# =============================================================================
# Oxigraph Cloud-Native: Multi-stage Containerfile
# =============================================================================
#
# Uses Fedora as builder (compatible glibc with UBI 9 runtime).
#
# Usage:
#   podman build -t oxigraph-cloud .
#   podman run -p 7878:7878 oxigraph-cloud
# =============================================================================

# ---------------------------------------------------------------------------
# Stage 1: Builder (UBI 9 for glibc compatibility with runtime)
# ---------------------------------------------------------------------------
FROM registry.access.redhat.com/ubi9/ubi:latest AS builder

# Install Rust toolchain and build deps
RUN dnf install -y --allowerasing gcc gcc-c++ make cmake clang llvm-devel curl git && \
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain 1.87.0 && \
    dnf clean all

ENV PATH="/root/.cargo/bin:${PATH}"

WORKDIR /usr/src/app

# Copy full source
COPY Cargo.toml Cargo.lock ./
COPY oxigraph/ oxigraph/
COPY crates/ crates/

# Build the server binary
RUN cargo build --release -p oxigraph-server && \
    cp target/release/oxigraph-cloud /usr/local/bin/oxigraph-cloud && \
    strip /usr/local/bin/oxigraph-cloud

# ---------------------------------------------------------------------------
# Stage 2: Runtime
# ---------------------------------------------------------------------------
FROM registry.access.redhat.com/ubi9/ubi-minimal:latest

LABEL name="oxigraph-cloud" \
      summary="Cloud-native Oxigraph SPARQL server with SHACL validation" \
      description="Distributed RDF triplestore with pluggable storage (RocksDB/TiKV) and SHACL validation." \
      io.k8s.display-name="Oxigraph Cloud" \
      io.openshift.tags="rdf,sparql,triplestore,shacl"

# Install libstdc++ (needed by RocksDB) and create non-root user
RUN microdnf install -y shadow-utils libstdc++ && \
    useradd -r -u 1001 -g 0 -d /opt/oxigraph -s /sbin/nologin oxigraph && \
    mkdir -p /opt/oxigraph/data && \
    chown -R 1001:0 /opt/oxigraph && \
    chmod -R g=u /opt/oxigraph && \
    microdnf clean all

COPY --from=builder /usr/local/bin/oxigraph-cloud /usr/local/bin/oxigraph-cloud

EXPOSE 7878
USER 1001
WORKDIR /opt/oxigraph

ENTRYPOINT ["/usr/local/bin/oxigraph-cloud"]
CMD ["--location", "/opt/oxigraph/data", "--bind", "0.0.0.0:7878"]
