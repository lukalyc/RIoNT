//! RIONT's in-memory NetworkTables engine: topic store, values, metadata,
//! publish-rate windowing, and pose classification/decoding.
//!
//! Product-agnostic by design: this crate knows nothing about any
//! particular dashboard. Both RIONT (the inspector TUI) and other tools
//! built on the same engine consume it.

pub mod pose;
pub mod schema;
pub mod store;
