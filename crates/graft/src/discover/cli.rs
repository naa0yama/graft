/// CLI argument definitions for the `discover` subcommand.
use clap::Parser;

/// Arguments for `graft discover`.
#[derive(Debug, Parser)]
pub struct DiscoverArgs {
    /// GitHub owner/org to scan for downstream repos.
    #[arg(long)]
    pub owner: String,

    /// Upstream template repository. Format: `[owner/]repo`.
    /// Owner is prepended from `--owner` when absent.
    #[arg(long)]
    pub upstream_repo: String,

    /// Filter to specific downstream repos (repeatable: `--repo a --repo b`).
    #[arg(long)]
    pub repo: Option<Vec<String>>,
}
