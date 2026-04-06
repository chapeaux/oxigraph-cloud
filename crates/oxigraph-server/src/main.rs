mod changelog;
#[cfg(feature = "otel")]
mod telemetry;
mod transactions;

use changelog::{Changelog, ChangelogError, entry_to_detail_json, entry_to_list_json};
use clap::Parser;
use oxhttp::Server;
use oxhttp::model::header::{
    ACCEPT, ACCESS_CONTROL_ALLOW_HEADERS, ACCESS_CONTROL_ALLOW_METHODS,
    ACCESS_CONTROL_ALLOW_ORIGIN, ACCESS_CONTROL_REQUEST_HEADERS, ACCESS_CONTROL_REQUEST_METHOD,
    AUTHORIZATION, CONTENT_TYPE, ORIGIN,
};
use oxhttp::model::{Body, HeaderValue, Method, Request, Response, StatusCode};
use oxigraph::io::{RdfFormat, RdfParser, RdfSerializer};
use oxigraph::model::Quad;
use std::collections::HashSet;
use oxigraph::sparql::results::{QueryResultsFormat, QueryResultsSerializer};
use oxigraph::sparql::{QueryResults, SparqlEvaluator};
use oxigraph::store::Store;
use std::cell::RefCell;
use std::cmp::min;
use std::io::{self, Read, Write};
use std::net::ToSocketAddrs;
use std::rc::Rc;
use std::sync::Arc;
#[cfg(feature = "shacl")]
use std::sync::Mutex;
use std::thread::available_parallelism;
use std::time::{Duration, Instant};
use std::{fmt, str};
use transactions::{BufferedOp, TransactionError, TransactionRegistry};
use url::form_urlencoded;

#[cfg(feature = "shacl")]
use oxigraph_shacl::shapes::CompiledShapes;
#[cfg(feature = "shacl")]
use oxigraph_shacl::validator::{ShaclMode as ShacllibMode, ShaclValidator};

const MAX_SPARQL_BODY_SIZE: u64 = 128 * 1024 * 1024; // 128 MB
const HTTP_TIMEOUT: Duration = Duration::from_secs(60);
const DEFAULT_QUERY_TIMEOUT_SECS: u64 = 30;
const DEFAULT_MAX_UPLOAD_SIZE: u64 = 128 * 1024 * 1024; // 128 MB

/// SHACL validation mode for the server.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ShaclMode {
    /// No SHACL validation
    Off,
    /// Validate on writes, reject invalid data
    Enforce,
    /// Validate on writes, log warnings but accept data
    Warn,
}

impl ShaclMode {
    fn from_str(s: &str) -> anyhow::Result<Self> {
        match s {
            "off" => Ok(Self::Off),
            "enforce" | "strict" => Ok(Self::Enforce),
            "warn" => Ok(Self::Warn),
            other => anyhow::bail!("Unknown SHACL mode: {other}. Supported: off, enforce, warn"),
        }
    }
}

/// Cloud-native Oxigraph SPARQL server with TiKV and SHACL support.
#[derive(Parser, Debug)]
#[command(name = "oxigraph-cloud", version, about)]
struct Args {
    /// Storage backend: "rocksdb" (default) or "tikv"
    #[arg(long, default_value = "rocksdb")]
    backend: String,

    /// TiKV PD endpoints (comma-separated), e.g. "127.0.0.1:2379"
    #[arg(long, default_value = "127.0.0.1:2379")]
    pd_endpoints: String,

    /// Bind address for the SPARQL HTTP server
    #[arg(long, default_value = "127.0.0.1:7878")]
    bind: String,

    /// Path for RocksDB storage (when using rocksdb backend)
    #[arg(long)]
    location: Option<String>,

    /// SHACL validation mode: "off" (default), "enforce", "warn"
    #[arg(long, default_value = "off")]
    shacl_mode: String,

    /// CORS allowed origins: empty (default, no CORS), "*" (wildcard), or
    /// comma-separated origins like "https://a.example,https://b.example"
    #[arg(long, default_value = "")]
    cors_origins: String,

    /// Query execution timeout in seconds (default 30)
    #[arg(long, default_value_t = DEFAULT_QUERY_TIMEOUT_SECS)]
    query_timeout: u64,

    /// Maximum upload body size in bytes for /store POST (default 128 MB)
    #[arg(long, default_value_t = DEFAULT_MAX_UPLOAD_SIZE)]
    max_upload_size: u64,

    /// API key required for write operations (update, store POST, SHACL mutations).
    /// Can also be set via OXIGRAPH_WRITE_KEY environment variable.
    /// Required when binding to a non-localhost address.
    #[arg(long, env = "OXIGRAPH_WRITE_KEY")]
    write_key: Option<String>,

    /// Enable changelog recording for write operations.
    /// When enabled, all writes are recorded and can be undone via /changelog/{id}/undo.
    #[arg(long, default_value_t = false)]
    changelog: bool,

    /// Maximum number of changelog entries to retain (0 = unlimited).
    #[arg(long, default_value_t = 100)]
    changelog_retain: usize,

    /// Transaction idle timeout in seconds.
    /// Transactions inactive longer than this are automatically rolled back.
    #[arg(long, default_value_t = 60)]
    transaction_timeout: u64,

    /// Port for the CDC notification server. Enables CDC when set.
    /// The CDC server provides W3C Solid Notifications via WebSocket and SSE.
    #[cfg(feature = "cdc")]
    #[arg(long)]
    cdc_port: Option<u16>,

    /// CDC subscriber buffer size (max queued notifications per subscriber, default 1024).
    #[cfg(feature = "cdc")]
    #[arg(long, default_value_t = 1024)]
    cdc_buffer_size: usize,

    /// CDC batching window in milliseconds (default 100).
    /// Events within this window are merged into a single notification.
    #[cfg(feature = "cdc")]
    #[arg(long, default_value_t = 100)]
    cdc_batch_ms: u64,

    /// Enable OpenTelemetry metrics (/metrics endpoint) and optional OTLP tracing.
    #[arg(long, default_value_t = false)]
    otel: bool,

    /// OTLP endpoint for distributed trace export (e.g., http://localhost:4317).
    /// Only used when --otel is enabled. Also configurable via OTEL_EXPORTER_OTLP_ENDPOINT.
    #[arg(long, env = "OTEL_EXPORTER_OTLP_ENDPOINT")]
    otel_endpoint: Option<String>,

    /// Service name for OpenTelemetry resource.
    #[arg(long, default_value = "oxigraph-cloud", env = "OTEL_SERVICE_NAME")]
    otel_service_name: String,
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    // Initialize tracing (with or without OTel)
    #[cfg(feature = "otel")]
    let _otel_guard = if args.otel && args.otel_endpoint.is_some() {
        Some(telemetry::init_tracing_with_otel(&args.otel_service_name)?)
    } else {
        telemetry::init_tracing_basic();
        None
    };

    #[cfg(not(feature = "otel"))]
    tracing_subscriber::fmt().json().init();
    let shacl_mode = ShaclMode::from_str(&args.shacl_mode)?;
    let query_timeout = Duration::from_secs(args.query_timeout);
    let max_upload_size = args.max_upload_size;
    let cors_origins = args.cors_origins.clone();
    let write_key = args.write_key.clone();

    // SEC-01: Require write key when binding to non-localhost
    let is_localhost = args.bind.starts_with("127.0.0.1:")
        || args.bind.starts_with("localhost:")
        || args.bind.starts_with("[::1]:");
    if !is_localhost && write_key.is_none() {
        anyhow::bail!(
            "Binding to {} requires --write-key (or OXIGRAPH_WRITE_KEY env var) to be set.\n\
             Use --bind 127.0.0.1:7878 for local-only access without authentication.",
            args.bind
        );
    }

