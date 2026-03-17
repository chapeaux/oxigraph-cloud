# SHACL Shape Management REST API Specification

> **Status**: Draft v1 | **Date**: 2026-03-17
> **Task**: Phase 3, Task 3.2 | **Related**: [03-rudof-shacl-integration.md](03-rudof-shacl-integration.md)

---

## Overview

This document specifies the REST API for managing SHACL shapes and triggering validation in the Oxigraph cloud-native server. Shapes graphs are stored as named graphs within the Oxigraph store using a well-known IRI scheme. The validation engine is powered by the `rudof` crate's `shacl_validation` module via the SRDF trait bridge.

### Base Path

All endpoints are mounted under `/shacl`.

### Named Graph Convention

Each uploaded shapes graph is stored in a named graph with the IRI:

```
urn:oxigraph:shacl:shapes:{id}
```

where `{id}` is a server-generated UUID v4 (e.g., `urn:oxigraph:shacl:shapes:a1b2c3d4-e5f6-7890-abcd-ef1234567890`).

---

## Endpoints

### 1. Upload a Shapes Graph

Upload a SHACL shapes graph to the store.

| Field | Value |
|-------|-------|
| **Method** | `POST` |
| **Path** | `/shacl/shapes` |

#### Request

| Header | Required | Values |
|--------|----------|--------|
| `Content-Type` | Yes | `text/turtle`, `application/ld+json`, `application/n-triples`, `application/rdf+xml` |
| `X-Shape-Name` | No | Human-readable name for this shapes graph (max 256 chars) |

**Body**: RDF document containing SHACL shape definitions.

#### Response

**201 Created**

```json
{
  "id": "a1b2c3d4-e5f6-7890-abcd-ef1234567890",
  "name": "Person shapes",
  "graph": "urn:oxigraph:shacl:shapes:a1b2c3d4-e5f6-7890-abcd-ef1234567890",
  "triple_count": 42,
  "created": "2026-03-17T14:30:00Z"
}
```

| Header | Value |
|--------|-------|
| `Location` | `/shacl/shapes/a1b2c3d4-e5f6-7890-abcd-ef1234567890` |

#### Error Responses

| Status | Condition | Body |
|--------|-----------|------|
| 400 Bad Request | Malformed RDF or unsupported Content-Type | `{"error": "parse_error", "message": "Expected predicate at line 5, column 12"}` |
| 400 Bad Request | Body contains no SHACL shape definitions (`sh:NodeShape` or `sh:PropertyShape`) | `{"error": "no_shapes", "message": "No SHACL shape definitions found in uploaded graph"}` |
| 413 Payload Too Large | Body exceeds server-configured size limit | `{"error": "payload_too_large", "message": "Request body exceeds 10MB limit"}` |

#### Example

```bash
curl -X POST http://localhost:7878/shacl/shapes \
  -H "Content-Type: text/turtle" \
  -H "X-Shape-Name: Person shapes" \
  -d @- <<'EOF'
@prefix sh: <http://www.w3.org/ns/shacl#> .
@prefix ex: <http://example.org/> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .

ex:PersonShape a sh:NodeShape ;
  sh:targetClass ex:Person ;
  sh:property [
    sh:path ex:name ;
    sh:minCount 1 ;
    sh:datatype xsd:string ;
  ] ;
  sh:property [
    sh:path ex:age ;
    sh:datatype xsd:integer ;
    sh:minInclusive 0 ;
  ] .
EOF
```

---

### 2. List All Shapes Graphs

Retrieve metadata for all active shapes graphs.

| Field | Value |
|-------|-------|
| **Method** | `GET` |
| **Path** | `/shacl/shapes` |

#### Request

No request body. No required headers.

Optional query parameters:

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `offset` | integer | 0 | Pagination offset |
| `limit` | integer | 100 | Maximum results to return (max 1000) |

#### Response

**200 OK**

```json
{
  "shapes": [
    {
      "id": "a1b2c3d4-e5f6-7890-abcd-ef1234567890",
      "name": "Person shapes",
      "graph": "urn:oxigraph:shacl:shapes:a1b2c3d4-e5f6-7890-abcd-ef1234567890",
      "triple_count": 42,
      "created": "2026-03-17T14:30:00Z"
    },
    {
      "id": "b2c3d4e5-f6a7-8901-bcde-f23456789012",
      "name": null,
      "graph": "urn:oxigraph:shacl:shapes:b2c3d4e5-f6a7-8901-bcde-f23456789012",
      "triple_count": 18,
      "created": "2026-03-17T15:00:00Z"
    }
  ],
  "total": 2,
  "offset": 0,
  "limit": 100
}
```

