# **Architecting a Cloud-Native, Distributed SPARQL and SHACL Database: Integrating Rudof and Evaluating Storage Backends for Oxigraph**

## **Introduction to Cloud-Native Graph Database Evolution**

The landscape of semantic web technologies and knowledge graph storage is undergoing a profound paradigm shift. Historically, graph databases and RDF (Resource Description Framework) triplestores have relied on tightly coupled, single-node architectures. These monolithic designs bind the query execution engine, the storage manager, and the in-memory buffer pool to the exact same physical or virtual machine. While this tightly coupled approach optimizes local disk I/O and memory access patterns, it fundamentally limits the horizontal scalability, fault tolerance, and high availability required in modern cloud-native environments. As enterprises increasingly rely on massive, highly interconnected datasets for real-time artificial intelligence, recommendation systems, and complex analytics, the limitations of single-node triplestores become severe operational bottlenecks.

Oxigraph represents the cutting edge of modern RDF databases. Written entirely in the Rust programming language, Oxigraph provides a highly compliant, memory-safe, and rapidly evolving implementation of the SPARQL 1.1 standard.1 Its core philosophy emphasizes safety and performance, leveraging Rust's strict compiler guarantees to eliminate data races and memory vulnerabilities that often plague complex C++ database engines.1 However, Oxigraph's current persistence architecture is inherently bound to embedded, single-node key-value stores. By default, it utilizes RocksDB, a highly optimized, single-machine Log-Structured Merge-tree (LSM-tree) storage engine developed by Facebook.3 While RocksDB provides an excellent tradeoff between read and write performance on local NVMe storage, it does not possess native capabilities for distributed consensus, geographic replication, or decoupled storage and compute scaling.3

To transform Oxigraph from a high-performance embedded database into a powerful, cloud-native SPARQL and SHACL (Shapes Constraint Language) platform, two major architectural evolutions are mandatory. First, the database must natively integrate robust SHACL validation capabilities to ensure schema compliance and data quality across massive, evolving knowledge graphs. This requirement can be optimally fulfilled by integrating the Rust-native rudof crate.2 Second, Oxigraph's internal storage abstraction must be fundamentally decoupled from RocksDB, allowing the compute layer—which handles query parsing, optimization, and execution—to interface with external, horizontally scalable, cloud-native storage backends.3

This comprehensive research report provides an exhaustive architectural analysis of how to execute this transformation. It thoroughly examines the mechanics of integrating the rudof ecosystem into Oxigraph. Furthermore, it deeply evaluates the most performant and capable cloud-native storage architectures available—specifically TiKV, FoundationDB, Amazon DynamoDB, and S3-backed columnar object storage. By dissecting the transactional guarantees, range-scan performance, network bottleneck mitigation strategies, and distributed query execution paradigms of each backend, this report outlines the optimal technological pathway for engineering a distributed, cloud-native semantic database.

## **Analyzing Oxigraph's Current Storage Architecture and Encoding**

To accurately evaluate the suitability of any distributed key-value store, it is critical to first deconstruct how Oxigraph currently maps the complex, multi-dimensional structure of an RDF graph onto a flat, one-dimensional key-value abstraction. The efficiency of a graph database is heavily dependent on how quickly it can traverse relationships—a process that translates to sequential range scans over underlying data structures.

### **Key-Value Table Indexing**

Oxigraph currently implements its underlying persistent storage by instantiating eleven distinct key-value tables within RocksDB.3 This approach relies on a massive dictionary encoding strategy and multiple index permutations to ensure that any valid SPARQL triple or quad pattern can be resolved using contiguous, sequential read operations.

The eleven tables are structurally divided to optimize different types of lookups:

1. **The Dictionary Table (id2str):** A single table dedicated to mapping unique, compressed string identifiers (hashes or specialized numerical representations) back to their full, uncompressed string representations.3 This dictionary encoding is vital for reducing the physical storage footprint of repeated URIs, IRIs, and literal values across the graph.  
2. **The Default Graph Quad Tables:** Oxigraph maintains three primary tables for triples residing in the default graph, sorting them by different permutations of their components:  
   * **SPO (Subject ![][image1] Predicate ![][image1] Object):** Optimized for queries where the subject is known, allowing the engine to sequentially scan all predicates and objects attached to that specific subject.3  
   * **POS (Predicate ![][image1] Object ![][image1] Subject):** Optimized for queries traversing inbound links, identifying all subjects connected to a specific object via a specific predicate.3  
   * **OSP (Object ![][image1] Subject ![][image1] Predicate):** Optimized for queries where only the object is known, or for identifying patterns radiating backwards from a destination node.3  
3. **The Named Graph Tables:** To support SPARQL 1.1 Named Graphs, Oxigraph utilizes additional tables that incorporate the Graph component (G) into the sorting permutations (e.g., SPOG, POSG, OSPG).3  
4. **The Graph Directory Table:** A single table that maintains a definitive list of all existing named graphs within the database.3

### **Byte-Level Term Encoding**

When transitioning to a distributed database, the physical encoding of keys becomes a paramount concern. Distributed key-value stores generally partition and route data based on the lexicographical sorting of raw byte arrays.6 Oxigraph's term encoding strategy is highly optimized for this exact paradigm.

Oxigraph encodes RDF terms using a leading type byte that defines the "kind" of the term, followed by a fixed-length value composed of at most 32 bytes (with exceptions made for inline RDF-star quoted triples).3 The length and structure of the trailing value depend strictly on the term kind defined by the leading byte. Key encoding formats include:

* **NamedNode**: Encoded using a 128-bit cryptographic hash of its full string representation.3  
* **NumericalBlankNode**: If an auto-generated blank node identifier perfectly fits within a hexadecimal number up to u128, it is stored directly as a numerical value, bypassing hash collisions.3  
* **SmallBlankNode and BigBlankNode**: Identifiers shorter than 16 bytes are stored inline, while larger identifiers are hashed.3  
* **Literal Values**: Categories such as SmallStringLiteral, BigStringLiteral, SmallSmallLangStringLiteral, and various combinations of numeric data types are encoded to prioritize inline storage where possible, falling back to hashed representations when string lengths exceed the 32-byte constraint.3

Because Oxigraph relies on these 32-byte structures to form the keys within its SPO, POS, and OSP index tables, any distributed storage backend chosen must provide robust, highly efficient support for ordered, lexicographical range scans. If a backend relies purely on random hashing for partition distribution (like standard Memcached or default DynamoDB partitions), it will catastrophically fail to support the sequential prefix scans required to resolve SPARQL Basic Graph Patterns (BGPs).

### **Transactional Guarantees and Concurrency Control**

In its current RocksDB implementation, Oxigraph enforces an atomic batch write strategy. Writes are buffered and executed as an atomic batch at the exact end of a transaction.3 To ensure read consistency, the database takes a snapshot at the beginning of the transaction, providing "repeatable read" isolation semantics.3 This means that the store only exposes changes that have been fully committed, and the state of the graph remains immutable for the entire duration of a read operation (like a complex SPARQL query).8

