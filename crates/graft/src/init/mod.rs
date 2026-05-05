/// CLI argument definitions for the `init` subcommand.
pub mod cli;
/// Interactively generate a config from an upstream file listing.
mod generate;
/// JSON Schema constant and writer helper.
pub mod schema;
/// Interactive file + strategy picker widget.
mod select;

use std::io::{self, IsTerminal as _, Write as _};
use std::path::Path;
use std::process::ExitCode;

use anyhow::Context as _;
use cli::InitArgs;

use crate::sync::detect;
use crate::sync::runner::{GhRunner, SystemGhRunner};
use crate::sync::upstream::GhFetcher;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Return `true` when `repo` matches the `owner/name` pattern.
fn is_valid_repo(repo: &str) -> bool {
    let Some((owner, name)) = repo.split_once('/') else {
        return false;
    };
    if name.contains('/') {
        return false;
    }
    let valid_segment = |s: &str| {
        !s.is_empty()
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-')
    };
    valid_segment(owner) && valid_segment(name)
}

/// Validate repo format and return a descriptive error if invalid.
///
/// # Errors
///
/// Returns an error when `repo` does not match the `owner/name` pattern.
fn validate_repo_format(repo: &str) -> anyhow::Result<()> {
    if is_valid_repo(repo) {
        Ok(())
    } else {
        anyhow::bail!(
            "invalid repository '{repo}': must be owner/name format \
             (e.g. naa0yama/boilerplate-rust)"
        )
    }
}

