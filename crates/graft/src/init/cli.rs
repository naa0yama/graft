use std::path::PathBuf;

use clap::Parser;

/// Arguments for the `init` subcommand.
#[derive(Parser, Debug)]
pub struct InitArgs {
    /// Repository in `owner/name` format (stored in config upstream.repo)
    #[arg(short = 'r', long = "repo")]
    pub repo: Option<String>,

    /// Git ref to use (branch, tag, or commit SHA)
    #[arg(long = "ref", default_value = "main")]
    pub ref_: String,

    /// Output path for the generated config file
    #[arg(short = 'o', long = "output")]
    pub output: Option<PathBuf>,

    /// Interactively select files from the repository
    #[arg(long = "select")]
    pub select: bool,

    /// Overwrite existing files without prompting
    #[arg(long = "force")]
    pub force: bool,
}