    let store = match args.backend.as_str() {
        "rocksdb" => {
            if let Some(location) = &args.location {
                tracing::info!(backend = "rocksdb", path = %location, "Opening RocksDB store");
                Store::open(location)?
            } else {
                tracing::info!(
                    backend = "rocksdb",
                    "Opening in-memory store (no --location given)"
                );
                Store::new()?
            }
        }
        "tikv" => {
            #[cfg(feature = "tikv")]
            {
                let pd_endpoints: Vec<String> = args
                    .pd_endpoints
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .collect();
                tracing::info!(backend = "tikv", pd_endpoints = ?pd_endpoints, "Connecting to TiKV cluster");
                Store::open_tikv(&pd_endpoints)?
            }
            #[cfg(not(feature = "tikv"))]
            {
                anyhow::bail!(
                    "TiKV support requires the 'tikv' feature. \
                     Rebuild with: cargo build -p oxigraph-server --features tikv"
                );
            }
        }
        other => {
            anyhow::bail!("Unknown backend: {other}. Supported: rocksdb, tikv");
        }
    };

    if shacl_mode != ShaclMode::Off {
        tracing::info!(shacl_mode = ?shacl_mode, "SHACL validation-on-ingest enabled");
    }

    // Metrics (feature-gated, stored globally via OnceLock)
    #[cfg(feature = "otel")]
    if args.otel {
        let m = telemetry::Metrics::new()?;
        if telemetry::METRICS.set(m).is_err() {
            anyhow::bail!("Metrics already initialized");
        }
        tracing::info!("Prometheus metrics enabled at /metrics");
    }

    // Transaction registry and changelog
    let txn_timeout = Duration::from_secs(args.transaction_timeout);
    let registry = Arc::new(TransactionRegistry::new(txn_timeout));
    let changelog = Arc::new(Changelog::new(args.changelog, args.changelog_retain));
    changelog.init_counter(&store);

    if args.changelog {
        tracing::info!(retain = args.changelog_retain, "Changelog enabled");
    }

    // CDC notification server
    #[cfg(feature = "cdc")]
    let _cdc_description_url = if let Some(cdc_port) = args.cdc_port {
        let cdc_config = oxigraph_cdc::CdcConfig {
            bind_port: cdc_port,
            main_server_url: format!("http://{}", args.bind),
            buffer_size: args.cdc_buffer_size,
            batch_window: Duration::from_millis(args.cdc_batch_ms),
        };
        let (sender, cdc_server) = oxigraph_cdc::CdcServer::new(cdc_config);
        changelog.set_cdc_sender(sender);

        let description_url = format!("http://0.0.0.0:{cdc_port}/.well-known/solid");

        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .unwrap_or_else(|e| {
                    tracing::error!(error = %e, "Failed to create CDC tokio runtime");
                    std::process::exit(1);
                });
            if let Err(e) = rt.block_on(cdc_server.run()) {
                tracing::error!(error = %e, "CDC server exited with error");
            }
        });

        tracing::info!(cdc_port, "CDC notification server started");
        Some(description_url)
    } else {
        None
    };

    // Spawn background cleanup for expired transactions
    {
        let reg = Arc::clone(&registry);
        std::thread::spawn(move || -> ! {
            loop {
                std::thread::sleep(Duration::from_secs(30));
                let cleaned = reg.cleanup_expired();
                if cleaned > 0 {
                    tracing::info!(count = cleaned, "Cleaned up expired transactions");
                }
            }
        });
    }

    #[cfg(feature = "shacl")]
    let validator = {
        let lib_mode = match shacl_mode {
            ShaclMode::Off => ShacllibMode::Off,
            ShaclMode::Enforce => ShacllibMode::Enforce,
            ShaclMode::Warn => ShacllibMode::Warn,
        };
        Arc::new(Mutex::new(ShaclValidator::new(lib_mode)))
    };

    #[cfg(feature = "shacl")]
    return serve(
        store,
        &args.bind,
        &cors_origins,
        query_timeout,
        max_upload_size,
        write_key,
        &validator,
        &registry,
        &changelog,
    );

    #[cfg(not(feature = "shacl"))]
    serve(
        store,
        &args.bind,
        &cors_origins,
        query_timeout,
        max_upload_size,
        write_key,
        &registry,
        &changelog,
    )
}

#[cfg(feature = "shacl")]
#[expect(clippy::too_many_arguments)]
fn serve(
    store: Store,
    bind: &str,
    cors_origins: &str,
    query_timeout: Duration,
    max_upload_size: u64,
    write_key: Option<String>,
    validator: &Arc<Mutex<ShaclValidator>>,
    registry: &Arc<TransactionRegistry>,
    changelog: &Arc<Changelog>,
) -> anyhow::Result<()> {
    let write_key = Arc::new(write_key);
    let mut server = if cors_origins.is_empty() {
        let v = Arc::clone(validator);
        let wk = Arc::clone(&write_key);
        let reg = Arc::clone(registry);
        let cl = Arc::clone(changelog);
        Server::new(move |request| {
            handle_with_metrics(request, |r| {
                handle_request(
                    r,
                    store.clone(),
                    &v,
                    query_timeout,
                    max_upload_size,
                    &wk,
                    &reg,
                    &cl,
                )
            })
        })
    } else {
        let v = Arc::clone(validator);
        let wk = Arc::clone(&write_key);
        let reg = Arc::clone(registry);
        let cl = Arc::clone(changelog);
        let origins = cors_origins.to_owned();
        Server::new(cors_middleware(
            move |request| {
                handle_with_metrics(request, |r| {
                    handle_request(
                        r,
                        store.clone(),
                        &v,
                        query_timeout,
                        max_upload_size,
                        &wk,
                        &reg,
                        &cl,
                    )
                })
            },
            origins,
        ))
    }
    .with_global_timeout(HTTP_TIMEOUT)
    .with_server_name(concat!("OxigraphCloud/", env!("CARGO_PKG_VERSION")))?
    .with_max_concurrent_connections(available_parallelism()?.get() * 128);

    for socket in bind.to_socket_addrs()? {
        server = server.bind(socket);
    }
    let server = server.spawn()?;
    tracing::info!(bind = %bind, cors_origins = %cors_origins, query_timeout_secs = query_timeout.as_secs(), max_upload_size, "Server listening");
    server.join()?;
    Ok(())
}

#[cfg(not(feature = "shacl"))]
fn serve(
    store: Store,
    bind: &str,
    cors_origins: &str,
    query_timeout: Duration,
    max_upload_size: u64,
    write_key: Option<String>,
    registry: &Arc<TransactionRegistry>,
    changelog: &Arc<Changelog>,
) -> anyhow::Result<()> {
    let write_key = Arc::new(write_key);
    let mut server = if cors_origins.is_empty() {
        let wk = Arc::clone(&write_key);
        let reg = Arc::clone(registry);
        let cl = Arc::clone(changelog);
        Server::new(move |request| {
            handle_with_metrics(request, |r| {
                handle_request(
                    r,
                    store.clone(),
                    query_timeout,
                    max_upload_size,
                    &wk,
                    &reg,
                    &cl,
                )
            })
        })
    } else {
        let wk = Arc::clone(&write_key);
        let reg = Arc::clone(registry);
        let cl = Arc::clone(changelog);
        let origins = cors_origins.to_owned();
        Server::new(cors_middleware(
            move |request| {
                handle_with_metrics(request, |r| {
                    handle_request(
                        r,
                        store.clone(),
                        query_timeout,
                        max_upload_size,
                        &wk,
                        &reg,
                        &cl,
                    )
                })
            },
            origins,
        ))
    }
    .with_global_timeout(HTTP_TIMEOUT)
    .with_server_name(concat!("OxigraphCloud/", env!("CARGO_PKG_VERSION")))?
    .with_max_concurrent_connections(available_parallelism()?.get() * 128);

    for socket in bind.to_socket_addrs()? {
        server = server.bind(socket);
    }
    let server = server.spawn()?;
    tracing::info!(bind = %bind, cors_origins = %cors_origins, query_timeout_secs = query_timeout.as_secs(), max_upload_size, "Server listening");
    server.join()?;
    Ok(())
}

