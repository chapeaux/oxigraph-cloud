//! SHACL validation integration for Oxigraph via the rudof crate.
//!
//! This crate bridges Oxigraph's [`Store`](oxigraph::store::Store) to rudof's
//! SHACL validation engine, enabling validation of RDF data against SHACL
//! shapes graphs.
//!
//! # Architecture
//!
//! The validation flow is:
//!
//! 1. **Load shapes** -- parse SHACL shapes from Turtle (or other RDF format)
//!    and compile them into a [`CompiledShapes`](shapes::CompiledShapes).
//! 2. **Configure validator** -- create a [`ShaclValidator`](validator::ShaclValidator)
//!    with a [`ShaclMode`](validator::ShaclMode) (Off, Warn, or Enforce).
//! 3. **Validate** -- call [`validate()`](validator::ShaclValidator::validate) with
//!    an Oxigraph `Store` reference. Internally, the store data is serialized
//!    and loaded into rudof's `RdfData`, then validated against the compiled shapes.
//! 4. **Inspect result** -- the returned [`ValidationOutcome`](validator::ValidationOutcome)
//!    is `Skipped`, `Passed`, or `Failed(ValidationReport)`.
//!
//! # Example
//!
//! ```no_run
//! use oxigraph_shacl::shapes::CompiledShapes;
//! use oxigraph_shacl::validator::{ShaclMode, ShaclValidator, ValidationOutcome};
//! use oxigraph::store::Store;
//!
//! let shapes = CompiledShapes::from_turtle(r#"
//!     @prefix sh: <http://www.w3.org/ns/shacl#> .
//!     @prefix ex: <http://example.org/> .
//!     ex:PersonShape a sh:NodeShape ;
//!         sh:targetClass ex:Person ;
//!         sh:property [ sh:path ex:name ; sh:minCount 1 ] .
//! "#).unwrap();
//!
//! let mut validator = ShaclValidator::new(ShaclMode::Enforce);
//! validator.set_shapes(shapes);
//!
//! let store = Store::new().unwrap();
//! match validator.validate(&store).unwrap() {
//!     ValidationOutcome::Passed => println!("All shapes conform"),
//!     ValidationOutcome::Failed(report) => println!("Violations: {report}"),
//!     ValidationOutcome::Skipped => println!("Validation disabled"),
//! }
//! ```

pub mod error;
pub mod report;
pub mod shapes;
pub mod validator;