However, this simplistic approach leads to limitations in write concurrency. In its purely in-memory mode, Oxigraph implements Multiversion Concurrency Control (MVCC) but currently restricts the system to a single concurrent write transaction at any given time, effectively upgrading the isolation level to full serializability but severely bottlenecking write throughput under heavy load.3 A cloud-native backend must alleviate this bottleneck by offering robust, distributed MVCC mechanisms capable of handling concurrent, highly contentious write workloads without sacrificing ACID (Atomicity, Consistency, Isolation, Durability) guarantees.9

## **Integrating SHACL Validation via the Rudof Crate**

Ensuring the structural integrity and semantic consistency of data entering a knowledge graph is a critical operational requirement. The W3C standard for defining these structural constraints is the Shapes Constraint Language (SHACL).2 Integrating SHACL validation directly into a cloud-native Oxigraph architecture provides an automated mechanism to enforce data quality at the point of ingestion. The rudof Rust crate provides an ideal, highly performant mechanism for achieving this integration.

### **The Rudof Architecture and Ecosystem**

rudof is a modern, flexible Rust library explicitly designed for parsing, validating, and managing RDF data shapes.2 It supports both ShEx (Shape Expressions) and SHACL, alongside other data modeling formalisms like DCTAP, and provides seamless conversion mechanisms between these disparate languages.2

The architectural design of rudof is highly modular, reflecting the best practices of the Rust ecosystem. The repository is divided into multiple independent crates that can be included selectively to minimize binary bloat and dependency overhead.4 The core modules relevant to an Oxigraph integration include:

* **iri\_s**: Defines simple, fast structures for handling Internationalized Resource Identifiers.4  
* **srdf**: The Simple RDF model, which provides the foundational abstraction layer used for all validation algorithms.4  
* **prefixmap**: Implements prefix map resolution, heavily utilized during the parsing of Turtle or TriG serializations.4  
* **shacl\_ast**: Defines the core Abstract Syntax Tree representation of SHACL shapes.4  
* **shacl\_validation**: Contains the highly optimized algorithmic logic required to validate target RDF graphs against the shacl\_ast definitions.4

By leveraging Rust's powerful memory safety paradigms, static typing, and zero-cost abstractions, rudof achieves execution performance that directly competes with, and often exceeds, traditional low-level system languages.2 Furthermore, rudof is cross-platform, offering binaries for Linux, Windows, macOS, Docker environments, and bindings for Python (pyrudof), while also supporting compilation to WebAssembly for execution in browser environments or edge runtimes.2

### **The SRDF Trait Abstraction: Bridging Rudof and Oxigraph**

The technical integration between the rudof SHACL validation engine and the Oxigraph storage backend is orchestrated exclusively through the SRDF (Simple RDF) trait.2

To minimize external dependencies and maintain absolute flexibility, the architects of rudof designed the SRDF trait to define the absolute minimum subset of RDF functionalities required to perform shape validation.12 The primary capability mandated by the SRDF trait is the ability to efficiently access the "neighborhood" of a specific node within the graph—for example, retrieving all outbound predicates and objects associated with a specific subject node to verify if it satisfies a sh:property constraint.12

Currently, the rudof repository provides default implementations of the SRDF trait for basic, in-memory parsed RDF files (such as Turtle documents) and remote SPARQL endpoints.12 To natively integrate rudof into Oxigraph, developers must construct a custom implementation of the SRDF trait directly for Oxigraph's internal Store struct.8

Because Oxigraph inherently maintains deeply optimized SPO (Subject-Predicate-Object) and POS (Predicate-Object-Subject) index tables within its underlying storage, implementing the SRDF neighborhood retrieval methods is highly efficient. When the shacl\_validation module requests the neighborhood of a node, the custom SRDF implementation will map that request directly to Oxigraph's internal quads\_for\_pattern(Some(subject), None, None, None) iterator method.8 This bypasses the entirety of Oxigraph's spareval (SPARQL evaluator) and spargebra (SPARQL parser) logic, routing the validation request directly to the raw, lexicographical range scans of the underlying key-value store.14 This direct storage integration minimizes CPU overhead and latency, allowing the validation engine to operate at near bare-metal disk speeds.

### **Performance Benchmarks of Rudof**

The theoretical benefits of utilizing a Rust-based SHACL validator are decisively proven by empirical performance benchmarks. In benchmark testing conducted using the 10-LUBM (Lehigh University Benchmark) dataset, evaluating the exact same SHACL shapes across state-of-the-art validation engines, rudof demonstrated exceptional performance characteristics.2

| SHACL Validation Engine | Implementation Language | Execution Time (ms) |
| :---- | :---- | :---- |
| **rdf4j** | Java | 1.6447 |
| **rudof** | Rust | 7.8971 |
| **Apache Jena** | Java | 60.3583 |
| **TopQuadrant** | Java | 85.7421 |
| **pyrudof (Python Bindings)** | Python / Rust | 39,364.2842 |
| **pySHACL** | Python | 72,227.2940 |

*Table 1: Performance comparison of SHACL validation across state-of-the-art implementations executing against the 10-LUBM dataset.* 2

While the highly mature, dynamically optimized JVM-based rdf4j engine slightly outperformed the current iteration of rudof, the Rust implementation drastically outperformed industry-standard enterprise engines like Apache Jena and TopQuadrant, executing validation up to 10.8x faster than TopQuadrant.2 Furthermore, for Python-based data engineering pipelines, pyrudof executed nearly twice as fast as the native pySHACL library, showcasing the profound benefits of binding high-level interfaces to low-level Rust computational cores.2 Integrating this engine directly into Oxigraph's ingestion pipeline guarantees that SHACL compliance validation will not become a computational bottleneck, even under massive write workloads.

## **The Theory of Cloud-Native Graph Storage and Distributed SPARQL**

To evaluate potential cloud-native storage backends for Oxigraph, one must first comprehend the unique operational pressures that a SPARQL evaluation engine places on a distributed database.

Database workloads are generally classified into two distinct paradigms: Online Transaction Processing (OLTP) and Online Analytical Processing (OLAP).16 OLTP workloads involve massive volumes of short, highly concurrent, atomic transactions consisting of simple point-lookups or small updates (e.g., fetching a specific user profile or updating a single graph edge).16 Conversely, OLAP workloads involve complex, long-running queries that aggregate massive volumes of historical data across the entire database (e.g., executing a deep, multi-hop SPARQL path traversal to find fraud rings within a financial network).16 Oxigraph explicitly aims to provide an efficient compromise, balancing both OLTP and OLAP requirements within a unified engine.3

When migrating from a local RocksDB instance to a distributed architecture, the primary technological hurdle shifts from disk I/O latency to network communication latency.16 In a traditional "shared-nothing" distributed architecture, the RDF graph is partitioned across dozens or hundreds of remote storage nodes.16 When a complex SPARQL query is executed, the central compute node must request intermediate result sets from these disparate storage nodes.

If the underlying storage engine acts merely as a "dumb" key-value store, it will transmit gigabytes of raw, unfiltered triples back to the compute node across the network, where the compute node will then attempt to perform massive, memory-exhausting hash joins.19 This phenomenon is universally recognized as the primary bottleneck in distributed SPARQL engines.19 Therefore, the optimal cloud-native storage candidate for Oxigraph must not only provide durable, highly available, and strongly consistent storage, but it must also provide mechanisms to push down computational logic (filtering, aggregating, and joining) directly to the storage nodes, thereby minimizing the volume of data transmitted over the network.17