// ---------------------------------------------------------------------------
// CORS middleware (mirrors upstream pattern)
// ---------------------------------------------------------------------------

/// SEC-07: Configurable CORS origins.
/// `allowed_origins` is either `"*"` (wildcard) or a comma-separated list of
/// allowed origins like `"https://a.example,https://b.example"`.
fn cors_middleware(
    on_request: impl Fn(&mut Request<Body>) -> Response<Body> + Send + Sync + 'static,
    allowed_origins: String,
) -> impl Fn(&mut Request<Body>) -> Response<Body> + Send + Sync + 'static {
    move |request| {
        let origin_header = request
            .headers()
            .get(ORIGIN)
            .and_then(|v| v.to_str().ok())
            .map(ToOwned::to_owned);

        let allowed_value = origin_header.as_deref().and_then(|origin| {
            if allowed_origins == "*" {
                Some(HeaderValue::from_static("*"))
            } else {
                // Check if request origin is in the comma-separated allow-list
                let is_allowed = allowed_origins.split(',').any(|o| o.trim() == origin);
                if is_allowed {
                    HeaderValue::try_from(origin).ok()
                } else {
                    None
                }
            }
        });

        if *request.method() == Method::OPTIONS {
            let mut response = Response::builder().status(StatusCode::NO_CONTENT);
            if let Some(allow_origin) = &allowed_value {
                response =
                    response.header(ACCESS_CONTROL_ALLOW_ORIGIN.clone(), allow_origin.clone());
            }
            let headers = request.headers();
            if let Some(method) = headers.get(ACCESS_CONTROL_REQUEST_METHOD) {
                response = response.header(ACCESS_CONTROL_ALLOW_METHODS, method.clone());
            }
            if let Some(h) = headers.get(ACCESS_CONTROL_REQUEST_HEADERS) {
                response = response.header(ACCESS_CONTROL_ALLOW_HEADERS, h.clone());
            }
            response.body(Body::empty()).unwrap()
        } else {
            let mut response = on_request(request);
            if let Some(allow_origin) = allowed_value {
                response
                    .headers_mut()
                    .append(ACCESS_CONTROL_ALLOW_ORIGIN, allow_origin);
            }
            response
        }
    }
}

// ---------------------------------------------------------------------------
// Route dispatch
// ---------------------------------------------------------------------------

type HttpError = (StatusCode, String);

#[cfg(feature = "shacl")]
#[expect(clippy::too_many_arguments)]
fn handle_request(
    request: &mut Request<Body>,
    store: Store,
    validator: &Arc<Mutex<ShaclValidator>>,
    query_timeout: Duration,
    max_upload_size: u64,
    write_key: &Option<String>,
    registry: &TransactionRegistry,
    changelog: &Changelog,
) -> Result<Response<Body>, HttpError> {
    let method = request.method().as_ref().to_owned();
    let path = request.uri().path().to_owned();
    tracing::info!(method = %method, path = %path, "Incoming request");

    match (path.as_str(), method.as_str()) {
        // --- SHACL endpoints (write operations require auth) ---
        ("/shacl/shapes", "POST") => {
            check_write_auth(request, write_key)?;
            tracing::info!("SHACL shapes upload");
            handle_shacl_upload_shapes(request, validator)
        }
        ("/shacl/shapes", "GET") => handle_shacl_get_shapes(validator),
        ("/shacl/shapes", "DELETE") => {
            check_write_auth(request, write_key)?;
            tracing::info!("SHACL shapes delete");
            handle_shacl_delete_shapes(validator)
        }
        ("/shacl/validate", "POST") => {
            tracing::info!("SHACL validate");
            handle_shacl_validate(&store, validator)
        }
        ("/shacl/mode", "GET") => handle_shacl_get_mode(validator),
        ("/shacl/mode", "PUT") => {
            check_write_auth(request, write_key)?;
            tracing::info!("SHACL mode update");
            handle_shacl_set_mode(request, validator)
        }

        // --- Everything else (with validation-on-ingest for write paths) ---
        ("/update", "POST") => {
            check_write_auth(request, write_key)?;
            tracing::info!("SPARQL UPDATE (with SHACL validation)");
            let ct = content_type(request).ok_or_else(|| bad_request("No Content-Type given"))?;
            let update = if ct == "application/sparql-update" {
                limited_string_body(request)?
            } else if ct == "application/x-www-form-urlencoded" {
                let body = limited_body(request)?;
                form_urlencoded::parse(&body)
                    .find(|(k, _)| k == "update")
                    .map(|(_, v)| v.into_owned())
                    .ok_or_else(|| bad_request("Missing 'update' parameter in form body"))?
            } else {
                return Err(unsupported_media_type(&ct));
            };
            let (resp, ops) = evaluate_sparql_update_with_diff(&store, &update, changelog)?;
            validate_after_write(&store, validator)?;
            if let Some(ops) = ops {
                drop(changelog.record(&store, &ops, "update"));
            }
            Ok(resp)
        }
        ("/store", "POST") => {
            check_write_auth(request, write_key)?;
            tracing::info!("Store POST with SHACL validation");
            let ct = content_type(request).ok_or_else(|| bad_request("No Content-Type given"))?;
            let format =
                RdfFormat::from_media_type(&ct).ok_or_else(|| unsupported_media_type(&ct))?;
            let parser = RdfParser::from_format(format);
            let body = limited_body_with_max(request, max_upload_size)?;
            let quads: Vec<Quad> = RdfParser::from_format(format)
                .for_slice(&body)
                .flatten()
                .collect();
            store
                .load_from_reader(parser, body.as_slice())
                .map_err(internal_server_error)?;
            validate_after_write(&store, validator)?;
            if changelog.is_enabled() && !quads.is_empty() {
                let ops = vec![BufferedOp::InsertQuads(quads)];
                drop(changelog.record(&store, &ops, "store"));
            }
            Response::builder()
                .status(StatusCode::NO_CONTENT)
                .body(Body::empty())
                .map_err(internal_server_error)
        }
        _ => handle_request_core(
            request,
            store,
            query_timeout,
            max_upload_size,
            write_key,
            registry,
            changelog,
        ),
    }
}

#[cfg(not(feature = "shacl"))]
fn handle_request(
    request: &mut Request<Body>,
    store: Store,
    query_timeout: Duration,
    max_upload_size: u64,
    write_key: &Option<String>,
    registry: &TransactionRegistry,
    changelog: &Changelog,
) -> Result<Response<Body>, HttpError> {
    let method = request.method().as_ref().to_owned();
    let path = request.uri().path().to_owned();
    tracing::info!(method = %method, path = %path, "Incoming request");
    handle_request_core(
        request,
        store,
        query_timeout,
        max_upload_size,
        write_key,
        registry,
        changelog,
    )
}

