# API Reference

## SPARQL Endpoints

### Query (GET)
```
GET /query?query=SELECT+*+WHERE+{+?s+?p+?o+}
Accept: application/sparql-results+json
```

### Query (POST)
```
POST /query
Content-Type: application/sparql-query

SELECT * WHERE { ?s ?p ?o } LIMIT 10
```

### Update (POST) — requires auth
```
POST /update
Content-Type: application/sparql-update
Authorization: Bearer <write-key>

INSERT DATA { <http://ex.org/s> <http://ex.org/p> "value" }
```

## Graph Store Protocol

### Load Data (POST) — requires auth
```
POST /store
Content-Type: text/turtle
Authorization: Bearer <write-key>

@prefix ex: <http://example.org/> .
ex:s ex:p ex:o .
```
Supported formats: `text/turtle`, `application/n-triples`, `application/n-quads`, `application/rdf+xml`

### Dump Data (GET)
```
GET /store
Accept: application/n-quads
```

## SHACL Management

### Upload Shapes (POST) — requires auth
```
POST /shacl/shapes
Content-Type: text/turtle
Authorization: Bearer <write-key>

# SHACL shapes in Turtle format
```
Response: `201 Created` with `{"loaded": true, "shape_count": N}`

### Get Shapes Info (GET)
```
GET /shacl/shapes
```
Response: `{"loaded": true, "shape_count": N}`

### Delete Shapes (DELETE) — requires auth
```
DELETE /shacl/shapes
Authorization: Bearer <write-key>
```

### Validate Store (POST)
```
POST /shacl/validate
```
Response: `{"conforms": true/false, ...}`

### Get/Set Validation Mode
```
GET /shacl/mode
PUT /shacl/mode
Content-Type: application/json
Authorization: Bearer <write-key>

{"mode": "enforce"}
```
Valid modes: `off`, `warn`, `enforce`

## Health Endpoints

### Liveness
```
GET /health
```
Returns `200 OK` if server is alive.

### Readiness
```
GET /ready
```
Returns `200 READY` if backend connection is healthy.

## Authentication

Write operations require `Authorization: Bearer <key>` when `--write-key` is set. Read operations (queries, health checks) are always open.

## Error Responses

| Status | Meaning |
|--------|---------|
| 400 | Bad request (invalid SPARQL, missing parameters) |
| 401 | Unauthorized (missing or invalid write key) |
| 406 | Not acceptable (unsupported Accept header) |
| 408 | Request timeout (query exceeded timeout) |
| 415 | Unsupported media type |
| 422 | Unprocessable entity (SHACL validation failure) |
| 500 | Internal server error |
