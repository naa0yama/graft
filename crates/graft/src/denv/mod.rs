pub mod cli;
pub mod traefik;

use std::process::ExitCode;

use cli::{DenvArgs, DenvCommand};

#[cfg_attr(coverage_nightly, coverage(off))]
pub fn execute(args: &DenvArgs) -> ExitCode {
    match &args.command {
        DenvCommand::Up => traefik::execute_up(),
        DenvCommand::Down => traefik::execute_down(),
        DenvCommand::Exec => traefik::execute_exec(),
        DenvCommand::Status => traefik::execute_status(),
        DenvCommand::Traefik(traefik_args) => traefik::execute(traefik_args),
    }
}
