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
}
