//! Re-export manifest types from `graft-manifest` so internal modules keep existing import paths.
#![allow(unused_imports)]
pub use graft_manifest::manifest::{
    Actions, BranchProtection, BranchProtectionStatusChecks, BypassActor, Features, Label,
    Manifest, MergeStrategy, PullRequestRule, RefNameCondition, RequiredStatusChecks, Rule,
    Ruleset, RulesetConditions, RulesetRules, SelectedActions, Spec, StatusCheckContext, Strategy,
    Upstream, resolve_patch_path, validate_references, validate_schema,
};
