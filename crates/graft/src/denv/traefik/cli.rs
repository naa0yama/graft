use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
pub struct TraefikArgs {
    #[command(subcommand)]
    pub command: TraefikCommand,
}

#[derive(Subcommand, Debug)]
pub enum TraefikCommand {
    /// Install Traefik binary and configure systemd user service
    Setup,
    /// Start devcontainer with Traefik routing
    Up,
    /// Stop and remove devcontainer, clean up routes
    Down,
    /// Attach to running devcontainer (starts if not running)
    Exec,
    /// List running devcontainers with Traefik FQDNs
    Status,
}
