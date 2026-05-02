//! Re-export diff utilities from `graft-engine` so internal modules keep existing import paths.
#![allow(unused_imports)]
#[allow(clippy::module_name_repetitions)]
pub use graft_engine::diff::unified_diff;
