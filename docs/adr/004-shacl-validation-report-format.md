# ADR-004: SHACL Validation Report Format

## Status: Accepted

## Context

When SHACL validation is enabled in `enforce` mode (PLAN task 3.5), SPARQL UPDATE requests or bulk loads that violate shape constraints must be rejected. We need to define what the server returns to the client in this case: the HTTP status code, response content type, and body format. The format must convey enough detail for the client to understand which triples violated which constraints, while remaining practical for both RDF-native tooling and general-purpose API consumers (e.g., web applications, CI pipelines).

The W3C SHACL specification defines a [Validation Report vocabulary](https://www.w3.org/TR/shacl/#validation-report) built on these core classes and properties:

- `sh:ValidationReport` -- top-level report resource
- `sh:conforms` -- boolean, true if no violations
- `sh:result` -- links to individual `sh:ValidationResult` nodes
- `sh:ValidationResult` properties:
  - `sh:focusNode` -- the node that was validated
  - `sh:resultPath` -- the property path involved
  - `sh:value` -- the offending value (if applicable)
  - `sh:sourceShape` -- the shape that produced the violation
  - `sh:sourceConstraintComponent` -- the specific constraint component
  - `sh:resultSeverity` -- `sh:Violation`, `sh:Warning`, or `sh:Info`
  - `sh:resultMessage` -- human-readable description

Rudof's `shacl_validation` module already produces validation reports using this vocabulary internally. The question is how to serialize and deliver them over HTTP.

## Options Considered

### (a) W3C SHACL Validation Report as RDF (Turtle or JSON-LD)

Return the validation report serialized as standard RDF, using content negotiation (`Accept` header) to choose between Turtle, JSON-LD, N-Triples, or RDF/XML.

- **Pros**: Fully standards-compliant. Clients that already understand SHACL can parse the report directly. The report is itself an RDF graph, so it can be stored, queried, or merged with other RDF data. Rudof already produces this format internally, so serialization is straightforward.
- **Cons**: Verbose for simple error cases. JSON-LD in particular can be difficult to consume without an RDF library. General-purpose API consumers (curl, JavaScript frontends, CI scripts) must parse RDF to extract error messages. Turtle is human-readable but not machine-friendly for non-RDF tooling.

### (b) Simplified JSON Error Format

Return a custom JSON structure that maps the key SHACL validation report fields to a flat, easy-to-consume schema.

- **Pros**: Immediately consumable by any HTTP client. No RDF library needed. Easy to integrate into error-handling middleware, logging, and monitoring. Compact.
- **Cons**: Non-standard -- every consumer must learn our custom schema. Loses the semantic richness of the W3C vocabulary. If clients need to forward the report to another SHACL-aware system, they must transform it back. We would need to maintain the mapping as the W3C vocabulary evolves.

### (c) Both, via Content Negotiation

Return the W3C RDF format when the client sends `Accept: text/turtle`, `Accept: application/ld+json`, or other RDF media types. Return the simplified JSON format when the client sends `Accept: application/json` or does not specify a preference. The `Content-Type` response header reflects the actual format returned.

- **Pros**: Standards-compliant clients get what they expect. General-purpose clients get something practical. The default (no Accept header or `application/json`) is the simplified JSON, which optimizes for the common case. No information is lost -- both formats are generated from the same internal `sh:ValidationReport` structure.
- **Cons**: Two serialization paths to maintain and test. Slightly more complex server implementation. Clients must be aware that the response format depends on their `Accept` header.

## Decision

**Both, via content negotiation (option c).**

The cost of maintaining two serialization paths is low because both derive from the same internal rudof `ValidationReport` structure. The simplified JSON format is a direct field-by-field mapping of the W3C vocabulary, not an independent schema, so it stays synchronized by construction. Content negotiation is already a standard HTTP mechanism that Oxigraph's SPARQL endpoint uses for query results (JSON, XML, CSV).

### HTTP Status Code

Validation failures return **`422 Unprocessable Entity`**.

Rationale: The request is syntactically valid (it parses as correct SPARQL UPDATE or valid RDF payload), but the server cannot process it because the data violates semantic constraints defined by SHACL shapes. This aligns with RFC 4918 Section 11.2 and is the established convention for "the request body is well-formed but semantically invalid" in REST APIs. We do not use `400 Bad Request` because the request itself is not malformed. We do not use `409 Conflict` because SHACL violations are not resource state conflicts.

The response includes a `Link` header pointing to the SHACL shapes graph that produced the violation, enabling clients to retrieve the shapes for programmatic analysis:

```
HTTP/1.1 422 Unprocessable Entity
Content-Type: application/json
Link: </shacl>; rel="describedby"
```

### Simplified JSON Format

When the client accepts `application/json` (or sends no `Accept` header), the response body uses this structure:

```json
{
  "conforms": false,
  "results": [
    {
      "focusNode": "<IRI or blank node identifier>",
      "resultPath": "<predicate IRI>",
      "value": "<offending value, if applicable>",
      "sourceShape": "<shape IRI>",
      "sourceConstraintComponent": "<constraint component IRI>",
      "resultSeverity": "Violation | Warning | Info",
      "resultMessage": "Human-readable description"
    }
  ]
}
```

All IRI values are serialized as full IRIs (not prefixed names) to avoid requiring prefix context. The `value` field is omitted when not applicable (e.g., cardinality violations where the problem is a missing value). The `resultSeverity` field uses the local name only (`Violation`, `Warning`, `Info`) rather than the full `sh:` IRI, for readability.

### W3C RDF Format

When the client accepts an RDF media type (`text/turtle`, `application/ld+json`, `application/n-triples`, `application/rdf+xml`), the response body is the standard W3C SHACL Validation Report serialized in the requested format. This is produced directly from rudof's internal report structure.

## Examples

### Example 1: Cardinality Violation

A shape requires every `ex:Person` to have exactly one `ex:name`. A SPARQL UPDATE inserts a person with no name.

**Request:**
```sparql
INSERT DATA {
  <http://example.org/alice> a <http://example.org/Person> .
}
```

**Response (JSON, 422):**
```json
{
  "conforms": false,
  "results": [
    {
      "focusNode": "http://example.org/alice",
      "resultPath": "http://example.org/name",
      "sourceShape": "http://example.org/PersonShape-name",
      "sourceConstraintComponent": "http://www.w3.org/ns/shacl#MinCountConstraintComponent",
      "resultSeverity": "Violation",
      "resultMessage": "Property http://example.org/name: expected min count 1, got 0"
    }
  ]
}
```

**Response (Turtle, 422):**
```turtle
@prefix sh: <http://www.w3.org/ns/shacl#> .
@prefix ex: <http://example.org/> .

[] a sh:ValidationReport ;
    sh:conforms false ;
    sh:result [
        a sh:ValidationResult ;
        sh:focusNode ex:alice ;
        sh:resultPath ex:name ;
        sh:sourceShape ex:PersonShape-name ;
        sh:sourceConstraintComponent sh:MinCountConstraintComponent ;
        sh:resultSeverity sh:Violation ;
        sh:resultMessage "Property http://example.org/name: expected min count 1, got 0"
    ] .
```

### Example 2: Datatype Constraint Violation

A shape requires `ex:age` values to be `xsd:integer`. A SPARQL UPDATE inserts a string value.

**Request:**
```sparql
INSERT DATA {
  <http://example.org/bob> a <http://example.org/Person> ;
    <http://example.org/name> "Bob" ;
    <http://example.org/age> "not a number" .
}
```

**Response (JSON, 422):**
```json
{
  "conforms": false,
  "results": [
    {
      "focusNode": "http://example.org/bob",
      "resultPath": "http://example.org/age",
      "value": "not a number",
      "sourceShape": "http://example.org/PersonShape-age",
      "sourceConstraintComponent": "http://www.w3.org/ns/shacl#DatatypeConstraintComponent",
      "resultSeverity": "Violation",
      "resultMessage": "Value \"not a number\" does not have datatype http://www.w3.org/2001/XMLSchema#integer"
    }
  ]
}
```

**Response (Turtle, 422):**
```turtle
@prefix sh: <http://www.w3.org/ns/shacl#> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
@prefix ex: <http://example.org/> .

[] a sh:ValidationReport ;
    sh:conforms false ;
    sh:result [
        a sh:ValidationResult ;
        sh:focusNode ex:bob ;
        sh:resultPath ex:age ;
        sh:value "not a number" ;
        sh:sourceShape ex:PersonShape-age ;
        sh:sourceConstraintComponent sh:DatatypeConstraintComponent ;
        sh:resultSeverity sh:Violation ;
        sh:resultMessage "Value \"not a number\" does not have datatype xsd:integer"
    ] .
```

## Consequences

- The server must implement two serialization paths for validation reports: simplified JSON and W3C RDF. Both derive from rudof's internal `ValidationReport` struct, keeping them consistent.
- Content negotiation logic must be added to the validation error response path. This reuses the existing `Accept` header parsing that the SPARQL endpoint already performs.
- API documentation must describe both formats, the `422` status code convention, and the `Link` header.
- In `warn` mode (PLAN task 3.5), the validation report is included in response headers or a separate log endpoint rather than as the response body, since the request succeeds with `2xx`. This detail is deferred to the implementation phase.
- The simplified JSON format is intentionally a 1:1 mapping of the W3C vocabulary fields. If the W3C vocabulary adds new properties, they can be added to the JSON format without breaking existing consumers (additive change).
- Test suite (PLAN task 3.6) must verify both response formats for each validation scenario.
