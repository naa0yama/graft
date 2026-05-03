use clap::{Parser, Subcommand};

use crate::discover::cli::DiscoverArgs;
use crate::init::cli::InitArgs;
use crate::issue_sync::cli::IssueSyncArgs;
use crate::sync::cli::SyncArgs;

/// graft CLI for pulling upstream files into downstream repos.
#[derive(Parser, Debug)]
#[command(about, version = crate::APP_VERSION)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

/// Available subcommands.
#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Synchronize files or repository settings from upstream
    Sync(SyncArgs),
    /// Initialize a graft configuration file
    Init(InitArgs),
    /// Detect upstream drift and manage a tracking GitHub Issue
    IssueSync(IssueSyncArgs),
    /// Discover downstream repositories that fork or use this repo as a template
    Discover(DiscoverArgs),
}