fn handle_request_core(
    request: &mut Request<Body>,
    store: Store,
    query_timeout: Duration,
    max_upload_size: u64,
    write_key: &Option<String>,
    registry: &TransactionRegistry,
    changelog: &Changelog,
) -> Result<Response<Body>, HttpError> {
    let path = request.uri().path().to_owned();
    let method = request.method().as_ref().to_owned();

    // Check for transaction endpoints: /transactions or /transactions/{id}/...
    if let Some(rest) = path.strip_prefix("/transactions") {
        return handle_transaction_routes(
            request, &store, write_key, registry, changelog, rest, &method,
        );
    }

    // Check for changelog endpoints: /changelog or /changelog/{id}/...
    if let Some(rest) = path.strip_prefix("/changelog") {
        return handle_changelog_routes(request, &store, write_key, changelog, rest, &method);
    }

    match (path.as_str(), method.as_str()) {
        // --- Health & readiness (always open) ---
        ("/health", "GET") => Response::builder()
            .status(StatusCode::OK)
            .header(CONTENT_TYPE, "text/plain")
            .body("OK".into())
            .map_err(internal_server_error),

        ("/ready", "GET") => {
            let _: bool = store.is_empty().map_err(internal_server_error)?;
            Response::builder()
                .status(StatusCode::OK)
                .header(CONTENT_TYPE, "text/plain")
                .body("READY".into())
                .map_err(internal_server_error)
        }

        // --- Prometheus metrics endpoint ---
        #[cfg(feature = "otel")]
        ("/metrics", "GET") => {
            if let Some(m) = telemetry::metrics() {
                m.update_store_size(&store);
                telemetry::handle_metrics(m)
            } else {
                Err((
                    StatusCode::NOT_FOUND,
                    "Metrics not enabled (use --otel)".to_owned(),
                ))
            }
        }

        // --- Root page (always open) ---
        ("/", "GET") => Response::builder()
            .header(CONTENT_TYPE, "text/html; charset=utf-8")
            .body(
                "<html><body>\
                 <h1>Oxigraph Cloud SPARQL Server</h1>\
                 <p>Endpoints: POST /query, POST /update, POST /store</p>\
                 <p>Transaction API: POST /transactions, /transactions/{id}/...</p>\
                 <p>Changelog: GET /changelog, POST /changelog/{id}/undo</p>\
                 </body></html>"
                    .into(),
            )
            .map_err(internal_server_error),

        // --- SPARQL Query (reads are open) ---
        ("/query", "GET") => {
            tracing::info!("SPARQL query via GET");
            let query = url_query_parameter(request, "query")
                .ok_or_else(|| bad_request("Missing 'query' parameter"))?;
            evaluate_sparql_query(&store, &query, request, query_timeout)
        }

        ("/query", "POST") => {
            tracing::info!("SPARQL query via POST");
            let ct = content_type(request).ok_or_else(|| bad_request("No Content-Type given"))?;
            if ct == "application/sparql-query" {
                let query = limited_string_body(request)?;
                evaluate_sparql_query(&store, &query, request, query_timeout)
            } else if ct == "application/x-www-form-urlencoded" {
                let body = limited_body(request)?;
                let query = form_urlencoded::parse(&body)
                    .find(|(k, _)| k == "query")
                    .map(|(_, v)| v.into_owned())
                    .ok_or_else(|| bad_request("Missing 'query' parameter in form body"))?;
                evaluate_sparql_query(&store, &query, request, query_timeout)
            } else {
                Err(unsupported_media_type(&ct))
            }
        }

        // --- SPARQL Update (write - requires auth) ---
        ("/update", "POST") => {
            check_write_auth(request, write_key)?;
            tracing::info!("SPARQL UPDATE");
            let ct = content_type(request).ok_or_else(|| bad_request("No Content-Type given"))?;
            let update = if ct == "application/sparql-update" {
                limited_string_body(request)?
            } else if ct == "application/x-www-form-urlencoded" {
                let body = limited_body(request)?;
                form_urlencoded::parse(&body)
                    .find(|(k, _)| k == "update")
                    .map(|(_, v)| v.into_owned())
                    .ok_or_else(|| bad_request("Missing 'update' parameter in form body"))?
            } else {
                return Err(unsupported_media_type(&ct));
            };
            let (resp, ops) = evaluate_sparql_update_with_diff(&store, &update, changelog)?;
            if let Some(ops) = ops {
                drop(changelog.record(&store, &ops, "update"));
            }
            Ok(resp)
        }

        // --- Graph Store Protocol: load data (write - requires auth) ---
        ("/store", "POST") => {
            check_write_auth(request, write_key)?;
            tracing::info!("Store POST (graph upload)");
            let ct = content_type(request).ok_or_else(|| bad_request("No Content-Type given"))?;
            let format =
                RdfFormat::from_media_type(&ct).ok_or_else(|| unsupported_media_type(&ct))?;
            let parser = RdfParser::from_format(format);
            // SEC-14: Apply upload body size limit
            let body = limited_body_with_max(request, max_upload_size)?;
            let quads: Vec<Quad> = RdfParser::from_format(format)
                .for_slice(&body)
                .flatten()
                .collect();
            store
                .load_from_reader(parser, body.as_slice())
                .map_err(internal_server_error)?;
            if changelog.is_enabled() && !quads.is_empty() {
                let ops = vec![BufferedOp::InsertQuads(quads)];
                drop(changelog.record(&store, &ops, "store"));
            }
            Response::builder()
                .status(StatusCode::NO_CONTENT)
                .body(Body::empty())
                .map_err(internal_server_error)
        }

        // --- Graph Store Protocol: dump all data ---
        ("/store", "GET") => {
            let format = rdf_content_negotiation(request)?;
            if !format.supports_datasets() {
                return Err(bad_request(format!(
                    "Cannot serialize full dataset with {format} (no named graph support)"
                )));
            }
            ReadForWrite::build_response(
                move |w| {
                    Ok((
                        RdfSerializer::from_format(format).for_writer(w),
                        store.iter(),
                    ))
                },
                |(mut serializer, mut quads)| {
                    Ok(if let Some(q) = quads.next() {
                        serializer.serialize_quad(&q.map_err(io::Error::other)?)?;
                        Some((serializer, quads))
                    } else {
                        serializer.finish()?;
                        None
                    })
                },
                format.media_type(),
            )
        }

        // --- Catch-all ---
        _ => Err((
            StatusCode::NOT_FOUND,
            format!("Not found: {} {}", request.method(), request.uri().path()),
        )),
    }
}

// ---------------------------------------------------------------------------
// Transaction endpoint handlers
// ---------------------------------------------------------------------------

