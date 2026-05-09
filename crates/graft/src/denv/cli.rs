use clap::{Parser, Subcommand};

use crate::denv::traefik::cli::TraefikArgs;

#[derive(Parser, Debug)]
pub struct DenvArgs {
    #[command(subcommand)]
    pub command: DenvCommand,
}

#[derive(Subcommand, Debug)]
pub enum DenvCommand {
    /// Start devcontainer with Traefik routing
    Up,
    /// Full reset: stop container, remove container, remove image, clean up Traefik routes
    Down,
    /// Attach to running devcontainer (starts if not running)
    Exec,
    /// List running devcontainers with Traefik FQDNs
    Status,
    /// Manage Traefik reverse proxy infrastructure
    Traefik(TraefikArgs),
}
