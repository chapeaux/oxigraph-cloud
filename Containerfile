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
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable && \
    dnf clean all

ENV PATH="/root/.cargo/bin:${PATH}"

WORKDIR /usr/src/app

# Copy full source
COPY Cargo.toml Cargo.lock ./
COPY oxigraph/ oxigraph/
COPY crates/ crates/
COPY tests/ tests/

# Build the server binary with SHACL validation support
RUN cargo build --release -p oxigraph-server --no-default-features --features rocksdb,shacl && \
    cp target/release/oxigraph-cloud /usr/local/bin/oxigraph-cloud && \
    strip /usr/local/bin/oxigraph-cloud

# ---------------------------------------------------------------------------
# Stage 2: Runtime — minimal attack surface
# ---------------------------------------------------------------------------
FROM registry.access.redhat.com/ubi9/ubi-micro:latest

LABEL name="oxigraph-cloud" \
      summary="Cloud-native Oxigraph SPARQL server with SHACL validation" \
      description="Distributed RDF triplestore with pluggable storage (RocksDB/TiKV) and SHACL validation." \
      io.k8s.display-name="Oxigraph Cloud" \
      io.openshift.tags="rdf,sparql,triplestore,shacl"

# ubi-micro has no package manager, no shell, minimal attack surface.
# It already provides libc, libm, libgcc_s, libpthread, libdl, librt.
# We only need to add libstdc++ (for RocksDB C++ code) from the builder.
COPY --from=builder /usr/lib64/libstdc++.so.6* /usr/lib64/
COPY --from=builder /usr/local/bin/oxigraph-cloud /usr/local/bin/oxigraph-cloud

# Create data directory (no shadow-utils needed — use numeric UID directly)
RUN mkdir -p /opt/oxigraph/data && \
    chmod -R 775 /opt/oxigraph

EXPOSE 7878
USER 1001
WORKDIR /opt/oxigraph

ENTRYPOINT ["/usr/local/bin/oxigraph-cloud"]
CMD ["--location", "/opt/oxigraph/data", "--bind", "0.0.0.0:7878"]