fn handle_transaction_routes(
    request: &mut Request<Body>,
    store: &Store,
    write_key: &Option<String>,
    registry: &TransactionRegistry,
    changelog: &Changelog,
    rest: &str,
    method: &str,
) -> Result<Response<Body>, HttpError> {
    // POST /transactions — begin
    if rest.is_empty() && method == "POST" {
        check_write_auth(request, write_key)?;
        let txn_id = registry.begin();
        return Response::builder()
            .status(StatusCode::CREATED)
            .header(CONTENT_TYPE, "application/json")
            .header("Location", format!("/transactions/{txn_id}"))
            .body(format!("{{\"transaction_id\":\"{txn_id}\"}}").into())
            .map_err(internal_server_error);
    }

    // Parse /{id} or /{id}/action
    let rest = rest.strip_prefix('/').unwrap_or(rest);
    let (txn_id, action) = match rest.split_once('/') {
        Some((id, action)) => (id, action),
        None => (rest, ""),
    };

    if txn_id.is_empty() {
        return Err(bad_request("Missing transaction ID"));
    }

    match (action, method) {
        // PUT /transactions/{id}/add
        ("add", "PUT") => {
            check_write_auth(request, write_key)?;
            let ct = content_type(request).ok_or_else(|| bad_request("No Content-Type given"))?;
            let format =
                RdfFormat::from_media_type(&ct).ok_or_else(|| unsupported_media_type(&ct))?;
            let body = limited_body(request)?;
            let count = registry
                .add(txn_id, format, &body)
                .map_err(txn_error_to_http)?;
            Response::builder()
                .status(StatusCode::OK)
                .header(CONTENT_TYPE, "application/json")
                .body(format!("{{\"added\":{count}}}").into())
                .map_err(internal_server_error)
        }

        // PUT /transactions/{id}/remove
        ("remove", "PUT") => {
            check_write_auth(request, write_key)?;
            let ct = content_type(request).ok_or_else(|| bad_request("No Content-Type given"))?;
            let format =
                RdfFormat::from_media_type(&ct).ok_or_else(|| unsupported_media_type(&ct))?;
            let body = limited_body(request)?;
            let count = registry
                .remove(txn_id, format, &body)
                .map_err(txn_error_to_http)?;
            Response::builder()
                .status(StatusCode::OK)
                .header(CONTENT_TYPE, "application/json")
                .body(format!("{{\"removed\":{count}}}").into())
                .map_err(internal_server_error)
        }

        // POST /transactions/{id}/query
        ("query", "POST") => {
            let sparql = limited_string_body(request)?;
            let result = registry
                .query(txn_id, store, &sparql)
                .map_err(txn_error_to_http)?;
            Response::builder()
                .status(StatusCode::OK)
                .header(CONTENT_TYPE, "application/sparql-results+json")
                .body(result.into())
                .map_err(internal_server_error)
        }

        // POST /transactions/{id}/update
        ("update", "POST") => {
            check_write_auth(request, write_key)?;
            let sparql = limited_string_body(request)?;
            registry.update(txn_id, sparql).map_err(txn_error_to_http)?;
            Response::builder()
                .status(StatusCode::NO_CONTENT)
                .body(Body::empty())
                .map_err(internal_server_error)
        }

        // PUT /transactions/{id}/commit
        ("commit", "PUT") => {
            check_write_auth(request, write_key)?;
            let result = registry.commit(txn_id, store).map_err(txn_error_to_http)?;

            // Record in changelog if enabled
            if changelog.is_enabled() {
                drop(changelog.record(store, &result.ops, "transaction"));
            }

            Response::builder()
                .status(StatusCode::OK)
                .header(CONTENT_TYPE, "application/json")
                .header("X-Transaction-Id", &result.txn_id)
                .body(
                    format!(
                        "{{\"committed\":true,\"transaction_id\":\"{}\"}}",
                        result.txn_id
                    )
                    .into(),
                )
                .map_err(internal_server_error)
        }

        // DELETE /transactions/{id} — rollback
        ("", "DELETE") => {
            check_write_auth(request, write_key)?;
            registry.rollback(txn_id).map_err(txn_error_to_http)?;
            Response::builder()
                .status(StatusCode::NO_CONTENT)
                .body(Body::empty())
                .map_err(internal_server_error)
        }

        _ => Err((
            StatusCode::NOT_FOUND,
            format!("Not found: {method} /transactions/{rest}"),
        )),
    }
}

// ---------------------------------------------------------------------------
// Changelog endpoint handlers
// ---------------------------------------------------------------------------

fn handle_changelog_routes(
    request: &mut Request<Body>,
    store: &Store,
    write_key: &Option<String>,
    changelog: &Changelog,
    rest: &str,
    method: &str,
) -> Result<Response<Body>, HttpError> {
    // GET /changelog — list entries
    if rest.is_empty() && method == "GET" {
        let offset: usize = url_query_parameter(request, "offset")
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        let limit: usize = url_query_parameter(request, "limit")
            .and_then(|v| v.parse().ok())
            .unwrap_or(20);

        let entries = changelog
            .list(store, offset, limit)
            .map_err(changelog_error_to_http)?;
        let json_entries: Vec<String> = entries.iter().map(entry_to_list_json).collect();
        let body = format!("{{\"entries\":[{}]}}", json_entries.join(","));
        return Response::builder()
            .status(StatusCode::OK)
            .header(CONTENT_TYPE, "application/json")
            .body(body.into())
            .map_err(internal_server_error);
    }

    // DELETE /changelog — purge all
    if rest.is_empty() && method == "DELETE" {
        check_write_auth(request, write_key)?;
        let count = changelog.purge(store).map_err(changelog_error_to_http)?;
        return Response::builder()
            .status(StatusCode::OK)
            .header(CONTENT_TYPE, "application/json")
            .body(format!("{{\"purged\":{count}}}").into())
            .map_err(internal_server_error);
    }

    // Parse /{id} or /{id}/undo
    let rest = rest.strip_prefix('/').unwrap_or(rest);
    let (id_str, action) = match rest.split_once('/') {
        Some((id, action)) => (id, action),
        None => (rest, ""),
    };

    let id: u64 = id_str
        .parse()
        .map_err(|_| bad_request(format!("Invalid changelog ID: {id_str}")))?;

    match (action, method) {
        // GET /changelog/{id} — get detail
        ("", "GET") => {
            let entry = changelog
                .get(store, id)
                .map_err(changelog_error_to_http)?
                .ok_or_else(|| {
                    (
                        StatusCode::NOT_FOUND,
                        "Changelog entry not found".to_owned(),
                    )
                })?;
            let body = entry_to_detail_json(&entry);
            Response::builder()
                .status(StatusCode::OK)
                .header(CONTENT_TYPE, "application/json")
                .body(body.into())
                .map_err(internal_server_error)
        }

        // POST /changelog/{id}/undo
        ("undo", "POST") => {
            check_write_auth(request, write_key)?;
            let undo_entry = changelog.undo(store, id).map_err(changelog_error_to_http)?;
            let body = entry_to_detail_json(&undo_entry);
            Response::builder()
                .status(StatusCode::OK)
                .header(CONTENT_TYPE, "application/json")
                .header("X-Transaction-Id", undo_entry.id.to_string())
                .body(body.into())
                .map_err(internal_server_error)
        }

        _ => Err((
            StatusCode::NOT_FOUND,
            format!("Not found: {method} /changelog/{rest}"),
        )),
    }
}

/// Wrapper that handles a request and records HTTP metrics (when otel is enabled).
fn handle_with_metrics(
    request: &mut Request<Body>,
    handler: impl FnOnce(&mut Request<Body>) -> Result<Response<Body>, HttpError>,
) -> Response<Body> {
    #[cfg(feature = "otel")]
    let method = request.method().as_ref().to_owned();
    #[cfg(feature = "otel")]
    let path = request.uri().path().to_owned();

    let response = handler(request).unwrap_or_else(|(status, message)| error(status, message));

    #[cfg(feature = "otel")]
    record_http_metrics(&method, &path, response.status().as_u16());

    response
}

/// Record HTTP request metrics after handling.
#[cfg(feature = "otel")]
fn record_http_metrics(method: &str, path: &str, status: u16) {
    if let Some(m) = telemetry::metrics() {
        // Normalize path to avoid high cardinality (strip IDs)
        let norm_path = if path.starts_with("/transactions/") {
            "/transactions/{id}"
        } else if path.starts_with("/changelog/") {
            "/changelog/{id}"
        } else {
            path
        };
        m.http_requests_total
            .with_label_values(&[method, norm_path, &status.to_string()])
            .inc();
    }
}

