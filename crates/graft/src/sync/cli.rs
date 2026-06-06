use std::path::PathBuf;

use clap::{Parser, Subcommand};

/// Arguments for the `sync` subcommand.
#[derive(Parser, Debug)]
pub struct SyncArgs {
    #[command(subcommand)]
    pub command: SyncCommand,
}

/// Available sync targets.
#[derive(Subcommand, Debug)]
pub enum SyncCommand {
    /// Synchronize files from the upstream template repository
    File(SyncFileArgs),
    /// Synchronize repository settings (spec) to GitHub
    Repo(SyncRepoArgs),
}

/// Arguments for `sync file`.
#[derive(Parser, Debug)]
// CLI mode flags cannot be a state machine — they map directly to clap boolean arguments
// and mutual exclusion is enforced by `conflicts_with_all`.
#[allow(clippy::struct_excessive_bools)]
pub struct SyncFileArgs {
    /// Path to the local graft manifest file (used as overlay when
    /// --upstream-manifest is also given; may be absent in that case)
    #[arg(
        short = 'm',
        long = "manifest",
        default_value = ".github/graft/config.yaml"
    )]
    pub manifest: PathBuf,

    /// Upstream manifest reference in `owner/repo@ref:path` format.
    ///
    /// When given the manifest is fetched from the upstream repository and
    /// merged with the local manifest (local wins on conflicting paths).
    /// If the local manifest file does not exist only the upstream manifest
    /// is used.
    ///
    /// Example:
    ///   graft sync file --upstream-manifest naa0yama/boilerplate-rust@main:.github/graft/config.yaml
    #[arg(long = "upstream-manifest", value_name = "OWNER/REPO@REF:PATH")]
    pub upstream_manifest: Option<String>,

    /// Show what would change without writing any files
    #[arg(short = 'n', long = "dry-run")]
    pub dry_run: bool,

    /// Validate the manifest only; do not contact upstream
    #[arg(long = "validate", conflicts_with_all = ["ci_check", "patch_refresh"])]
    pub validate: bool,

    /// Detect drift and emit GitHub Actions annotations (implies --dry-run)
    #[arg(long = "ci-check", conflicts_with_all = ["validate", "patch_refresh"])]
    pub ci_check: bool,

    /// Re-generate patch files from the current upstream diff
    #[arg(long = "patch-refresh", conflicts_with_all = ["validate", "ci_check", "dry_run"])]
    pub patch_refresh: bool,

    /// Apply changes without prompting for confirmation
    #[arg(short = 'y', long = "yes", conflicts_with_all = ["dry_run", "validate", "ci_check", "patch_refresh"])]
    pub yes: bool,
}

/// Arguments for `sync repo`.
#[derive(Parser, Debug)]
#[allow(clippy::struct_excessive_bools)]
pub struct SyncRepoArgs {
    /// Path to the local graft manifest file (used as overlay when
    /// --upstream-manifest is also given; may be absent in that case)
    #[arg(
        short = 'm',
        long = "manifest",
        default_value = ".github/graft/config.yaml"
    )]
    pub manifest: PathBuf,

    /// Upstream manifest reference in `owner/repo@ref:path` format.
    ///
    /// When given the manifest is fetched from the upstream repository and
    /// merged with the local manifest (local wins on conflicting paths).
    /// If the local manifest file does not exist only the upstream manifest
    /// is used.
    ///
    /// Example:
    ///   graft sync repo --upstream-manifest naa0yama/boilerplate-rust@main:.github/graft/config.yaml
    #[arg(long = "upstream-manifest", value_name = "OWNER/REPO@REF:PATH")]
    pub upstream_manifest: Option<String>,

    /// Show what would change without applying
    #[arg(short = 'n', long = "dry-run")]
    pub dry_run: bool,

    /// Detect drift and exit non-zero if spec differs from GitHub (implies --dry-run)
    #[arg(long = "ci-check")]
    pub ci_check: bool,

    /// Apply changes without prompting for confirmation
    #[arg(short = 'y', long = "yes", conflicts_with_all = ["dry_run", "ci_check"])]
    pub yes: bool,
}
