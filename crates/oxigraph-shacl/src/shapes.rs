//! SHACL shape loading and compilation.
//!
//! Parses SHACL shapes from Turtle (or other RDF formats) and compiles
//! them into a [`SchemaIR`] for use with the validation engine.

use crate::error::ShaclError;
use rudof_rdf::rdf_core::RDFFormat;
use rudof_rdf::rdf_impl::ReaderMode;
use shacl_ir::compiled::schema_ir::SchemaIR;

/// Compiled SHACL shapes ready for validation.
///
/// This is a thin wrapper around rudof's [`SchemaIR`] that provides
/// convenient construction methods.
#[derive(Clone, Debug)]
pub struct CompiledShapes {
    schema: SchemaIR,
}

impl CompiledShapes {
    /// Compiles SHACL shapes from a Turtle string.
    pub fn from_turtle(turtle: &str) -> Result<Self, ShaclError> {
        Self::from_str(turtle, RDFFormat::Turtle)
    }

    /// Compiles SHACL shapes from an RDF string in the given format.
    pub fn from_str(data: &str, format: RDFFormat) -> Result<Self, ShaclError> {
        let schema = SchemaIR::from_str(data, &format, None, &ReaderMode::default())
            .map_err(|e| ShaclError::ShapeCompilation(e.to_string()))?;
        Ok(Self { schema })
    }

    /// Returns a reference to the underlying compiled schema.
    pub fn schema(&self) -> &SchemaIR {
        &self.schema
    }

    /// Returns the number of shapes that have targets (and will be validated).
    pub fn target_shape_count(&self) -> usize {
        self.schema.iter_with_targets().count()
    }

    /// Returns the total number of shapes in the schema.
    pub fn shape_count(&self) -> usize {
        self.schema.iter().count()
    }
}