fn txn_error_to_http(e: TransactionError) -> HttpError {
    match e {
        TransactionError::NotFound => (StatusCode::NOT_FOUND, e.to_string()),
        TransactionError::Parse(msg) | TransactionError::Query(msg) => {
            (StatusCode::BAD_REQUEST, msg)
        }
        TransactionError::Storage(msg) => (StatusCode::INTERNAL_SERVER_ERROR, msg),
    }
}

fn changelog_error_to_http(e: ChangelogError) -> HttpError {
    match e {
        ChangelogError::Disabled | ChangelogError::NotFound => {
            (StatusCode::NOT_FOUND, e.to_string())
        }
        ChangelogError::NotUndoable => (StatusCode::UNPROCESSABLE_ENTITY, e.to_string()),
        ChangelogError::Internal(msg) => (StatusCode::INTERNAL_SERVER_ERROR, msg),
    }
}

// ---------------------------------------------------------------------------
// SHACL endpoint handlers (feature-gated)
// ---------------------------------------------------------------------------

/// Validate store data after a write operation (SHACL validation-on-ingest).
/// Returns HTTP 422 if validation fails in enforce mode.
#[cfg(feature = "shacl")]
fn validate_after_write(
    store: &Store,
    validator: &Arc<Mutex<ShaclValidator>>,
) -> Result<(), HttpError> {
    let v = validator.lock().map_err(internal_server_error)?;
    if v.shapes().is_none() || v.mode() == ShacllibMode::Off {
        return Ok(());
    }
    match v.validate(store) {
        Ok(
            oxigraph_shacl::validator::ValidationOutcome::Passed
            | oxigraph_shacl::validator::ValidationOutcome::Skipped,
        ) => Ok(()),
        Ok(oxigraph_shacl::validator::ValidationOutcome::Failed(report)) => {
            let report_json = oxigraph_shacl::report::report_to_json(&report);
            if v.mode() == ShacllibMode::Warn {
                tracing::warn!(report = %report_json, "SHACL validation warning");
                Ok(())
            } else {
                tracing::warn!(report = %report_json, "SHACL validation failed - rejecting write");
                Err((StatusCode::UNPROCESSABLE_ENTITY, report_json))
            }
        }
        Err(e) => {
            tracing::error!(error = %e, "SHACL validation error");
            Err(internal_server_error(e))
        }
    }
}

#[cfg(feature = "shacl")]
fn handle_shacl_upload_shapes(
    request: &mut Request<Body>,
    validator: &Arc<Mutex<ShaclValidator>>,
) -> Result<Response<Body>, HttpError> {
    let turtle = limited_string_body(request)?;
    let shapes = CompiledShapes::from_turtle(&turtle)
        .map_err(|e| bad_request(format!("Failed to compile shapes: {e}")))?;
    let count = shapes.shape_count();
    let mut v = validator.lock().map_err(internal_server_error)?;
    v.set_shapes(shapes);
    let body = format!("{{\"loaded\": true, \"shape_count\": {count}}}");
    Response::builder()
        .status(StatusCode::CREATED)
        .header(CONTENT_TYPE, "application/json")
        .body(body.into())
        .map_err(internal_server_error)
}

#[cfg(feature = "shacl")]
fn handle_shacl_get_shapes(
    validator: &Arc<Mutex<ShaclValidator>>,
) -> Result<Response<Body>, HttpError> {
    let v = validator.lock().map_err(internal_server_error)?;
    let (loaded, count) = match v.shapes() {
        Some(s) => (true, s.shape_count()),
        None => (false, 0),
    };
    let body = format!("{{\"loaded\": {loaded}, \"shape_count\": {count}}}");
    Response::builder()
        .status(StatusCode::OK)
        .header(CONTENT_TYPE, "application/json")
        .body(body.into())
        .map_err(internal_server_error)
}

#[cfg(feature = "shacl")]
fn handle_shacl_delete_shapes(
    validator: &Arc<Mutex<ShaclValidator>>,
) -> Result<Response<Body>, HttpError> {
    let mut v = validator.lock().map_err(internal_server_error)?;
    // Replace the validator with a fresh one keeping the same mode
    let mode = v.mode();
    *v = ShaclValidator::new(mode);
    Response::builder()
        .status(StatusCode::NO_CONTENT)
        .body(Body::empty())
        .map_err(internal_server_error)
}

#[cfg(feature = "shacl")]
fn handle_shacl_validate(
    store: &Store,
    validator: &Arc<Mutex<ShaclValidator>>,
) -> Result<Response<Body>, HttpError> {
    let v = validator.lock().map_err(internal_server_error)?;
    if v.shapes().is_none() {
        return Err(bad_request(
            "No SHACL shapes loaded. Upload shapes first via POST /shacl/shapes",
        ));
    }
    let outcome = v.validate(store).map_err(internal_server_error)?;
    let body = match &outcome {
        oxigraph_shacl::validator::ValidationOutcome::Skipped => {
            "{\"conforms\": null, \"skipped\": true}".to_owned()
        }
        oxigraph_shacl::validator::ValidationOutcome::Passed => {
            "{\"conforms\": true, \"results_count\": 0}".to_owned()
        }
        oxigraph_shacl::validator::ValidationOutcome::Failed(report) => {
            oxigraph_shacl::report::report_to_json(report)
        }
    };
    Response::builder()
        .status(StatusCode::OK)
        .header(CONTENT_TYPE, "application/json")
        .body(body.into())
        .map_err(internal_server_error)
}

#[cfg(feature = "shacl")]
fn shacl_mode_str(mode: ShacllibMode) -> &'static str {
    match mode {
        ShacllibMode::Off => "off",
        ShacllibMode::Warn => "warn",
        ShacllibMode::Enforce => "enforce",
    }
}

#[cfg(feature = "shacl")]
fn handle_shacl_get_mode(
    validator: &Arc<Mutex<ShaclValidator>>,
) -> Result<Response<Body>, HttpError> {
    let v = validator.lock().map_err(internal_server_error)?;
    let mode = shacl_mode_str(v.mode());
    let body = format!("{{\"mode\": \"{mode}\"}}");
    Response::builder()
        .status(StatusCode::OK)
        .header(CONTENT_TYPE, "application/json")
        .body(body.into())
        .map_err(internal_server_error)
}

#[cfg(feature = "shacl")]
fn handle_shacl_set_mode(
    request: &mut Request<Body>,
    validator: &Arc<Mutex<ShaclValidator>>,
) -> Result<Response<Body>, HttpError> {
    let body_str = limited_string_body(request)?;
    // Simple JSON parsing: find "mode" value without pulling in serde_json
    let mode_value = body_str.split('"').collect::<Vec<_>>();
    // Expected format: {"mode": "enforce"}
    // After splitting by '"': ["", "{", "mode", ": ", "enforce", "}"]
    let mode_str = mode_value
        .iter()
        .enumerate()
        .find(|(_, s)| **s == "mode")
        .and_then(|(i, _)| mode_value.get(i + 2))
        .ok_or_else(|| {
            bad_request("Expected JSON body with \"mode\" field, e.g. {\"mode\": \"enforce\"}")
        })?;

    let new_mode = match *mode_str {
        "off" => ShacllibMode::Off,
        "warn" => ShacllibMode::Warn,
        "enforce" => ShacllibMode::Enforce,
        other => {
            return Err(bad_request(format!(
                "Invalid mode: \"{other}\". Must be one of: off, warn, enforce"
            )));
        }
    };

    let mut v = validator.lock().map_err(internal_server_error)?;
    v.set_mode(new_mode);
    let mode = shacl_mode_str(v.mode());
    let body = format!("{{\"mode\": \"{mode}\"}}");
    Response::builder()
        .status(StatusCode::OK)
        .header(CONTENT_TYPE, "application/json")
        .body(body.into())
        .map_err(internal_server_error)
}