#### Error Responses

| Status | Condition | Body |
|--------|-----------|------|
| 400 Bad Request | Invalid pagination parameters | `{"error": "invalid_parameter", "message": "limit must be between 1 and 1000"}` |

#### Example

```bash
curl http://localhost:7878/shacl/shapes
```

```bash
curl "http://localhost:7878/shacl/shapes?offset=0&limit=10"
```

---

### 3. Get a Specific Shapes Graph

Retrieve the RDF content of a specific shapes graph. Supports content negotiation.

| Field | Value |
|-------|-------|
| **Method** | `GET` |
| **Path** | `/shacl/shapes/{id}` |

#### Request

| Header | Required | Values |
|--------|----------|--------|
| `Accept` | No | `text/turtle` (default), `application/ld+json`, `application/n-triples`, `application/rdf+xml`, `application/json` |

When `Accept: application/json` is specified, the response is a metadata object (same schema as the list endpoint) rather than the raw RDF content.

#### Response

**200 OK** (with `Accept: text/turtle` or default)

```turtle
@prefix sh: <http://www.w3.org/ns/shacl#> .
@prefix ex: <http://example.org/> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .

ex:PersonShape a sh:NodeShape ;
  sh:targetClass ex:Person ;
  sh:property [
    sh:path ex:name ;
    sh:minCount 1 ;
    sh:datatype xsd:string ;
  ] .
```

**200 OK** (with `Accept: application/json`)

```json
{
  "id": "a1b2c3d4-e5f6-7890-abcd-ef1234567890",
  "name": "Person shapes",
  "graph": "urn:oxigraph:shacl:shapes:a1b2c3d4-e5f6-7890-abcd-ef1234567890",
  "triple_count": 42,
  "created": "2026-03-17T14:30:00Z"
}
```

#### Error Responses

| Status | Condition | Body |
|--------|-----------|------|
| 404 Not Found | No shapes graph with given ID | `{"error": "not_found", "message": "Shapes graph a1b2c3d4... not found"}` |
| 406 Not Acceptable | Unsupported Accept header | `{"error": "not_acceptable", "message": "Supported formats: text/turtle, application/ld+json, application/n-triples, application/rdf+xml, application/json"}` |

#### Example

```bash
# Get as Turtle (default)
curl http://localhost:7878/shacl/shapes/a1b2c3d4-e5f6-7890-abcd-ef1234567890

# Get as JSON-LD
curl -H "Accept: application/ld+json" \
  http://localhost:7878/shacl/shapes/a1b2c3d4-e5f6-7890-abcd-ef1234567890

# Get metadata only
curl -H "Accept: application/json" \
  http://localhost:7878/shacl/shapes/a1b2c3d4-e5f6-7890-abcd-ef1234567890
```

---

### 4. Delete a Shapes Graph

Remove a shapes graph from the store. The named graph and all its triples are deleted.

| Field | Value |
|-------|-------|
| **Method** | `DELETE` |
| **Path** | `/shacl/shapes/{id}` |

#### Request

No request body. No required headers.

#### Response

**204 No Content**

No response body.

#### Error Responses

| Status | Condition | Body |
|--------|-----------|------|
| 404 Not Found | No shapes graph with given ID | `{"error": "not_found", "message": "Shapes graph a1b2c3d4... not found"}` |
| 409 Conflict | Validation mode is `enforce` and this is the only active shapes graph | `{"error": "conflict", "message": "Cannot delete the only shapes graph while enforcement mode is active. Set mode to 'off' or 'warn' first, or upload a replacement."}` |

#### Example

```bash
curl -X DELETE http://localhost:7878/shacl/shapes/a1b2c3d4-e5f6-7890-abcd-ef1234567890
```

---

### 5. Validate Data Against Shapes

Trigger on-demand SHACL validation of data in the store against one or more shapes graphs. Returns a SHACL Validation Report.

| Field | Value |
|-------|-------|
| **Method** | `POST` |
| **Path** | `/shacl/validate` |

#### Request

No request body required. Validation scope is controlled via query parameters.

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `shapes` | string | (all active shapes) | Comma-separated shape graph IDs to validate against. If omitted, all shapes graphs are used. |
| `graph` | string | (default graph) | Named graph IRI to validate. If omitted, validates the default graph. Use `all` to validate all named graphs (excluding shapes graphs). |
| `focus` | string | (none) | Specific focus node IRI to validate. If omitted, all target nodes from shape declarations are validated. |

