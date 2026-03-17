# References

## Primary Projects
1. Oxigraph: https://github.com/oxigraph/oxigraph
2. Rudof paper: https://ceur-ws.org/Vol-3828/paper32.pdf
3. Oxigraph Architecture: https://github.com/oxigraph/oxigraph/wiki/Architecture
4. Rudof repo: https://github.com/rudof-project/rudof

## Oxigraph
5. Oxigraph Store API: https://docs.rs/oxigraph/latest/oxigraph/store/struct.Store.html
6. Oxigraph crate (Lib.rs): https://lib.rs/crates/oxigraph
7. Oxigraph CHANGELOG: https://github.com/oxigraph/oxigraph/blob/main/CHANGELOG.md
8. StorageBackend trait discussion: https://github.com/oxigraph/oxigraph/discussions/1487

## Rudof
9. Rudof SRDF demo: https://labra.weso.es/pdf/2024_rudof_demo.pdf
10. Rudof refactoring discussion: https://github.com/rudof-project/rudof/discussions/212
11. shacl_validation crate: https://crates.io/crates/shacl_validation/0.1.12

## TiKV
12. TiKV storage architecture: https://tikv.org/docs/5.1/reference/architecture/storage/
13. TiKV data sharding: https://tikv.org/deep-dive/scalability/data-sharding/
14. TiKV Coprocessor guide: https://tikv.github.io/tikv-dev-guide/understanding-tikv/coprocessor/intro.html
15. TiKV Distributed SQL pushdown: https://tikv.org/deep-dive/distributed-sql/dist-sql/
16. TiKV Region tuning: https://tikv.org/blog/tune-with-massive-regions-in-tikv/
17. TiKV Rust client: https://tikv.org/docs/7.1/develop/clients/rust/
18. TiKV performance: https://tikv.org/docs/6.1/deploy/performance/overview/
19. TiKV FAQs: https://tikv.org/docs/5.1/reference/faq/
20. TiKV Coprocessor config: https://tikv.org/docs/6.5/deploy/configure/coprocessor/
21. TiKV deep dive: https://tikv.github.io/deep-dive-tikv/print.html
22. TiGraph (8700x improvement): https://pingcap.co.jp/blog/tigraph-8700x-computing-performance-achieved-by-combining-graphs-rdbms-syntax/
23. TiKV Coprocessor cache: https://docs.pingcap.com/tidb/stable/coprocessor-cache/
24. TiKV monitoring: https://docs.pingcap.com/tidb/stable/grafana-tikv-dashboard/
25. TiKV massive regions best practices: https://docs.pingcap.com/best-practices/massive-regions-best-practices/
26. TiKV at FOSDEM: https://archive.fosdem.org/2018/schedule/event/rust_distributed_kv_store/

## FoundationDB
27. FoundationDB Rust bindings: https://docs.rs/foundationdb/latest/foundationdb/struct.Database.html
28. FDB safety in Rust: https://pierrezemb.fr/posts/providing-safety-fdb-rs/
29. FDB Record Layer: https://github.com/FoundationDB/fdb-record-layer
30. FDB read performance: https://forums.foundationdb.org/t/foundationdb-read-performance/729
31. FDB range query latency: https://forums.foundationdb.org/t/latency-of-range-queries-that-return-large-number-of-key-value-pairs/1291
32. FDB pushdown discussion: https://forums.foundationdb.org/t/distributed-transaction-with-pushdown-predicates/459

## DynamoDB
33. DynamoDB best practices: https://docs.aws.amazon.com/amazondynamodb/latest/developerguide/best-practices.html
34. DynamoDB DAX: https://aws.amazon.com/blogs/database/reduce-latency-and-cost-in-read-heavy-applications-using-amazon-dynamodb-accelerator/

## Distributed SPARQL Theory
35. Distributed SPARQL survey (VLDB): https://www.vldb.org/pvldb/vol10/p2049-abdelaziz.pdf
36. S2RDF / ExtVP (VLDB): http://www.vldb.org/pvldb/vol9/p804-schaetzle.pdf
37. Distributed semi-joins: https://ceur-ws.org/Vol-401/iswc2008pd_submission_69.pdf
38. PRoST mixed partitioning: https://openproceedings.org/2018/conf/edbt/paper-288.pdf
39. Network-aware scheduling: https://ceur-ws.org/Vol-1046/SSWS2013_paper6.pdf
40. ML routing for SPARQL: https://www.itm.uni-luebeck.de/fileadmin/files/publications/computers-12-00210.pdf

## Cloud-Native Storage
41. Databend (Rust S3-native): https://www.databend.com/blog/engineering/rust-for-big-data-how-we-built-a-cloud-native-mpp-query-executor-on-s3-from-scratch/
42. Query pushdowns: https://www.yugabyte.com/blog/5-query-pushdowns-for-distributed-sql-and-how-they-differ-from-a-traditional-rdbms/

## Design Patterns
43. agentdb (Rust backend traits): https://docs.rs/agentdb
44. Confidential Containers backends: https://confidentialcontainers.org/docs/attestation/resources/resource-backends/
45. Ordered KV stores: https://en.wikipedia.org/wiki/Ordered_key%E2%80%93value_store
46. ACID KV store (Stanford): http://www.scs.stanford.edu/20sp-cs244b/projects/ACID%20Compliant%20Distributed%20Key-Value%20Store.pdf
