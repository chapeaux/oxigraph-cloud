//! Error types for the oxigraph-shacl crate.

use thiserror::Error;

/// Errors that can occur during SHACL validation.
#[derive(Debug, Error)]
pub enum ShaclError {
    /// Failed to compile SHACL shapes from input.
    #[error("Failed to compile SHACL shapes: {0}")]
    ShapeCompilation(String),

    /// Failed to serialize store data for validation.
    #[error("Failed to serialize store data: {0}")]
    StoreSerialization(String),

    /// Failed to load data into the validation engine.
    #[error("Failed to load data for validation: {0}")]
    DataLoading(String),

    /// The validation engine returned an error.
    #[error("Validation engine error: {0}")]
    ValidationEngine(String),
}
