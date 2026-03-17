//! SHACL validator for Oxigraph stores.
//!
//! Provides [`ShaclValidator`] which validates an Oxigraph [`Store`] against
//! compiled SHACL shapes using the rudof validation engine.

use crate::error::ShaclError;
use crate::shapes::CompiledShapes;
use oxigraph::store::Store;
use rudof_rdf::rdf_core::RDFFormat;
use rudof_rdf::rdf_impl::ReaderMode;
use shacl_validation::shacl_processor::{RdfDataValidation, ShaclProcessor, ShaclValidationMode};
use shacl_validation::validation_report::report::ValidationReport;
use sparql_service::RdfData;
use std::fmt;

/// Validation mode for SHACL enforcement.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ShaclMode {
    /// No validation (default, backward compatible).
    #[default]
    Off,
    /// Log validation failures but accept data.
    Warn,
    /// Reject data that fails validation.
    Enforce,
}

/// The result of a SHACL validation run.
#[derive(Clone, Debug)]
pub enum ValidationOutcome {
    /// Validation was skipped because mode is [`ShaclMode::Off`].
    Skipped,
    /// All shapes conform.
    Passed,
    /// One or more shapes did not conform.
    Failed(ValidationReport),
}

impl ValidationOutcome {
    /// Returns `true` if the outcome is [`ValidationOutcome::Passed`].
    pub fn is_passed(&self) -> bool {
        matches!(self, Self::Passed)
    }

    /// Returns `true` if the outcome is [`ValidationOutcome::Failed`].
    pub fn is_failed(&self) -> bool {
        matches!(self, Self::Failed(_))
    }

    /// Returns `true` if the outcome is [`ValidationOutcome::Skipped`].
    pub fn is_skipped(&self) -> bool {
        matches!(self, Self::Skipped)
    }
}

impl fmt::Display for ValidationOutcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Skipped => write!(f, "Validation skipped (mode=Off)"),
            Self::Passed => write!(f, "Validation passed: all shapes conform"),
            Self::Failed(report) => write!(f, "Validation failed: {report}"),
        }
    }
}

/// SHACL validator that validates Oxigraph store data against compiled shapes.
///
/// # Example
///
/// ```no_run
/// use oxigraph_shacl::validator::{ShaclMode, ShaclValidator};
/// use oxigraph_shacl::shapes::CompiledShapes;
/// use oxigraph::store::Store;
///
/// let shapes = CompiledShapes::from_turtle(r#"
///     @prefix sh: <http://www.w3.org/ns/shacl#> .
///     @prefix ex: <http://example.org/> .
///     @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
///
///     ex:PersonShape a sh:NodeShape ;
///         sh:targetClass ex:Person ;
///         sh:property [
///             sh:path ex:name ;
///             sh:datatype xsd:string ;
///             sh:minCount 1 ;
///         ] .
/// "#).unwrap();
///
/// let mut validator = ShaclValidator::new(ShaclMode::Enforce);
/// validator.set_shapes(shapes);
///
/// let store = Store::new().unwrap();
/// let outcome = validator.validate(&store).unwrap();
/// assert!(outcome.is_passed() || outcome.is_failed());
/// ```
pub struct ShaclValidator {
    mode: ShaclMode,
    shapes: Option<CompiledShapes>,
}

impl ShaclValidator {
    /// Creates a new validator with the given mode and no shapes loaded.
    pub fn new(mode: ShaclMode) -> Self {
        Self { mode, shapes: None }
    }

    /// Convenience: compiles shapes from Turtle and returns a [`CompiledShapes`].
    pub fn load_shapes_from_turtle(turtle: &str) -> Result<CompiledShapes, ShaclError> {
        CompiledShapes::from_turtle(turtle)
    }

    /// Sets the compiled shapes to validate against.
    pub fn set_shapes(&mut self, shapes: CompiledShapes) {
        self.shapes = Some(shapes);
    }

    /// Returns the current validation mode.
    pub fn mode(&self) -> ShaclMode {
        self.mode
    }

    /// Sets the validation mode.
    pub fn set_mode(&mut self, mode: ShaclMode) {
        self.mode = mode;
    }

    /// Returns a reference to the currently loaded shapes, if any.
    pub fn shapes(&self) -> Option<&CompiledShapes> {
        self.shapes.as_ref()
    }

