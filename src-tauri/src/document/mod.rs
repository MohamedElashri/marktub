//! Byte-preserving document and journal services used by the native editor.
//! CodeMirror owns working text; Rust retains byte provenance and recovery operations.
pub mod journal;
mod provenance;
pub use provenance::{Change, EditError, Insert, Provenance, Snapshot};