// ---------------------------------------------------------------------------
// SPARQL evaluation
// ---------------------------------------------------------------------------

fn evaluate_sparql_query(
    store: &Store,
    query: &str,
    request: &Request<Body>,
    query_timeout: Duration,
) -> Result<Response<Body>, HttpError> {
    let start = Instant::now();
    let evaluator = SparqlEvaluator::new();
    let prepared = evaluator.parse_query(query).map_err(|e| {
        #[cfg(feature = "otel")]
        if let Some(m) = telemetry::metrics() {
            m.sparql_queries_total.with_label_values(&["error"]).inc();
        }
        bad_request(e)
    })?;
    let results = prepared.on_store(store).execute().map_err(|e| {
        #[cfg(feature = "otel")]
        if let Some(m) = telemetry::metrics() {
            m.sparql_queries_total.with_label_values(&["error"]).inc();
            m.query_duration_seconds
                .observe(start.elapsed().as_secs_f64());
        }
        internal_server_error(e)
    })?;

    #[cfg(feature = "otel")]
    if let Some(m) = telemetry::metrics() {
        m.sparql_queries_total.with_label_values(&["ok"]).inc();
        m.query_duration_seconds
            .observe(start.elapsed().as_secs_f64());
    }

    // SEC-05/SEC-08: Check if query parsing already exceeded timeout
    if start.elapsed() > query_timeout {
        return Err((
            StatusCode::REQUEST_TIMEOUT,
            format!(
                "Query execution exceeded timeout of {}s",
                query_timeout.as_secs()
            ),
        ));
    }

    match results {
        QueryResults::Solutions(solutions) => {
            let format = query_results_content_negotiation(request)?;
            let deadline = start + query_timeout;
            ReadForWrite::build_response(
                move |w| {
                    Ok((
                        QueryResultsSerializer::from_format(format)
                            .serialize_solutions_to_writer(w, solutions.variables().to_vec())?,
                        solutions,
                        deadline,
                    ))
                },
                |(mut serializer, mut solutions, deadline)| {
                    if Instant::now() > deadline {
                        return Err(io::Error::other("Query execution exceeded timeout"));
                    }
                    Ok(if let Some(solution) = solutions.next() {
                        serializer.serialize(&solution.map_err(io::Error::other)?)?;
                        Some((serializer, solutions, deadline))
                    } else {
                        serializer.finish()?;
                        None
                    })
                },
                format.media_type(),
            )
        }
        QueryResults::Boolean(result) => {
            let format = query_results_content_negotiation(request)?;
            let mut body = Vec::new();
            QueryResultsSerializer::from_format(format)
                .serialize_boolean_to_writer(&mut body, result)
                .map_err(internal_server_error)?;
            Response::builder()
                .header(CONTENT_TYPE, format.media_type())
                .body(body.into())
                .map_err(internal_server_error)
        }
        QueryResults::Graph(triples) => {
            let format = rdf_content_negotiation(request)?;
            let deadline = start + query_timeout;
            ReadForWrite::build_response(
                move |w| {
                    Ok((
                        RdfSerializer::from_format(format).for_writer(w),
                        triples,
                        deadline,
                    ))
                },
                |(mut serializer, mut triples, deadline)| {
                    if Instant::now() > deadline {
                        return Err(io::Error::other("Query execution exceeded timeout"));
                    }
                    Ok(if let Some(t) = triples.next() {
                        serializer.serialize_triple(&t.map_err(io::Error::other)?)?;
                        Some((serializer, triples, deadline))
                    } else {
                        serializer.finish()?;
                        None
                    })
                },
                format.media_type(),
            )
        }
    }
}

fn evaluate_sparql_update(store: &Store, update: &str) -> Result<Response<Body>, HttpError> {
    let evaluator = SparqlEvaluator::new();
    let prepared = evaluator.parse_update(update).map_err(bad_request)?;
    prepared
        .on_store(store)
        .execute()
        .map_err(internal_server_error)?;
    Response::builder()
        .status(StatusCode::NO_CONTENT)
        .body(Body::empty())
        .map_err(internal_server_error)
}

/// Max store size (in quads) for which we'll compute a before/after diff
/// to capture the actual inserted/removed quads from a SPARQL UPDATE.
/// Above this threshold we record only metadata (no quad deltas).
const MAX_DIFF_STORE_SIZE: usize = 1_000_000;

/// Execute a SPARQL UPDATE with before/after diff to capture actual quad deltas.
/// Returns the HTTP response and the changelog ops (if changelog is enabled).
fn evaluate_sparql_update_with_diff(
    store: &Store,
    update: &str,
    changelog: &Changelog,
) -> Result<(Response<Body>, Option<Vec<BufferedOp>>), HttpError> {
    if !changelog.is_enabled() {
        let resp = evaluate_sparql_update(store, update)?;
        return Ok((resp, None));
    }

    // Take a snapshot of current quads (if store is small enough)
    let store_size = store.len().unwrap_or(0);
    if store_size > MAX_DIFF_STORE_SIZE {
        // Store too large for diff — record SPARQL text only (empty deltas)
        let resp = evaluate_sparql_update(store, update)?;
        let ops = vec![BufferedOp::SparqlUpdate(update.to_owned())];
        return Ok((resp, Some(ops)));
    }

    let before: HashSet<Quad> = store.iter().flatten().collect();

    let resp = evaluate_sparql_update(store, update)?;

    let after: HashSet<Quad> = store.iter().flatten().collect();
    let inserted: Vec<Quad> = after.difference(&before).cloned().collect();
    let removed: Vec<Quad> = before.difference(&after).cloned().collect();

    let ops = if inserted.is_empty() && removed.is_empty() {
        None
    } else {
        let mut ops = Vec::new();
        if !inserted.is_empty() {
            ops.push(BufferedOp::InsertQuads(inserted));
        }
        if !removed.is_empty() {
            ops.push(BufferedOp::RemoveQuads(removed));
        }
        Some(ops)
    };

    Ok((resp, ops))
}

// ---------------------------------------------------------------------------
// Content negotiation
// ---------------------------------------------------------------------------

fn query_results_content_negotiation(
    request: &Request<Body>,
) -> Result<QueryResultsFormat, HttpError> {
    content_negotiation(
        request,
        QueryResultsFormat::from_media_type,
        QueryResultsFormat::Json,
        &[
            ("application", QueryResultsFormat::Json),
            ("text", QueryResultsFormat::Json),
        ],
        "application/sparql-results+json or text/tsv",
    )
}

fn rdf_content_negotiation(request: &Request<Body>) -> Result<RdfFormat, HttpError> {
    content_negotiation(
        request,
        RdfFormat::from_media_type,
        RdfFormat::NQuads,
        &[
            ("application", RdfFormat::NQuads),
            ("text", RdfFormat::NQuads),
        ],
        "application/n-quads or text/turtle",
    )
}