| Header | Required | Values |
|--------|----------|--------|
| `Accept` | No | `text/turtle` (default), `application/ld+json`, `application/json` |

#### Response

**200 OK** -- Validation completed (regardless of conformance result).

With `Accept: text/turtle` (default):

```turtle
@prefix sh: <http://www.w3.org/ns/shacl#> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
@prefix ex: <http://example.org/> .

[] a sh:ValidationReport ;
  sh:conforms false ;
  sh:result [
    a sh:ValidationResult ;
    sh:resultSeverity sh:Violation ;
    sh:focusNode ex:person1 ;
    sh:resultPath ex:name ;
    sh:sourceConstraintComponent sh:MinCountConstraintComponent ;
    sh:sourceShape _:b0 ;
    sh:resultMessage "Property ex:name has fewer than 1 values" ;
  ] .
```

With `Accept: application/json` (simplified):

```json
{
  "conforms": false,
  "shapes_used": ["a1b2c3d4-e5f6-7890-abcd-ef1234567890"],
  "graph_validated": "default",
  "results_count": 1,
  "results": [
    {
      "severity": "Violation",
      "focus_node": "http://example.org/person1",
      "path": "http://example.org/name",
      "constraint": "sh:MinCountConstraintComponent",
      "message": "Property ex:name has fewer than 1 values",
      "source_shape": "http://example.org/PersonShape"
    }
  ]
}
```

#### Error Responses

| Status | Condition | Body |
|--------|-----------|------|
| 400 Bad Request | Invalid shapes ID or graph IRI | `{"error": "invalid_parameter", "message": "Shapes graph xyz not found"}` |
| 404 Not Found | Specified graph does not exist | `{"error": "not_found", "message": "Named graph http://example.org/mygraph not found"}` |
| 422 Unprocessable Entity | No shapes graphs available to validate against | `{"error": "no_shapes", "message": "No SHACL shapes graphs are loaded. Upload shapes first via POST /shacl/shapes"}` |

#### Example

```bash
# Validate default graph against all shapes
curl -X POST http://localhost:7878/shacl/validate

# Validate against a specific shapes graph
curl -X POST "http://localhost:7878/shacl/validate?shapes=a1b2c3d4-e5f6-7890-abcd-ef1234567890"

# Validate a specific named graph
curl -X POST "http://localhost:7878/shacl/validate?graph=http://example.org/mygraph"

# Validate a specific focus node, get JSON response
curl -X POST "http://localhost:7878/shacl/validate?focus=http://example.org/person1" \
  -H "Accept: application/json"

# Validate all graphs against all shapes
curl -X POST "http://localhost:7878/shacl/validate?graph=all"
```

---

### 6. Get Current Validation Mode

Retrieve the current SHACL validation mode for the ingestion pipeline.

| Field | Value |
|-------|-------|
| **Method** | `GET` |
| **Path** | `/shacl/mode` |

#### Request

No request body. No required headers.

#### Response

**200 OK**

```json
{
  "mode": "off",
  "description": "No validation (default, backward compatible)",
  "active_shapes_count": 2
}
```

The `mode` field is one of:

| Mode | Behavior |
|------|----------|
| `off` | No SHACL validation on ingest. Default for backward compatibility. |
| `warn` | Validate on ingest; log failures but accept all data. Validation report written to server log. |
| `enforce` | Validate on ingest; reject non-conforming data with HTTP 422. |

#### Example

```bash
curl http://localhost:7878/shacl/mode
```

---

### 7. Set Validation Mode

Change the SHACL validation mode. This affects all subsequent SPARQL UPDATE and bulk load operations.

| Field | Value |
|-------|-------|
| **Method** | `PUT` |
| **Path** | `/shacl/mode` |

#### Request

| Header | Required | Values |
|--------|----------|--------|
| `Content-Type` | Yes | `application/json` |

**Body**:

```json
{
  "mode": "enforce"
}
```

Valid values for `mode`: `off`, `warn`, `enforce`.

#### Response

**200 OK**

```json
{
  "mode": "enforce",
  "description": "Reject data that fails validation",
  "active_shapes_count": 2,
  "previous_mode": "off"
}
```

#### Error Responses

| Status | Condition | Body |
|--------|-----------|------|
| 400 Bad Request | Invalid mode value | `{"error": "invalid_mode", "message": "Mode must be one of: off, warn, enforce"}` |
| 400 Bad Request | Missing or malformed JSON body | `{"error": "parse_error", "message": "Expected JSON object with 'mode' field"}` |
| 409 Conflict | Setting `enforce` or `warn` with no shapes loaded | `{"error": "no_shapes", "message": "Cannot enable validation without any shapes graphs. Upload shapes first via POST /shacl/shapes"}` |

