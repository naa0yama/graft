pub mod cli;
pub mod traefik;

use std::process::ExitCode;

use cli::{DenvArgs, DenvCommand};

pub fn execute(args: &DenvArgs) -> ExitCode {
    match &args.command {
        DenvCommand::Traefik(traefik_args) => traefik::execute(traefik_args),
    }
}