fn content_negotiation<F: Copy>(
    request: &Request<Body>,
    parse: impl Fn(&str) -> Option<F>,
    default: F,
    default_by_base: &[(&str, F)],
    example: &str,
) -> Result<F, HttpError> {
    let header = request
        .headers()
        .get(ACCEPT)
        .map(|h| h.to_str())
        .transpose()
        .map_err(|_| bad_request("The Accept header should be a valid ASCII string"))?
        .unwrap_or_default();

    if header.is_empty() {
        return Ok(default);
    }

    let mut result = None;
    let mut result_score = 0_f32;
    for mut possible in header.split(',') {
        let mut score = 1.;
        if let Some((possible_type, last_param)) = possible.rsplit_once(';') {
            if let Some((name, value)) = last_param.split_once('=') {
                if name.trim().eq_ignore_ascii_case("q") {
                    score = value
                        .trim()
                        .parse::<f32>()
                        .map_err(|_| bad_request(format!("Invalid Accept score: {value}")))?;
                    possible = possible_type;
                }
            }
        }
        if score <= result_score {
            continue;
        }
        let (possible_base, possible_sub) = possible
            .split_once(';')
            .unwrap_or((possible, ""))
            .0
            .split_once('/')
            .ok_or_else(|| bad_request(format!("Invalid media type: '{possible}'")))?;
        let possible_base = possible_base.trim();
        let possible_sub = possible_sub.trim();

        let mut format = None;
        if possible_base == "*" && possible_sub == "*" {
            format = Some(default);
        } else if possible_sub == "*" {
            for (base, sub_format) in default_by_base {
                if *base == possible_base {
                    format = Some(*sub_format);
                }
            }
        } else {
            format = parse(possible);
        }
        if let Some(f) = format {
            result = Some(f);
            result_score = score;
        }
    }

    result.ok_or_else(|| {
        (
            StatusCode::NOT_ACCEPTABLE,
            format!("The Accept header does not provide any accepted format like {example}"),
        )
    })
}

// ---------------------------------------------------------------------------
// Request helpers
// ---------------------------------------------------------------------------

fn url_query_parameter(request: &Request<Body>, param: &str) -> Option<String> {
    let query_bytes = request.uri().query().unwrap_or_default().as_bytes();
    form_urlencoded::parse(query_bytes)
        .find(|(k, _)| k == param)
        .map(|(_, v)| v.into_owned())
}

fn content_type(request: &Request<Body>) -> Option<String> {
    let value = request.headers().get(CONTENT_TYPE)?.to_str().ok()?;
    Some(
        value
            .split_once(';')
            .map_or(value, |(b, _)| b)
            .trim()
            .to_ascii_lowercase(),
    )
}

fn limited_string_body(request: &mut Request<Body>) -> Result<String, HttpError> {
    String::from_utf8(limited_body(request)?)
        .map_err(|e| bad_request(format!("Invalid UTF-8 body: {e}")))
}

fn limited_body(request: &mut Request<Body>) -> Result<Vec<u8>, HttpError> {
    limited_body_with_max(request, MAX_SPARQL_BODY_SIZE)
}

/// SEC-14: Body size limit helper supporting a configurable maximum.
fn limited_body_with_max(request: &mut Request<Body>, max_size: u64) -> Result<Vec<u8>, HttpError> {
    let body = request.body_mut();
    if let Some(body_len) = body.len() {
        if body_len > max_size {
            return Err(bad_request(format!(
                "Body too large: {body_len} bytes (limit {max_size})"
            )));
        }
        let mut payload = Vec::with_capacity(
            body_len
                .try_into()
                .map_err(|_| bad_request("Huge body size"))?,
        );
        body.read_to_end(&mut payload)
            .map_err(internal_server_error)?;
        Ok(payload)
    } else {
        let mut payload = Vec::new();
        body.take(max_size + 1)
            .read_to_end(&mut payload)
            .map_err(internal_server_error)?;
        if payload.len() > max_size.try_into().map_err(internal_server_error)? {
            return Err(bad_request(format!(
                "Body too large (limit {max_size} bytes)"
            )));
        }
        Ok(payload)
    }
}

// ---------------------------------------------------------------------------
// Error helpers
// ---------------------------------------------------------------------------

fn error(status: StatusCode, message: impl fmt::Display) -> Response<Body> {
    Response::builder()
        .status(status)
        .header(CONTENT_TYPE, "text/plain; charset=utf-8")
        .body(message.to_string().into())
        .unwrap()
}

fn bad_request(message: impl fmt::Display) -> HttpError {
    (StatusCode::BAD_REQUEST, message.to_string())
}

/// SEC-01: Check write authorization.
/// If a write key is configured, the request must include a matching
/// `Authorization: Bearer <key>` header. If no key is configured
/// (localhost-only mode), all writes are allowed.
fn check_write_auth(request: &Request<Body>, write_key: &Option<String>) -> Result<(), HttpError> {
    let Some(expected_key) = write_key else {
        return Ok(()); // No key configured (localhost mode) - allow
    };

    let auth_header = request
        .headers()
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    let provided_key = auth_header.strip_prefix("Bearer ").unwrap_or(auth_header);

    if provided_key == expected_key {
        Ok(())
    } else {
        tracing::warn!(path = %request.uri().path(), "Unauthorized write attempt");
        Err((
            StatusCode::UNAUTHORIZED,
            "Write operations require a valid Authorization: Bearer <key> header".to_owned(),
        ))
    }
}

fn unsupported_media_type(content_type: &str) -> HttpError {
    (
        StatusCode::UNSUPPORTED_MEDIA_TYPE,
        format!("Unsupported Content-Type: {content_type}"),
    )
}

fn internal_server_error(message: impl fmt::Display) -> HttpError {
    tracing::error!(error = %message, "Internal server error");
    (StatusCode::INTERNAL_SERVER_ERROR, message.to_string())
}

// ---------------------------------------------------------------------------
// Streaming response helper (from upstream)
// ---------------------------------------------------------------------------

struct ReadForWrite<O, U: (Fn(O) -> io::Result<Option<O>>)> {
    buffer: Rc<RefCell<Vec<u8>>>,
    position: usize,
    add_more_data: U,
    state: Option<O>,
}

impl<O: 'static, U: (Fn(O) -> io::Result<Option<O>>) + 'static> ReadForWrite<O, U> {
    fn build_response(
        initial_state_builder: impl FnOnce(ReadForWriteWriter) -> io::Result<O>,
        add_more_data: U,
        content_type: &'static str,
    ) -> Result<Response<Body>, HttpError> {
        let buffer = Rc::new(RefCell::new(Vec::new()));
        let state = initial_state_builder(ReadForWriteWriter {
            buffer: Rc::clone(&buffer),
        })
        .map_err(internal_server_error)?;
        Response::builder()
            .header(CONTENT_TYPE, content_type)
            .body(Body::from_read(Self {
                buffer,
                position: 0,
                add_more_data,
                state: Some(state),
            }))
            .map_err(internal_server_error)
    }
}

impl<O, U: (Fn(O) -> io::Result<Option<O>>)> Read for ReadForWrite<O, U> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        while self.position == self.buffer.borrow().len() {
            if let Some(state) = self.state.take() {
                self.buffer.borrow_mut().clear();
                self.position = 0;
                self.state = match (self.add_more_data)(state) {
                    Ok(state) => state,
                    Err(e) => {
                        tracing::error!(error = %e, "Internal server error while streaming");
                        self.buffer
                            .borrow_mut()
                            .write_all(e.to_string().as_bytes())?;
                        None
                    }
                }
            } else {
                return Ok(0);
            }
        }
        let buffer = self.buffer.borrow();
        let len = min(buffer.len() - self.position, buf.len());
        buf[..len].copy_from_slice(&buffer[self.position..self.position + len]);
        self.position += len;
        Ok(len)
    }
}

struct ReadForWriteWriter {
    buffer: Rc<RefCell<Vec<u8>>>,
}

impl Write for ReadForWriteWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.buffer.borrow_mut().write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }

    fn write_all(&mut self, buf: &[u8]) -> io::Result<()> {
        self.buffer.borrow_mut().write_all(buf)
    }
}