## **Option 1: TiKV – The Optimal Rust-Native Distributed Engine**

TiKV represents the most compelling and architecturally aligned candidate for serving as Oxigraph's distributed cloud-native backend. TiKV is an open-source, Apache 2.0 licensed, distributed transactional key-value database explicitly designed for cloud-native architectures.22 Crucially, like Oxigraph, TiKV is implemented entirely in Rust.22 It was originally engineered as the foundational storage layer for TiDB, a highly successful distributed Hybrid Transactional/Analytical Processing (HTAP) SQL database.22

### **Architecture, Partitioning, and Consensus**

TiKV achieves high availability and fault tolerance by utilizing the Raft consensus algorithm.7 Every modification to the database is recorded as a Raft log and synchronously replicated to a quorum of nodes before the transaction is acknowledged as committed.7

To manage massive datasets, TiKV dynamically shards its key space into logical units called Regions.23 Each Region is typically 96 MB in size and stores a strictly continuous, lexicographically sorted range of keys.23 This range-based partitioning strategy is absolutely critical for an RDF triplestore.28 Because Oxigraph encodes its SPO, POS, and OSP indexes to group related data contiguously (e.g., all predicates and objects for the subject http://example.com/NodeA share an identical key prefix), TiKV's Region architecture naturally ensures that these related graph structures reside closely together, often within the exact same physical Region on a single machine.3 This localization drastically accelerates the prefix range scans required by Oxigraph's quads\_for\_pattern iterators.8

Underlying each TiKV node is a highly tuned, local instance of RocksDB (or TitanDB for value separation), which physically writes the data to the node's local NVMe storage.7 The orchestration of these nodes, including load balancing, leader election, and Region splitting, is handled autonomously by a centralized control plane known as the Placement Driver (PD).24

### **Transactional Guarantees**

TiKV provides rigorous ACID semantics and strictly serializable distributed transactions.24 It achieves this by employing a decentralized, two-phase commit protocol heavily inspired by Google's Percolator model.24 The Placement Driver (PD) acts as a centralized timestamp allocator, assigning a monotonically increasing timestamp to every transaction to detect and resolve conflicts deterministically across the distributed cluster.24 This architecture perfectly aligns with Oxigraph's requirement for "repeatable read" and atomic commit isolation levels, ensuring that multi-step graph updates (e.g., deleting a node and all its inbound/outbound edges) remain completely consistent, even under heavy concurrent access.3

### **Performance Characteristics**

TiKV is engineered to deliver predictable, low-latency performance at scale.31 Extensive benchmarking utilizing the industry-standard Yahoo\! Cloud Serving Benchmark (YCSB) confirms its capability.31

| Cluster Configuration | Workload Profile (10M Records) | Operations Per Second (OPS) | P99 Latency |
| :---- | :---- | :---- | :---- |
| 3-Node TiKV (40 vCPUs, NVMe SSD) | YCSB Workload C (100% Read, Point Get) | 212,000 OPS | \< 10 ms |
| 3-Node TiKV (40 vCPUs, NVMe SSD) | YCSB Workload A (50% Read / 50% Update) | 43,200 OPS | \< 10 ms |

*Table 2: TiKV Cluster Performance on standard YCSB workloads demonstrating massive throughput within strict latency boundaries.* 31

These metrics prove that TiKV can easily sustain the heavy, randomized point-read and write throughput generated by real-time SHACL validation pipelines or interactive semantic web applications.

### **The Coprocessor Paradigm: Solving the SPARQL Network Bottleneck**

The paramount advantage of TiKV for Oxigraph is its Coprocessor framework.32 As previously established, fetching millions of raw triples across the network to evaluate a complex SPARQL query represents a fatal performance bottleneck.17

TiKV solves this by adopting a distributed compute-to-data paradigm.29 The TiKV Coprocessor allows a compute node (in this case, Oxigraph's spareval engine) to construct a Directed Acyclic Graph (DAG) of physical execution plans and push that DAG directly down to the TiKV storage nodes via gRPC.29

Instead of executing a naive GetRange operation, Oxigraph could translate a SPARQL query like SELECT COUNT(?o) WHERE { \<SubjectA\> \<PredicateB\>?o } into a Coprocessor DAG containing an IndexScan node (to locate the specific POS keys) followed immediately by an Aggregation node.29 TiDB leverages this exact mechanism to push TableScan, Selection (filtering predicates), and Aggregation operations directly into the RocksDB layer of the TiKV nodes.35 The physical TiKV nodes execute the scan and count the matching elements locally, returning only a single integer (the partial sum) back to the Oxigraph compute node, reducing network transmission overhead by orders of magnitude.29

The profound efficacy of this paradigm for graph workloads was empirically proven by PingCAP's internal project, "TiGraph".38 By mapping graph data structures into TiKV and pushing graph-specific traversal calculations down via the Coprocessor, TiGraph achieved query performance improvements of up to 8,700x compared to executing identical logic at the SQL compute layer.38

Furthermore, TiKV features a native Coprocessor Cache.40 This mechanism caches the results of complex push-down calculations at the Region level within the memory of the compute instance.40 If a subsequent SPARQL query hits an identical BGP pattern, and the underlying Region has not experienced any write mutations, the engine serves the result instantly from memory, bypassing the storage network entirely.40

### **Consequences and Operational Complexity**

The primary consequence of adopting TiKV is the steep increase in operational complexity. Managing a TiKV deployment requires orchestrating not only the TiKV storage nodes but also maintaining a highly available quorum of Placement Driver (PD) nodes.24 Because Oxigraph heavily utilizes tiny, highly fragmented key-value pairs, a massive knowledge graph will generate tens of millions of distinct TiKV Regions.23 An excessive number of Regions can saturate the Raftstore module as it continuously polls to process Raft heartbeats for each Region, leading to severe CPU overhead and delayed append log operations.23 Mitigating this requires advanced operational tuning, such as enabling the "Hibernate Region" feature to suppress heartbeats for idle data, or aggressively tuning the Region Merge configurations.23

## **Option 2: FoundationDB – Unyielding Safety and Serializability**

FoundationDB (FDB) represents another elite tier of distributed key-value storage. Originally developed as an independent product, it was acquired by Apple, battle-tested internally to support massive cloud infrastructures like CloudKit, and subsequently open-sourced.43 While its core engine is written in C++, the FoundationDB community maintains foundationdb-rs, a highly mature, production-ready Rust binding crate with over 5 million downloads.44

### **Architecture and The Record Layer Concept**

FoundationDB abstracts its distributed complexities entirely, presenting developers with a unified, ordered, lexicographical key-value space.47 It is globally renowned for its unique deterministic simulation testing framework, which systematically injects network partitions, disk failures, and clock skews during development to mathematically guarantee that its strict ACID properties hold under any conceivable disaster scenario.45

A highly relevant architectural pattern to examine when evaluating FoundationDB for Oxigraph is Apple's "Record Layer".47 FoundationDB was explicitly designed to act as a foundational substrate; it intentionally lacks complex features like native secondary indexing or query parsing.50 Instead, it expects developers to build stateless "layers" on top of the KV store.43 The Record Layer is exactly this—a stateless compute engine that provides relational database semantics, schema management, protobuf serialization, and advanced query planning (including JOINs and aggregations) entirely on top of the raw FDB keyspace.47

Oxigraph fits perfectly into this paradigm. By decoupling the Store struct, Oxigraph's compute modules (spargebra and spareval) effectively become a stateless "Graph Layer" operating on top of FoundationDB, translating complex SPARQL syntax into fundamental KV transactions.14

### **Latency Profiles and Asynchronous Execution**

FoundationDB is optimized for extreme throughput and concurrent transaction execution, but it exhibits higher baseline latency for individual, synchronous operations compared to an embedded RocksDB instance.51 This is due to the inherent requirement of contacting a proxy node to acquire a read version timestamp before commencing any transaction.51

For an RDF database like Oxigraph, which constantly performs range scans to iterate over triple patterns, the client-side execution methodology heavily dictates performance.52 Benchmark analysis comparing FoundationDB's Java API (which mirrors the underlying C API used by the Rust bindings) reveals critical differences between asynchronous iteration and batched execution:

| Range Size (Key-Value Pairs fetched) | Execution using asList().get() (Batched) | Execution using iterator() (Sequential Async) |
| :---- | :---- | :---- |
| 10 | 1.237 ms | 1.920 ms |
| 100 | 1.515 ms | 2.947 ms |
| 1,000 | 3.316 ms | 6.013 ms |

*Table 3: Latency comparison of FoundationDB range queries based on client fetch methodologies. Keys are 8 bytes, values are 100 bytes.* 52

As demonstrated, attempting to iterate sequentially through a range scan using iterator() introduces significant latency drag due to repeated asynchronous network polling.52 To optimize Oxigraph for FoundationDB, the internal quad iterators must be comprehensively rewritten to leverage aggressive prefetching and bulk-batching mechanisms (analogous to asList().get()).52

### **The Five-Second Limitation and the Lack of Pushdown**

The most severe architectural consequence of utilizing FoundationDB for a SPARQL engine is its draconian 5-second transaction time limit.53 FoundationDB achieves its remarkable throughput by utilizing a specialized Multi-Version Concurrency Control (MVCC) architecture where the transaction logs and resolving proxies maintain state in-memory.54 To prevent memory exhaustion, any transaction—whether reading or writing—that exceeds 5 seconds is unilaterally aborted and rolled back by the cluster.54

For a graph database executing OLAP-style SPARQL queries, this limitation is catastrophic. A complex query attempting to aggregate values across a multi-billion-triple graph will almost certainly exceed five seconds.3 To circumvent this, the Oxigraph compute layer would need to implement complex continuation tokens—similar to the RecordScanLimiter utilized by the Apple Record Layer—which artificially partition long-running queries into sequences of sub-5-second transactions, carefully reconstructing the cursor state across boundaries.49 This introduces massive application-level complexity and invalidates true snapshot isolation for long-running analytics.

Furthermore, unlike TiKV, FoundationDB does not natively support Coprocessor pushdowns. The execution model is strictly scatter-gather; all filtering, aggregations, and join evaluations must occur within the Oxigraph compute node, drastically amplifying network bottlenecks.57

## **Option 3: Amazon DynamoDB – The Cost of the Impedance Mismatch**

Amazon DynamoDB represents the pinnacle of fully managed, serverless NoSQL databases. It delivers massive scalability, multi-region active-active replication, and consistent single-digit millisecond latency without requiring developers to manage infrastructure, patch operating systems, or tune storage engines.58 For enterprise deployments prioritizing operational simplicity, it appears highly attractive.

### **Architecture and Partitioning Constraints**

However, DynamoDB's architectural design introduces a fatal impedance mismatch with the requirements of a SPARQL evaluation engine. DynamoDB organizes data using a composite primary key consisting of a Partition Key (used as input for an internal hash function to distribute data physically) and an optional Sort Key (used to order items sharing the same Partition Key).60

To execute a query efficiently, DynamoDB requires the exact value of the Partition Key to be known in advance.62 If an application attempts to query data without specifying the Partition Key, the database must execute a full Scan operation, sweeping the entire multi-terabyte table.62

### **Unsuitability for Arbitrary Graph Queries**

In a SPARQL query, the known variables can appear in any position. A query might ask for all subjects linked to a specific object (SELECT?s WHERE {?s \<pred\> \<TargetObject\> }). To support this in DynamoDB, Oxigraph would be forced to create multiple Global Secondary Indexes (GSIs) to mirror its POS and OSP structures.58 Even with GSIs, if a query contains partial patterns or wildcards that do not perfectly align with the leading element of the Partition Key, a full table scan is triggered.62

AWS bills DynamoDB based on Read Capacity Units (RCUs). During a full scan, RCUs are consumed for *every* item read from disk, even if a filter condition subsequently discards 99.9% of the items before returning the result.62 For a complex SPARQL query iterating over an RDF graph, executing via DynamoDB would result in prohibitive, potentially astronomical financial costs and severe latency penalties. While Amazon offers DynamoDB Accelerator (DAX) to provide microsecond in-memory caching for read-heavy workloads 59, DAX only accelerates data retrieval; it does not alleviate the fundamental inability to perform arbitrary, multi-dimensional range scans across the entire dataset without triggering massive RCU consumption. Therefore, DynamoDB is technically and economically unsuitable to act as a primary backend for Oxigraph.

## **Option 4: S3-Native Columnar Storage (Parquet / Arrow)**

A relatively recent innovation in cloud-native database architecture is the complete disaggregation of compute and storage utilizing cloud object storage (Amazon S3) combined with columnar data formats like Apache Parquet.16 Projects built in Rust, such as Databend and DataFusion, demonstrate that extremely fast OLAP query execution can be achieved directly against immutable files resting in S3.63

### **Advantages for Analytical SPARQL Workloads**

In traditional shared-nothing database architectures, adding compute nodes requires migrating and rebalancing terabytes of data across the new local disks—a process that degrades performance and takes hours.16 In an S3-native architecture, compute nodes are entirely stateless.16 If a user executes a massive, resource-intensive SPARQL query, the system can instantly spin up hundreds of transient compute nodes, pull the requisite Parquet files from S3, perform the calculation, and terminate the nodes.16

If Oxigraph were to export its SPO, POS, and OSP tables as tightly compressed Parquet files, complex analytical SPARQL queries (e.g., aggregations, grouping, large-scale inference) would benefit immensely from columnar pruning and inherent bloom filters, allowing the compute nodes to skip irrelevant data blocks entirely.63

### **Consequences and the OLTP Deficit**

The glaring consequence of this architecture is astronomical latency for single-row lookups. Network traversal and HTTP overhead to Amazon S3 take tens or hundreds of milliseconds, compared to the microsecond latency of local NVMe storage or distributed memory.16 While caching layers can mitigate this, S3-native architectures are fundamentally incapable of supporting the high-frequency, low-latency OLTP point-queries required by real-time SHACL validation workflows or interactive semantic web applications. Furthermore, because S3 objects are immutable, supporting real-time graph updates (SPARQL UPDATE) requires implementing a highly complex append-only metadata layer (similar to Apache Iceberg) with background compaction processes. While an excellent tertiary mechanism for archival or pure data warehousing, S3 cannot serve as the primary operational backend for a responsive Oxigraph implementation.

## **Mitigating Network Bottlenecks in Distributed SPARQL**

Assuming a highly capable distributed backend like TiKV or FoundationDB is selected, the Oxigraph compute engine must still be heavily optimized to mitigate the penalties of network traversal. As previously noted, the naive execution of distributed joins requires fetching entire intermediate datasets across the network, an operation that swiftly paralyzes query execution.19

### **Extended Vertical Partitioning (ExtVP) and Semi-Joins**

In advanced distributed SPARQL engines, minimizing data transfer is prioritized above all else. A mathematically proven optimization strategy involves Extended Vertical Partitioning (ExtVP) and the aggressive utilization of semi-joins.67

The premise relies on algebraic join decomposition. A join between two tables ![][image2] and ![][image3] on attributes ![][image4] and ![][image5] can be factored to pre-filter the data before transmission:

![][image6] 67

In the context of an RDF graph, this translates to precomputing structural correlations between triples. A distributed engine can utilize Subject-Subject (SS), Object-Subject (OS), and Subject-Object (SO) correlations.67 For instance, if a query joins ?person foaf:name?name and ?person dbpedia:birth?birthdate 66, an OS semi-join filter can be passed to the storage nodes, guaranteeing that the storage node only transmits ?person records across the network if it can mathematically verify that the corresponding birthdate record also exists.67

Implementing this logic requires profound enhancements to Oxigraph's internal sparopt (SPARQL Optimizer) crate.14 The optimizer must be rewritten to generate execution plans that prioritize passing heavily restrictive bloom filters or semi-join conditions down to the remote KV store before requesting data iterators.

### **Coprocessor Pushdowns vs. Scatter-Gather**

The stark divergence between TiKV and FoundationDB becomes apparent during query execution. If FoundationDB is utilized, Oxigraph must rely on a scatter-gather approach.57 The spareval engine must fetch batches of data from FDB, assemble them in memory, filter them locally, and execute the joins.15

Conversely, if TiKV is implemented, the sparopt AST (Abstract Syntax Tree) can be natively translated into TiKV Protobuf Coprocessor requests (tikvpb.proto).29 By wrapping BGP matching logic into custom Coprocessor plugins deployed directly onto the TiKV storage nodes, Oxigraph effectively transforms a "dumb" KV store into a fully distributed graph processing engine.29 The storage nodes iterate over the LSM-tree locally, apply the SPARQL FILTER logic, and return only the matching quads. This architecture effortlessly circumvents the fundamental network bandwidth constraints that have historically stifled distributed semantic databases.20

## **Architectural Abstraction: Engineering the Storage Trait in Rust**

To execute this cloud-native transformation, the codebase of Oxigraph must be structurally refactored. Currently, Oxigraph isolates its persistence mechanisms behind a primary Store struct, which relies on hardcoded enum dispatch to switch between the in-memory fallback and the RocksDB engine.8 While efficient for local environments, this rigid coupling prevents community developers from injecting external backends.

The required architectural evolution mandates the introduction of a generic StorageBackend trait.68 This paradigm has been highly successful in other Rust data engineering projects. For example, the agentdb middleware utilizes traits to abstract SQL, Key-Value, and Graph backends seamlessly behind unified API interfaces.70 Similarly, the confidentialcontainers project abstracts secure storage through a generic StorageBackend trait.69

The proposed Oxigraph Storage trait must define standard CRUD abstractions (get, put, delete), but critically, it must also expose batched variants (batch\_put, batch\_scan) to allow backend implementations (like TiKV and FDB) to minimize network round-trips.25 Furthermore, transitioning from a synchronous embedded database to a distributed backend introduces the complexities of asynchronous I/O. Because network calls are inherently blocking, the trait definitions will require advanced Rust features, utilizing Generic Associated Types (GATs) and Pin\<Box\<dyn Future\>\> to safely manage asynchronous iterators and transaction lifetimes across threads without violating Rust's strict borrow checker rules.44

By establishing this abstract boundary, Oxigraph can preserve its high-performance RocksDB implementation for embedded edge-computing use cases, while simultaneously empowering enterprise users to deploy the system against massive, distributed TiKV or FoundationDB clusters.

## **Conclusion**

The transformation of Oxigraph into a powerful, cloud-native SPARQL and SHACL semantic database is a highly viable engineering objective, provided specific architectural paradigms are embraced.

The integration of SHACL validation is elegantly resolved by adopting the Rust-native rudof crate. By engineering a custom implementation of rudof's SRDF trait that directly maps to Oxigraph's internal quads\_for\_pattern iterators, the system bypasses parser overhead and achieves direct access to the underlying storage indices. This yields a validation engine that leverages Rust's memory safety to dramatically outperform traditional JVM-based industry standards.

Selecting the optimal cloud-native storage backend requires careful navigation of the impedance mismatch between graph transversals and distributed key-value logic. Amazon DynamoDB and S3-backed Parquet storage present fatal flaws for this specific workload, suffering from prohibitive costs during arbitrary queries and insurmountable latency for transactional lookups, respectively. FoundationDB offers an incredibly robust, mathematically proven foundation for strictly serializable transactions, and its Apple Record Layer demonstrates the exact stateless compute paradigm Oxigraph requires. However, its draconian 5-second transaction limit severely cripples long-running SPARQL OLAP workloads.

Ultimately, TiKV stands as the paramount choice for a distributed Oxigraph backend. Its Rust-native heritage, Raft-based consensus, and Region-based key-range partitioning align perfectly with Oxigraph's lexicographically sorted SPO, POS, and OSP indexes. Most importantly, TiKV's Coprocessor framework provides the exact distributed execution mechanism necessary to push SPARQL aggregations and BGP filtering directly down to the storage nodes. This compute-to-data paradigm shatters the network latency bottlenecks that have historically plagued distributed semantic databases, paving the way for a highly scalable, real-time, cloud-native knowledge graph ecosystem.

#### **Works cited**

1. oxigraph/oxigraph: SPARQL graph database \- GitHub, accessed March 17, 2026, [https://github.com/oxigraph/oxigraph](https://github.com/oxigraph/oxigraph)  
2. rudof: A Rust Library for handling RDF data models and Shapes \- CEUR-WS.org, accessed March 17, 2026, [https://ceur-ws.org/Vol-3828/paper32.pdf](https://ceur-ws.org/Vol-3828/paper32.pdf)  
3. Architecture · oxigraph/oxigraph Wiki \- GitHub, accessed March 17, 2026, [https://github.com/oxigraph/oxigraph/wiki/Architecture](https://github.com/oxigraph/oxigraph/wiki/Architecture)  
4. rudof-project/rudof: RDF data shapes implementation in Rust \- GitHub, accessed March 17, 2026, [https://github.com/rudof-project/rudof](https://github.com/rudof-project/rudof)  
5. Paul Muraviev / awesome-rust \- GitLab, accessed March 17, 2026, [https://gitlab.com/tecras/awesome-rust](https://gitlab.com/tecras/awesome-rust)  
6. Ordered key–value store \- Wikipedia, accessed March 17, 2026, [https://en.wikipedia.org/wiki/Ordered\_key%E2%80%93value\_store](https://en.wikipedia.org/wiki/Ordered_key%E2%80%93value_store)  
7. Storage \- TiKV, accessed March 17, 2026, [https://tikv.org/docs/5.1/reference/architecture/storage/](https://tikv.org/docs/5.1/reference/architecture/storage/)  
8. Store in oxigraph::store \- Rust \- Docs.rs, accessed March 17, 2026, [https://docs.rs/oxigraph/latest/oxigraph/store/struct.Store.html](https://docs.rs/oxigraph/latest/oxigraph/store/struct.Store.html)  
9. ACID Compliant Distributed Key-Value Store, accessed March 17, 2026, [http://www.scs.stanford.edu/20sp-cs244b/projects/ACID%20Compliant%20Distributed%20Key-Value%20Store.pdf](http://www.scs.stanford.edu/20sp-cs244b/projects/ACID%20Compliant%20Distributed%20Key-Value%20Store.pdf)  
10. ACID Transactions: The Cornerstone of Database Integrity | Yugabyte, accessed March 17, 2026, [https://www.yugabyte.com/key-concepts/acid-transactions/](https://www.yugabyte.com/key-concepts/acid-transactions/)  
11. shacl\_validation \- crates.io: Rust Package Registry, accessed March 17, 2026, [https://crates.io/crates/shacl\_validation/0.1.12](https://crates.io/crates/shacl_validation/0.1.12)  
12. rudof: A Rust Library for handling RDF data models and Shapes \- Jose Emilio Labra Gayo, accessed March 17, 2026, [https://labra.weso.es/pdf/2024\_rudof\_demo.pdf](https://labra.weso.es/pdf/2024_rudof_demo.pdf)  
13. Refactoring rudof · rudof-project rudof · Discussion \#212 · GitHub, accessed March 17, 2026, [https://github.com/rudof-project/rudof/discussions/212](https://github.com/rudof-project/rudof/discussions/212)  
14. Oxigraph — Rust database // Lib.rs, accessed March 17, 2026, [https://lib.rs/crates/oxigraph](https://lib.rs/crates/oxigraph)  
15. oxigraph/CHANGELOG.md at main \- GitHub, accessed March 17, 2026, [https://github.com/oxigraph/oxigraph/blob/main/CHANGELOG.md](https://github.com/oxigraph/oxigraph/blob/main/CHANGELOG.md)  
16. Rust for Big Data: How We Built a Cloud-Native MPP Query Executor on S3 from Scratch, accessed March 17, 2026, [https://www.databend.com/blog/engineering/rust-for-big-data-how-we-built-a-cloud-native-mpp-query-executor-on-s3-from-scratch/](https://www.databend.com/blog/engineering/rust-for-big-data-how-we-built-a-cloud-native-mpp-query-executor-on-s3-from-scratch/)  
17. Using Machine Learning and Routing Protocols for Optimizing Distributed SPARQL Queries in Collaboration, accessed March 17, 2026, [https://www.itm.uni-luebeck.de/fileadmin/files/publications/computers-12-00210.pdf](https://www.itm.uni-luebeck.de/fileadmin/files/publications/computers-12-00210.pdf)  
18. PRoST: Distributed Execution of SPARQL Queries Using Mixed Partitioning Strategies \- OpenProceedings.org, accessed March 17, 2026, [https://openproceedings.org/2018/conf/edbt/paper-288.pdf](https://openproceedings.org/2018/conf/edbt/paper-288.pdf)  
19. A Survey and Experimental Comparison of Distributed SPARQL Engines for Very Large RDF Data \- VLDB Endowment, accessed March 17, 2026, [https://www.vldb.org/pvldb/vol10/p2049-abdelaziz.pdf](https://www.vldb.org/pvldb/vol10/p2049-abdelaziz.pdf)  
20. Scalable Linked Data Stream Processing via Network-Aware Workload Scheduling \- CEUR-WS.org, accessed March 17, 2026, [https://ceur-ws.org/Vol-1046/SSWS2013\_paper6.pdf](https://ceur-ws.org/Vol-1046/SSWS2013_paper6.pdf)  
21. 5 Query Pushdowns for Distributed SQL and How They Differ from a Traditional RDBMS, accessed March 17, 2026, [https://www.yugabyte.com/blog/5-query-pushdowns-for-distributed-sql-and-how-they-differ-from-a-traditional-rdbms/](https://www.yugabyte.com/blog/5-query-pushdowns-for-distributed-sql-and-how-they-differ-from-a-traditional-rdbms/)  
22. FOSDEM 2018 \- TiKV \- building a distributed key-value store with Rust, accessed March 17, 2026, [https://archive.fosdem.org/2018/schedule/event/rust\_distributed\_kv\_store/](https://archive.fosdem.org/2018/schedule/event/rust_distributed_kv_store/)  
23. TiKV Performance Tuning with Massive Regions, accessed March 17, 2026, [https://tikv.org/blog/tune-with-massive-regions-in-tikv/](https://tikv.org/blog/tune-with-massive-regions-in-tikv/)  
24. FAQs \- TiKV, accessed March 17, 2026, [https://tikv.org/docs/5.1/reference/faq/](https://tikv.org/docs/5.1/reference/faq/)  
25. Rust Client \- TiKV, accessed March 17, 2026, [https://tikv.org/docs/7.1/develop/clients/rust/](https://tikv.org/docs/7.1/develop/clients/rust/)  
26. Databases | Skiddle Data Collection, accessed March 17, 2026, [https://wiki.skiddle.id/databases/](https://wiki.skiddle.id/databases/)  
27. ratoru/distrib-kv-store: Distributed key-value store with sharding and fault tolerance. \- GitHub, accessed March 17, 2026, [https://github.com/ratoru/distrib-kv-store](https://github.com/ratoru/distrib-kv-store)  
28. Data Sharding \- TiKV, accessed March 17, 2026, [https://tikv.org/deep-dive/scalability/data-sharding/](https://tikv.org/deep-dive/scalability/data-sharding/)  
29. Coprocessor \- TiKV Development Guide, accessed March 17, 2026, [https://tikv.github.io/tikv-dev-guide/understanding-tikv/coprocessor/intro.html](https://tikv.github.io/tikv-dev-guide/understanding-tikv/coprocessor/intro.html)  
30. Designing Distributed Storage Architectures with TiKV | by firman brilian \- Medium, accessed March 17, 2026, [https://medium.com/@firmanbrilian/designing-distributed-storage-architectures-with-tikv-403819dc2f6c](https://medium.com/@firmanbrilian/designing-distributed-storage-architectures-with-tikv-403819dc2f6c)  
31. Performance Overview \- TiKV, accessed March 17, 2026, [https://tikv.org/docs/6.1/deploy/performance/overview/](https://tikv.org/docs/6.1/deploy/performance/overview/)  
32. tikv::coprocessor \- Rust, accessed March 17, 2026, [https://tikv.github.io/doc/tikv/coprocessor/index.html](https://tikv.github.io/doc/tikv/coprocessor/index.html)  
33. Coprocessor Config \- TiKV, accessed March 17, 2026, [https://tikv.org/docs/6.5/deploy/configure/coprocessor/](https://tikv.org/docs/6.5/deploy/configure/coprocessor/)  
34. LFX: Coprocessor Plugin · Issue \#9747 · tikv/tikv \- GitHub, accessed March 17, 2026, [https://github.com/tikv/tikv/issues/9747](https://github.com/tikv/tikv/issues/9747)  
35. Distributed SQL \- TiKV, accessed March 17, 2026, [https://tikv.org/deep-dive/distributed-sql/dist-sql/](https://tikv.org/deep-dive/distributed-sql/dist-sql/)  
36. Deep Dive TiKV, accessed March 17, 2026, [https://tikv.github.io/deep-dive-tikv/print.html](https://tikv.github.io/deep-dive-tikv/print.html)  
37. A Deep Dive into TiKV | by TiDB \- Medium, accessed March 17, 2026, [https://pingcap.medium.com/a-deep-dive-into-tikv-b27989993d19](https://pingcap.medium.com/a-deep-dive-into-tikv-b27989993d19)  
38. TiGraph: 8,700x Computing Performance Achieved by Combining Graphs \+ the RDBMS Syntax | PingCAP株式会社, accessed March 17, 2026, [https://pingcap.co.jp/blog/tigraph-8700x-computing-performance-achieved-by-combining-graphs-rdbms-syntax/](https://pingcap.co.jp/blog/tigraph-8700x-computing-performance-achieved-by-combining-graphs-rdbms-syntax/)  
39. TiGraph: 8,700x Computing Performance Achieved by Combining, accessed March 17, 2026, [https://pingcap.medium.com/tigraph-8-700x-computing-performance-achieved-by-combining-graphs-the-rdbms-syntax-2d4ac59644b5?responsesOpen=true\&source=follow\_footer-----a35ca70c4b6e----1-------------------------------](https://pingcap.medium.com/tigraph-8-700x-computing-performance-achieved-by-combining-graphs-the-rdbms-syntax-2d4ac59644b5?responsesOpen=true&source=follow_footer-----a35ca70c4b6e----1-------------------------------)  
40. Coprocessor Cache \- TiDB Docs \- PingCAP, accessed March 17, 2026, [https://docs.pingcap.com/tidb/stable/coprocessor-cache/](https://docs.pingcap.com/tidb/stable/coprocessor-cache/)  
41. Key Monitoring Metrics of TiKV \- TiDB Docs, accessed March 17, 2026, [https://docs.pingcap.com/tidb/stable/grafana-tikv-dashboard/](https://docs.pingcap.com/tidb/stable/grafana-tikv-dashboard/)  
42. Best Practices for Tuning TiKV Performance with Massive Regions \- TiDB Docs, accessed March 17, 2026, [https://docs.pingcap.com/best-practices/massive-regions-best-practices/](https://docs.pingcap.com/best-practices/massive-regions-best-practices/)  
43. FoundationDB Record Layer \- Hacker News, accessed March 17, 2026, [https://news.ycombinator.com/item?id=18906341](https://news.ycombinator.com/item?id=18906341)  
44. Database in foundationdb \- Rust \- Docs.rs, accessed March 17, 2026, [https://docs.rs/foundationdb/latest/foundationdb/struct.Database.html](https://docs.rs/foundationdb/latest/foundationdb/struct.Database.html)  
45. Ensuring Safety in FoundationDB's Rust Crate | Pierre Zemb's Blog, accessed March 17, 2026, [https://pierrezemb.fr/posts/providing-safety-fdb-rs/](https://pierrezemb.fr/posts/providing-safety-fdb-rs/)  
46. How difficult would it be to implement the wire protocol in other languages?, accessed March 17, 2026, [https://forums.foundationdb.org/t/how-difficult-would-it-be-to-implement-the-wire-protocol-in-other-languages/69](https://forums.foundationdb.org/t/how-difficult-would-it-be-to-implement-the-wire-protocol-in-other-languages/69)  
47. FoundationDB/fdb-record-layer: A relational database with SQL support built on ... \- GitHub, accessed March 17, 2026, [https://github.com/FoundationDB/fdb-record-layer](https://github.com/FoundationDB/fdb-record-layer)  
48. For Anyone with experience with FoundationDB\! How does it compare to CockroachDB... | Hacker News, accessed March 17, 2026, [https://news.ycombinator.com/item?id=30644918](https://news.ycombinator.com/item?id=30644918)  
49. Rust FDB Record Layer Work-in-progress Repository \- FoundationDB forums, accessed March 17, 2026, [https://forums.foundationdb.org/t/rust-fdb-record-layer-work-in-progress-repository/3765](https://forums.foundationdb.org/t/rust-fdb-record-layer-work-in-progress-repository/3765)  
50. RFD 53 Control plane data storage requirements \- Oxide RFD, accessed March 17, 2026, [https://rfd.shared.oxide.computer/rfd/53](https://rfd.shared.oxide.computer/rfd/53)  
51. FoundationDB read performance, accessed March 17, 2026, [https://forums.foundationdb.org/t/foundationdb-read-performance/729](https://forums.foundationdb.org/t/foundationdb-read-performance/729)  
52. Latency of range queries that return large number of key-value pairs \- FoundationDB forums, accessed March 17, 2026, [https://forums.foundationdb.org/t/latency-of-range-queries-that-return-large-number-of-key-value-pairs/1291](https://forums.foundationdb.org/t/latency-of-range-queries-that-return-large-number-of-key-value-pairs/1291)  
53. FoundationDB has, in my experience, always been well regarded in DB development ... \- Hacker News, accessed March 17, 2026, [https://news.ycombinator.com/item?id=36575333](https://news.ycombinator.com/item?id=36575333)  
54. To anyone who is on the fence about putting FoundationDB into production (or at ... \- Hacker News, accessed March 17, 2026, [https://news.ycombinator.com/item?id=18489410](https://news.ycombinator.com/item?id=18489410)  
55. FoundationDB: A Distributed Key-Value Store \- Hacker News, accessed March 17, 2026, [https://news.ycombinator.com/item?id=36572658](https://news.ycombinator.com/item?id=36572658)  
56. Record Layer Design Questions \- FoundationDB forums, accessed March 17, 2026, [https://forums.foundationdb.org/t/record-layer-design-questions/3468](https://forums.foundationdb.org/t/record-layer-design-questions/3468)  
57. Distributed transaction with pushdown predicates \- FoundationDB forums, accessed March 17, 2026, [https://forums.foundationdb.org/t/distributed-transaction-with-pushdown-predicates/459](https://forums.foundationdb.org/t/distributed-transaction-with-pushdown-predicates/459)  
58. Best practices for designing and architecting with DynamoDB \- AWS Documentation, accessed March 17, 2026, [https://docs.aws.amazon.com/amazondynamodb/latest/developerguide/best-practices.html](https://docs.aws.amazon.com/amazondynamodb/latest/developerguide/best-practices.html)  
59. Reduce latency and cost in read-heavy applications using Amazon DynamoDB Accelerator, accessed March 17, 2026, [https://aws.amazon.com/blogs/database/reduce-latency-and-cost-in-read-heavy-applications-using-amazon-dynamodb-accelerator/](https://aws.amazon.com/blogs/database/reduce-latency-and-cost-in-read-heavy-applications-using-amazon-dynamodb-accelerator/)  
60. Effective data sorting with Amazon DynamoDB | AWS Database Blog, accessed March 17, 2026, [https://aws.amazon.com/blogs/database/effective-data-sorting-with-amazon-dynamodb/](https://aws.amazon.com/blogs/database/effective-data-sorting-with-amazon-dynamodb/)  
61. Using write sharding to distribute workloads evenly in your DynamoDB table, accessed March 17, 2026, [https://docs.aws.amazon.com/amazondynamodb/latest/developerguide/bp-partition-key-sharding.html](https://docs.aws.amazon.com/amazondynamodb/latest/developerguide/bp-partition-key-sharding.html)  
62. DynamoDB only useful if partition key value known in advance? \- Reddit, accessed March 17, 2026, [https://www.reddit.com/r/aws/comments/1e86hh3/dynamodb\_only\_useful\_if\_partition\_key\_value\_known/](https://www.reddit.com/r/aws/comments/1e86hh3/dynamodb_only_useful_if_partition_key_value_known/)  
63. We built an open-source, S3-native SQL query executor in Rust. Here's a deep dive into our async architecture. \- Reddit, accessed March 17, 2026, [https://www.reddit.com/r/rust/comments/1n9ctc3/we\_built\_an\_opensource\_s3native\_sql\_query/](https://www.reddit.com/r/rust/comments/1n9ctc3/we_built_an_opensource_s3native_sql_query/)  
64. FOSS, cloud native, log storage and query engine build with Apache Arrow & Parquet, written in Rust and React. \- Reddit, accessed March 17, 2026, [https://www.reddit.com/r/rust/comments/xt1c5w/foss\_cloud\_native\_log\_storage\_and\_query\_engine/](https://www.reddit.com/r/rust/comments/xt1c5w/foss_cloud_native_log_storage_and_query_engine/)  
65. rust-unofficial/awesome-rust: A curated list of Rust code and resources. \- GitHub, accessed March 17, 2026, [https://github.com/rust-unofficial/awesome-rust](https://github.com/rust-unofficial/awesome-rust)  
66. Optimizing SPARQL Queries over Disparate RDF Data Sources through Distributed Semi-joins \- CEUR-WS.org, accessed March 17, 2026, [https://ceur-ws.org/Vol-401/iswc2008pd\_submission\_69.pdf](https://ceur-ws.org/Vol-401/iswc2008pd_submission_69.pdf)  
67. S2RDF: RDF Querying with SPARQL on Spark \- VLDB Endowment, accessed March 17, 2026, [http://www.vldb.org/pvldb/vol9/p804-schaetzle.pdf](http://www.vldb.org/pvldb/vol9/p804-schaetzle.pdf)  
68. Should Store and Storage be a Trait instead of an enum · oxigraph ..., accessed March 17, 2026, [https://github.com/oxigraph/oxigraph/discussions/1487](https://github.com/oxigraph/oxigraph/discussions/1487)  
69. Resource Backends \- Confidential Containers, accessed March 17, 2026, [https://confidentialcontainers.org/docs/attestation/resources/resource-backends/](https://confidentialcontainers.org/docs/attestation/resources/resource-backends/)  
70. agentdb \- Rust \- Docs.rs, accessed March 17, 2026, [https://docs.rs/agentdb](https://docs.rs/agentdb)

[image1]: <data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAABMAAAAXCAYAAADpwXTaAAAAbElEQVR4XmNgGAWjgKogAF2AErABiAXRBckFLkBcgS5ICegBYit0QXIBMxCvBOJKIGZFllgIxLvJwBeA+B0QJzJQCESBeD0Qi6FLkAqYgHgrEEuiS5ADgoE4Gl2QXADyHkqgUwL00AVGwSAAAG69EzceZiPbAAAAAElFTkSuQmCC>

[image2]: <data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAABMAAAAYCAYAAAAYl8YPAAABCklEQVR4XmNgGAWDAoQA8X8g/gvEN4B4NxB/hor9BuITQHwQKg8Ss4Voww62AHEtEPMjie1igGhURRLTA+JvQCyCJIYC+IB4O5oYBwNE0200cRC4jC6ADBKBOApNzJUB4qqpaOKcDJAgwAlA3mBHE2tlgBgGCktkwAbEmmhiBMFRBohhAugSpAJeBkgMnkKXQAKCQJyELogN+DJAXNWOLgEFLUC8FojvoUtgAxMZIIa5oEsgAQcGIg07D8TfGSDJAxdwYCDCMAUGiKvwRj8DHsNEGSCaQfgpA8QwEAYpBonJIpTCgQMQ30cXJBc4APEDNDGygQMQP0IXJAfkAPFSBkjebWCAGDwKyAQAqtM1AqGjiR0AAAAASUVORK5CYII=>

[image3]: <data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAABMAAAAYCAYAAAAYl8YPAAABLklEQVR4Xu2UMUuCURSGXwgqhyzQaAocbEjIXciIqFFwcGqLlqa2FsExag1qq7/QUhDo5BBES0MNgRAt/YCIbAix93CueTxl8jk1+MADn+977+X63csHjPgXlGibtugjrdG3kH3SG1oPvWR5nfY7l7RCp01WhU5cMFmWNmnSZD3E6ZXLJqGTGi4X7n1g2aKbLtuA7urE5THoK+iL/I0Jl+1DF5N3aRmniy4byDV0sRlfRGUKeoK3vggU6Tl9oHuu+0EBuqsDX5A0vaBjNEGf6bYd4DmCLrbuC7JMX+lc+H0KvUJ9uaMf0OsxCDnZMx92SEF39efxB+QCv9MlG85CJ4sv0MXEp5DNd4d+I4ck3YovhuGYZsLzmi2iskt36Grw0JZRyKH75ehY7hkxIhJfjqs7SFhZbPkAAAAASUVORK5CYII=>

[image4]: <data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAA8AAAAXCAYAAADUUxW8AAAArElEQVR4XmNgGAU0AUFA/ApdkBggDMR3gPg/EHOiyREEBUC8gAGiWRVVCj/wZ4Bo6GKAaLZHlcYNBIE4BMouZoBojkBI4weZQMwIZccxQDQXIqRxA1cg1kHiezJANIOcjxdMAWJxNDE5Bojm3WjiKAAUQKDQRQfcDBDNV9ElYAAUqreBmAtdAgq+AvFndEEzID4DxH8ZEKazIck3APEhqBwIHwPiUiT5UTD4AQDQdx61ug1UBwAAAABJRU5ErkJggg==>

[image5]: <data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAABAAAAAYCAYAAADzoH0MAAAA+UlEQVR4XmNgGAVUB2ZAvBuInwHxfyC+BuXD8CMobQXTgAssZoAYIIkmrgzE34H4HRALo8mhgPtAfApdEApOMkAMN0WXgAEdBoiCbnQJIDBmgMhdB2JmNDk4KGCAKHJCEmMD4lQgfgXEB4FYDUkOA2xlgBiwDoj7gLgWiOcD8XsgbkRShxVwMUACaQO6BBA4APFvIF6EJo4C3BggtoO8gQ2cYIDIa6BLwEAnA0SBProEFDxngMiroEvAwDEGiCJsIIYBohmUkLACKQaIgqVo4qxAnA/E3xggaQCkDgUYMUD89osBYsATBtTkewuIdwFxJgOeuB8FQxoAAB5CNyVzL5aRAAAAAElFTkSuQmCC>

[image6]: <data:image/png;base64,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>