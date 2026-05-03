use clap::{Parser, Subcommand};

use crate::denv::traefik::cli::TraefikArgs;

#[derive(Parser, Debug)]
pub struct DenvArgs {
    #[command(subcommand)]
    pub command: DenvCommand,
}

#[derive(Subcommand, Debug)]
pub enum DenvCommand {
    /// Manage Traefik reverse proxy and devcontainer lifecycle
    Traefik(TraefikArgs),
}
