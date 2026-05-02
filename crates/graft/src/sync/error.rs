//! Re-export error types from `graft-manifest` so internal modules keep existing import paths.
#![allow(unused_imports)]
#[allow(clippy::module_name_repetitions)]
pub use graft_manifest::error::{SyncError, ValidationError};
