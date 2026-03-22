use oxhttp::model::{Body, Response, StatusCode};
use prometheus::{
    Encoder, Histogram, HistogramOpts, IntCounterVec, IntGauge, Opts, Registry, TextEncoder,
};
use std::sync::OnceLock;

use crate::HttpError;

/// Global metrics instance, initialized once via `METRICS.set()`.
pub static METRICS: OnceLock<Metrics> = OnceLock::new();

/// Get a reference to the global metrics, if initialized.
pub fn metrics() -> Option<&'static Metrics> {
    METRICS.get()
}

/// Application metrics matching the Grafana dashboard definitions.
///
/// Uses the `prometheus` crate directly for `/metrics` endpoint.
/// OpenTelemetry is used separately for distributed tracing (OTLP export).
#[expect(
    dead_code,
    reason = "shacl_validations_total will be wired into SHACL validation handlers"
)]
pub struct Metrics {
    pub http_requests_total: IntCounterVec,
    pub sparql_queries_total: IntCounterVec,
    pub query_duration_seconds: Histogram,
    pub active_transactions: IntGauge,
    pub shacl_validations_total: IntCounterVec,
    pub store_triple_count: IntGauge,
    pub registry: Registry,
}

impl Metrics {
    pub fn new() -> anyhow::Result<Self> {
        let registry = Registry::new();

        let http_requests_total = IntCounterVec::new(
            Opts::new("oxigraph_http_requests_total", "Total HTTP requests"),
            &["method", "path", "status"],
        )?;
        registry.register(Box::new(http_requests_total.clone()))?;

        let sparql_queries_total = IntCounterVec::new(
            Opts::new(
                "oxigraph_sparql_queries_total",
                "Total SPARQL queries executed",
            ),
            &["status"],
        )?;
        registry.register(Box::new(sparql_queries_total.clone()))?;

        let query_duration_seconds = Histogram::with_opts(
            HistogramOpts::new(
                "oxigraph_query_duration_seconds",
                "SPARQL query duration in seconds",
            )
            .buckets(vec![
                0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0,
            ]),
        )?;
        registry.register(Box::new(query_duration_seconds.clone()))?;

        let active_transactions = IntGauge::new(
            "oxigraph_active_transactions",
            "Number of active HTTP transactions",
        )?;
        registry.register(Box::new(active_transactions.clone()))?;

        let shacl_validations_total = IntCounterVec::new(
            Opts::new(
                "oxigraph_shacl_validations_total",
                "Total SHACL validations performed",
            ),
            &["result"],
        )?;
        registry.register(Box::new(shacl_validations_total.clone()))?;

        let store_triple_count = IntGauge::new(
            "oxigraph_store_triple_count",
            "Number of quads in the store",
        )?;
        registry.register(Box::new(store_triple_count.clone()))?;

        Ok(Self {
            http_requests_total,
            sparql_queries_total,
            query_duration_seconds,
            active_transactions,
            shacl_validations_total,
            store_triple_count,
            registry,
        })
    }

    /// Update the store triple count gauge.
    pub fn update_store_size(&self, store: &oxigraph::store::Store) {
        if let Ok(count) = store.len() {
            self.store_triple_count
                .set(i64::try_from(count).unwrap_or(i64::MAX));
        }
    }
}

/// Initialize OpenTelemetry distributed tracing with OTLP export.
///
/// The OTLP endpoint is read from the `OTEL_EXPORTER_OTLP_ENDPOINT` env var.
///
/// Sets up a layered tracing subscriber: JSON fmt + OTel trace layer.
/// Returns a guard that shuts down the tracer provider on drop.
pub fn init_tracing_with_otel(service_name: &str) -> anyhow::Result<OtelGuard> {
    use opentelemetry::global;
    use opentelemetry_otlp::SpanExporter;
    use opentelemetry_sdk::trace::SdkTracerProvider;
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;

    let span_exporter = SpanExporter::builder().with_tonic().build()?;

    let provider = SdkTracerProvider::builder()
        .with_batch_exporter(span_exporter)
        .with_resource(
            opentelemetry_sdk::Resource::builder()
                .with_service_name(service_name.to_owned())
                .build(),
        )
        .build();

    global::set_tracer_provider(provider.clone());
    let tracer = global::tracer("oxigraph-cloud");

    let otel_layer = tracing_opentelemetry::layer().with_tracer(tracer);
    let fmt_layer = tracing_subscriber::fmt::layer().json();
    let env_filter =
        tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into());

    tracing_subscriber::registry()
        .with(env_filter)
        .with(fmt_layer)
        .with(otel_layer)
        .init();

    tracing::info!("OpenTelemetry tracing initialized");
    Ok(OtelGuard {
        tracer_provider: provider,
    })
}

/// Initialize tracing without OTel (JSON fmt only, with env filter).
pub fn init_tracing_basic() {
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;

    let fmt_layer = tracing_subscriber::fmt::layer().json();
    let env_filter =
        tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into());

    tracing_subscriber::registry()
        .with(env_filter)
        .with(fmt_layer)
        .init();
}

/// Guard that shuts down OTel tracer provider on drop.
pub struct OtelGuard {
    tracer_provider: opentelemetry_sdk::trace::SdkTracerProvider,
}

impl Drop for OtelGuard {
    fn drop(&mut self) {
        drop(self.tracer_provider.shutdown());
    }
}

/// Handle GET /metrics — render Prometheus text format.
pub fn handle_metrics(metrics: &Metrics) -> Result<Response<Body>, HttpError> {
    let encoder = TextEncoder::new();
    let metric_families = metrics.registry.gather();
    let mut buf = Vec::new();
    encoder
        .encode(&metric_families, &mut buf)
        .map_err(crate::internal_server_error)?;
    Response::builder()
        .status(StatusCode::OK)
        .header(
            oxhttp::model::header::CONTENT_TYPE,
            "text/plain; version=0.0.4; charset=utf-8",
        )
        .body(buf.into())
        .map_err(crate::internal_server_error)
}
