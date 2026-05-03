//! Docker command runner abstraction for Traefik devenv commands.
#![allow(clippy::module_name_repetitions)]
use anyhow::Context as _;

#[derive(Debug, Clone)]
pub struct CommandOutput {
    pub exit_code: i32,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

impl CommandOutput {
    pub fn stdout_str(&self) -> anyhow::Result<String> {
        String::from_utf8(self.stdout.clone()).context("command stdout is not valid UTF-8")
    }
}

pub trait DockerRunner: Send + Sync {
    fn run(&self, args: &[&str]) -> anyhow::Result<CommandOutput>;
}

pub struct SystemDockerRunner;

impl DockerRunner for SystemDockerRunner {
    #[cfg_attr(coverage_nightly, coverage(off))]
    fn run(&self, args: &[&str]) -> anyhow::Result<CommandOutput> {
        let output = std::process::Command::new("docker")
            .args(args)
            .output()
            .context("failed to run docker")?;
        Ok(CommandOutput {
            exit_code: output.status.code().unwrap_or(-1),
            stdout: output.stdout,
            stderr: output.stderr,
        })
    }
}

pub fn run_checked(runner: &dyn DockerRunner, args: &[&str]) -> anyhow::Result<CommandOutput> {
    let out = runner.run(args)?;
    if out.exit_code != 0 {
        let stderr = String::from_utf8_lossy(&out.stderr);
        anyhow::bail!(
            "docker {} failed (exit {}): {}",
            args.first().unwrap_or(&""),
            out.exit_code,
            stderr.trim()
        );
    }
    Ok(out)
}