/// Resolve the repository argument, prompting interactively when not provided.
///
/// `default_hint` is pre-filled into the interactive prompt when `--repo` is
/// absent, so the user can simply press Enter to accept the detected value.
///
/// # Errors
///
/// Returns an error when the repo format is invalid, the prompt is cancelled,
/// or `--repo` is absent in non-interactive mode.
#[cfg_attr(coverage_nightly, coverage(off))]
fn resolve_repo(
    args: &InitArgs,
    prompt: &str,
    non_tty_example: &str,
    default_hint: Option<&str>,
) -> anyhow::Result<String> {
    match &args.repo {
        Some(r) => {
            validate_repo_format(r)?;
            Ok(r.clone())
        }
        None => {
            if io::stdin().is_terminal() {
                let mut input = dialoguer::Input::<String>::new().with_prompt(prompt);
                if let Some(hint) = default_hint {
                    input = input.default(hint.to_owned()).show_default(true);
                }
                input
                    .validate_with(|i: &String| -> Result<(), &str> {
                        if is_valid_repo(i) {
                            Ok(())
                        } else {
                            Err("must be owner/name format (e.g. naa0yama/boilerplate-rust)")
                        }
                    })
                    .interact_text()
                    .context("repo prompt cancelled")
            } else {
                anyhow::bail!(
                    "--repo is required in non-interactive mode\nexample: {non_tty_example}"
                )
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Execute the `init` subcommand.
///
/// Generates `config.yaml` and `schema.json` for a template (upstream) project.
#[cfg_attr(coverage_nightly, coverage(off))]
pub fn execute(args: &InitArgs) -> ExitCode {
    match run(args, &GhFetcher::new(), &SystemGhRunner) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            tracing::error!("init failed: {e:#}");
            ExitCode::FAILURE
        }
    }
}

/// Core logic for `init`.
///
/// # Errors
///
/// Returns an error when upstream initialization fails.
#[cfg_attr(coverage_nightly, coverage(off))]
pub fn run(
    args: &InitArgs,
    fetcher: &dyn crate::sync::upstream::UpstreamFetcher,
    runner: &dyn GhRunner,
) -> anyhow::Result<()> {
    run_upstream(args, fetcher, runner)
}

/// Upstream mode: generate `config.yaml` + `schema.json` for a template project.
///
/// # Errors
///
/// Returns an error when the repo cannot be determined, fetching fails,
/// or the output file cannot be written.
#[cfg_attr(coverage_nightly, coverage(off))]
fn run_upstream(
    args: &InitArgs,
    fetcher: &dyn crate::sync::upstream::UpstreamFetcher,
    runner: &dyn GhRunner,
) -> anyhow::Result<()> {
    let output = args
        .output
        .as_deref()
        .unwrap_or_else(|| Path::new(".github/graft/config.yaml"));

    // -----------------------------------------------------------------------
    // 1. Check for existing output file
    // -----------------------------------------------------------------------
    if output.exists() && !args.force {
        if io::stdin().is_terminal() {
            let confirmed = dialoguer::Confirm::new()
                .with_prompt(format!("'{}' already exists. Overwrite?", output.display()))
                .default(false)
                .interact()
                .context("confirmation prompt cancelled")?;
            if !confirmed {
                let mut stdout = io::stdout();
                writeln!(stdout, "Aborted.").context("failed to write to stdout")?;
                return Ok(());
            }
        } else {
            anyhow::bail!(
                "'{}' already exists; use --force to overwrite",
                output.display()
            );
        }
    }

    // -----------------------------------------------------------------------
    // 2. Determine repo (detect current repo as default hint)
    // -----------------------------------------------------------------------
    let hint = detect::detect_repo_hint(runner, false);
    let repo = resolve_repo(
        args,
        "Upstream repository (owner/name)",
        "graft init --repo owner/name --select",
        hint.as_deref(),
    )?;

    // -----------------------------------------------------------------------
    // 3. Determine mode and generate config content
    // -----------------------------------------------------------------------
    let content = if args.select || io::stdin().is_terminal() {
        generate::run_interactive(fetcher, &repo, &args.ref_, "")?
    } else {
        anyhow::bail!(
            "no mode specified; use --select\n\
             example: graft init --repo owner/name --select"
        );
    };

    // -----------------------------------------------------------------------
    // 4. Write output file and schema.json
    // -----------------------------------------------------------------------
    let output_dir = output.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(output_dir)
        .with_context(|| format!("failed to create directory '{}'", output_dir.display()))?;

    std::fs::write(output, &content)
        .with_context(|| format!("failed to write '{}'", output.display()))?;

    let schema_outcome = schema::write_schema_file(output_dir)
        .with_context(|| format!("failed to write schema.json to '{}'", output_dir.display()))?;

    let mut stdout = io::stdout();
    writeln!(stdout, "[OK] created '{}'", output.display()).context("failed to write to stdout")?;
    match schema_outcome {
        schema::WriteOutcome::Created => writeln!(
            stdout,
            "[OK] created '{}/schema.json'",
            output_dir.display()
        )
        .context("failed to write to stdout")?,
        schema::WriteOutcome::Updated => writeln!(
            stdout,
            "[OK] updated '{}/schema.json'",
            output_dir.display()
        )
        .context("failed to write to stdout")?,
        schema::WriteOutcome::Unchanged => writeln!(
            stdout,
            "'{}/schema.json' is already up to date.",
            output_dir.display()
        )
        .context("failed to write to stdout")?,
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // ---------------------------------------------------------------------------
    // is_valid_repo
    // ---------------------------------------------------------------------------

    #[test]
    fn valid_repo_accepts_owner_slash_name() {
        assert!(is_valid_repo("owner/repo"));
        assert!(is_valid_repo("naa0yama/boilerplate-rust"));
        assert!(is_valid_repo("my.org/my_repo-name"));
    }

    #[test]
    fn valid_repo_rejects_missing_slash() {
        assert!(!is_valid_repo("no-slash"));
        assert!(!is_valid_repo(""));
    }

    #[test]
    fn valid_repo_rejects_multiple_slashes() {
        assert!(!is_valid_repo("owner/name/extra"));
    }

    #[test]
    fn valid_repo_rejects_empty_segments() {
        assert!(!is_valid_repo("/name"));
        assert!(!is_valid_repo("owner/"));
    }
}