    /// Validates the data in the given Oxigraph [`Store`] against the loaded shapes.
    ///
    /// The process:
    /// 1. If mode is [`ShaclMode::Off`], returns [`ValidationOutcome::Skipped`].
    /// 2. Serializes all quads from the store to NTriples.
    /// 3. Loads the serialized data into rudof's [`RdfData`].
    /// 4. Runs the SHACL validation engine against the compiled shapes.
    /// 5. Returns [`ValidationOutcome::Passed`] or [`ValidationOutcome::Failed`].
    ///
    /// Returns [`ShaclError`] if no shapes are loaded or if serialization/validation fails.
    pub fn validate(&self, store: &Store) -> Result<ValidationOutcome, ShaclError> {
        if self.mode == ShaclMode::Off {
            return Ok(ValidationOutcome::Skipped);
        }

        let shapes = self
            .shapes
            .as_ref()
            .ok_or_else(|| ShaclError::ValidationEngine("No shapes loaded".to_string()))?;

        // Serialize quads from the oxigraph store to NTriples format
        let ntriples = serialize_store_to_ntriples(store)?;

        // Load into rudof's RdfData
        let rdf_data = RdfData::from_str(
            &ntriples,
            &RDFFormat::NTriples,
            None,
            &ReaderMode::default(),
        )
        .map_err(|e| ShaclError::DataLoading(e.to_string()))?;

        // Run validation
        let mut processor =
            RdfDataValidation::from_rdf_data(rdf_data, ShaclValidationMode::Native);
        let report = processor
            .validate(shapes.schema())
            .map_err(|e| ShaclError::ValidationEngine(e.to_string()))?;

        if report.conforms() {
            Ok(ValidationOutcome::Passed)
        } else {
            Ok(ValidationOutcome::Failed(report))
        }
    }
}

/// Serializes all quads in the store as NTriples (ignoring graph names).
fn serialize_store_to_ntriples(store: &Store) -> Result<String, ShaclError> {
    let mut output = String::new();
    for quad_result in store.iter() {
        let quad = quad_result
            .map_err(|e| ShaclError::StoreSerialization(e.to_string()))?;
        // NTriples format: <subject> <predicate> <object> .
        // We use the triple (ignoring the graph name) for default-graph validation
        let triple = quad.as_ref().into();
        output.push_str(&format_ntriples_triple(&triple));
        output.push('\n');
    }
    Ok(output)
}

/// Formats a single triple as an NTriples line.
fn format_ntriples_triple(triple: &oxrdf::TripleRef<'_>) -> String {
    format!("{} {} {} .", triple.subject, triple.predicate, triple.object)
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxigraph::io::RdfFormat;
    use oxigraph::store::Store;

    const PERSON_SHAPES: &str = r#"
        @prefix sh: <http://www.w3.org/ns/shacl#> .
        @prefix ex: <http://example.org/> .
        @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .

        ex:PersonShape a sh:NodeShape ;
            sh:targetClass ex:Person ;
            sh:property [
                sh:path ex:name ;
                sh:datatype xsd:string ;
                sh:minCount 1 ;
                sh:maxCount 1 ;
            ] .
    "#;

    const CONFORMANT_DATA: &str = r#"
        @prefix ex: <http://example.org/> .
        @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .

        ex:alice a ex:Person ;
            ex:name "Alice"^^xsd:string .
    "#;

    const NON_CONFORMANT_DATA: &str = r#"
        @prefix ex: <http://example.org/> .

        ex:bob a ex:Person .
    "#;

    #[test]
    fn test_mode_off_skips_validation() {
        let validator = ShaclValidator::new(ShaclMode::Off);
        let store = Store::new().unwrap();
        let outcome = validator.validate(&store).unwrap();
        assert!(outcome.is_skipped());
    }

    #[test]
    fn test_no_shapes_returns_error() {
        let validator = ShaclValidator::new(ShaclMode::Enforce);
        let store = Store::new().unwrap();
        let result = validator.validate(&store);
        assert!(result.is_err());
    }

    #[test]
    fn test_compile_shapes() {
        let shapes = CompiledShapes::from_turtle(PERSON_SHAPES).unwrap();
        assert_eq!(shapes.target_shape_count(), 1);
    }

    #[test]
    fn test_conformant_data_passes() {
        let shapes = CompiledShapes::from_turtle(PERSON_SHAPES).unwrap();
        let mut validator = ShaclValidator::new(ShaclMode::Enforce);
        validator.set_shapes(shapes);

        let store = Store::new().unwrap();
        store
            .load_from_reader(RdfFormat::Turtle, CONFORMANT_DATA.as_bytes())
            .unwrap();

        let outcome = validator.validate(&store).unwrap();
        assert!(
            outcome.is_passed(),
            "Expected Passed but got: {outcome}"
        );
    }

    #[test]
    fn test_non_conformant_data_fails() {
        let shapes = CompiledShapes::from_turtle(PERSON_SHAPES).unwrap();
        let mut validator = ShaclValidator::new(ShaclMode::Enforce);
        validator.set_shapes(shapes);

        let store = Store::new().unwrap();
        store
            .load_from_reader(RdfFormat::Turtle, NON_CONFORMANT_DATA.as_bytes())
            .unwrap();

        let outcome = validator.validate(&store).unwrap();
        assert!(
            outcome.is_failed(),
            "Expected Failed but got: {outcome}"
        );
    }

    #[test]
    fn test_empty_store_passes() {
        let shapes = CompiledShapes::from_turtle(PERSON_SHAPES).unwrap();
        let mut validator = ShaclValidator::new(ShaclMode::Enforce);
        validator.set_shapes(shapes);

        let store = Store::new().unwrap();
        let outcome = validator.validate(&store).unwrap();
        assert!(
            outcome.is_passed(),
            "Expected Passed for empty store but got: {outcome}"
        );
    }
}
