//! Re-export output helpers from `graft-engine` so internal modules keep existing import paths.
#![allow(unused_imports)]
pub use graft_engine::output::{
    DriftOutcome, DriftSummary, RuleOutcome, StatusTag, Summary, build_pr_comment, colorize_diff,
    emit_diff, emit_drift_summary, emit_gha_annotations, emit_status, emit_summary,
};