#### Example

```bash
# Enable enforcement
curl -X PUT http://localhost:7878/shacl/mode \
  -H "Content-Type: application/json" \
  -d '{"mode": "enforce"}'

# Switch to warn-only
curl -X PUT http://localhost:7878/shacl/mode \
  -H "Content-Type: application/json" \
  -d '{"mode": "warn"}'

# Disable validation
curl -X PUT http://localhost:7878/shacl/mode \
  -H "Content-Type: application/json" \
  -d '{"mode": "off"}'
```

---

## Validation-on-Ingest Behavior

When the validation mode is `warn` or `enforce`, SHACL validation is triggered automatically during data ingestion operations:

| Operation | Validation Trigger |
|-----------|--------------------|
| `POST /store` (SPARQL UPDATE) | Before transaction commit |
| `POST /store` (bulk load via `Content-Type: text/turtle` etc.) | Before transaction commit |
| `PUT /store` (graph replacement) | Before transaction commit |

### Enforcement Flow

```
Client sends SPARQL UPDATE
    |
    v
Parse & execute update (build candidate graph state)
    |
    v
[mode == off?] --yes--> Commit transaction --> 200 OK
    |
    no
    v
Run shacl_validation::validate() against candidate state
    |
    v
[conforms?] --yes--> Commit transaction --> 200 OK
    |
    no
    v
[mode == warn?] --yes--> Log report --> Commit transaction --> 200 OK
    |
    no (mode == enforce)
    v
Rollback transaction --> 422 Unprocessable Entity + Validation Report
```

### Rejection Response (mode: enforce)

When data is rejected, the response includes the full SHACL validation report:

**422 Unprocessable Entity**

```turtle
@prefix sh: <http://www.w3.org/ns/shacl#> .

[] a sh:ValidationReport ;
  sh:conforms false ;
  sh:result [
    a sh:ValidationResult ;
    sh:resultSeverity sh:Violation ;
    sh:focusNode <http://example.org/person1> ;
    sh:resultPath <http://example.org/name> ;
    sh:sourceConstraintComponent sh:MinCountConstraintComponent ;
    sh:resultMessage "Property ex:name has fewer than 1 values" ;
  ] .
```

The `Accept` header from the original request controls the format of the validation report in the 422 response.

---

## Server CLI Flags

These endpoints interact with the server configuration. Initial mode can be set via CLI:

```bash
oxigraph-cloud \
  --backend tikv \
  --pd-endpoints 127.0.0.1:2379 \
  --bind 0.0.0.0:7878 \
  --shacl-mode enforce
```

The `PUT /shacl/mode` endpoint overrides the CLI setting at runtime. The runtime setting is not persisted across server restarts; the CLI flag is the source of truth on startup.

---

## Implementation Notes

### Storage

- Shapes graph metadata (name, created timestamp) is stored as triples within the shapes named graph using a custom vocabulary:
  - `<urn:oxigraph:shacl:shapes:{id}> <urn:oxigraph:shacl:name> "Person shapes"`
  - `<urn:oxigraph:shacl:shapes:{id}> <urn:oxigraph:shacl:created> "2026-03-17T14:30:00Z"^^xsd:dateTime`
- The shapes named graphs are excluded from `SPARQL SELECT` queries unless explicitly referenced by `FROM NAMED` or `GRAPH` clauses. This avoids polluting user query results with shape definitions.

### Concurrency

- `PUT /shacl/mode` is an atomic operation (single `AtomicU8` or `RwLock`).
- Shape graph uploads and deletions are transactional within the store.
- Concurrent ingestion under `enforce` mode: each transaction validates independently against the shapes as they existed at transaction start (snapshot isolation).

### Rudof Integration

- Validation is performed by calling `shacl_validation::validate()` via the SRDF trait implementation on `Store<B>`.
- The shapes graph is loaded into `rudof`'s `shacl_ast::Schema` at upload time and cached in memory. Cache is invalidated on shape deletion or upload.
- For the `POST /shacl/validate` endpoint, a read-only snapshot is used so validation does not block concurrent writes.

### ShaclMode Enum Mapping

The `ShaclMode` enum in `oxigraph-shacl/src/validator.rs` maps directly to the API:

| API value | Rust enum |
|-----------|-----------|
| `"off"` | `ShaclMode::Off` |
| `"warn"` | `ShaclMode::Warn` |
| `"enforce"` | `ShaclMode::Enforce` |
