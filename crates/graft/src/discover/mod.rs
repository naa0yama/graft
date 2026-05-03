//! Discovery of downstream repositories and the `graft discover` subcommand.
//!
//! Queries the GitHub API to find repositories under an owner whose `parent`
//! or `template_repository` matches the supplied upstream repo, then prints
//! each match to stdout (one `owner/repo` per line).

pub mod cli;

use std::io::{self, Write as _};
use std::process::ExitCode;

use anyhow::Context as _;

use crate::sync::runner::{GhOutput, GhRunner, SystemGhRunner, run_checked};
use cli::DiscoverArgs;

// ---------------------------------------------------------------------------
// Public entry points
// ---------------------------------------------------------------------------

/// Execute the `discover` subcommand.
#[cfg_attr(coverage_nightly, coverage(off))]
pub fn execute(args: &DiscoverArgs) -> ExitCode {
    let runner = SystemGhRunner;
    match run(args, &runner) {
        Ok(repos) => {
            let mut stdout = io::stdout();
            for repo in &repos {
                if let Err(e) = writeln!(stdout, "{repo}") {
                    tracing::error!("failed to write to stdout: {e:#}");
                    return ExitCode::FAILURE;
                }
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            tracing::error!("discover failed: {e:#}");
            ExitCode::FAILURE
        }
    }
}

/// Run the discover workflow and return the list of downstream repos.
///
/// # Errors
///
/// Returns an error when the repository listing API call fails.
pub fn run(args: &DiscoverArgs, runner: &dyn GhRunner) -> anyhow::Result<Vec<String>> {
    let upstream = resolve_upstream(&args.upstream_repo, &args.owner);
    let filter = args.repo.as_deref().unwrap_or(&[]);
    discover_downstream_repos(&args.owner, &upstream, filter, runner)
}

// ---------------------------------------------------------------------------
// Internal deserialization types
// ---------------------------------------------------------------------------

#[derive(serde::Deserialize)]
struct RepoMeta {
    fork: bool,
    parent: Option<RepoRef>,
    template_repository: Option<RepoRef>,
}

#[derive(serde::Deserialize)]
struct RepoRef {
    full_name: String,
}

// ---------------------------------------------------------------------------
// Core discovery logic
// ---------------------------------------------------------------------------

/// Discover downstream repositories under `owner` whose `parent` or
/// `template_repository` matches `upstream_repo`.
///
/// When `filter_repos` is non-empty, only those repos are checked (no listing).
/// The upstream repo itself is never included in the result.
///
/// # Errors
///
/// Returns an error when the repository listing API call fails.
/// Per-repo metadata fetch failures are logged as warnings and skipped.
pub fn discover_downstream_repos(
    owner: &str,
    upstream_repo: &str,
    filter_repos: &[String],
    runner: &dyn GhRunner,
) -> anyhow::Result<Vec<String>> {
    let repos_to_check: Vec<String> = if filter_repos.is_empty() {
        list_repos(owner, runner)?
    } else {
        filter_repos.to_vec()
    };

    let mut result = Vec::new();
    for repo in repos_to_check {
        if repo == upstream_repo {
            continue;
        }
        if is_downstream_of(&repo, upstream_repo, runner) == Some(true) {
            result.push(repo);
        }
    }
    Ok(result)
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Resolve a full `owner/repo` string from the CLI argument.
///
/// Prepends `default_owner/` when the argument contains no `/`.
fn resolve_upstream(upstream_repo: &str, default_owner: &str) -> String {
    if upstream_repo.contains('/') {
        upstream_repo.to_owned()
    } else {
        format!("{default_owner}/{upstream_repo}")
    }
}

/// List all repositories under `owner` using the GitHub API with pagination.
///
/// Tries the `/orgs/{owner}/repos` endpoint first. If it returns a non-success
/// status (e.g., the owner is a user, not an org), falls back to
/// `/users/{owner}/repos`.
///
/// # Errors
///
/// Returns an error when both API endpoints fail.
fn list_repos(owner: &str, runner: &dyn GhRunner) -> anyhow::Result<Vec<String>> {
    let org_path = format!("/orgs/{owner}/repos");
    let org_out = runner
        .run(
            &["api", "--paginate", &org_path, "--jq", ".[].full_name"],
            None,
        )
        .with_context(|| format!("failed to spawn gh for GET {org_path}"))?;

    let raw = if org_out.success() {
        org_out.stdout
    } else {
        let user_path = format!("/users/{owner}/repos");
        let out = run_checked(
            runner,
            &["api", "--paginate", &user_path, "--jq", ".[].full_name"],
            None,
            &format!("GET {user_path}"),
        )?;
        out.stdout
    };

    let stdout = String::from_utf8(raw).context("repository list output is not valid UTF-8")?;
    let repos = stdout
        .lines()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect();
    Ok(repos)
}

/// Check whether `repo` is a downstream of `upstream_repo`.
///
/// Returns `Some(true)` if the repo's `parent.full_name` or
/// `template_repository.full_name` equals `upstream_repo`, `Some(false)` if
/// neither matches, and `None` when the API call or JSON parsing fails (a
/// warning is logged in that case).
fn is_downstream_of(repo: &str, upstream_repo: &str, runner: &dyn GhRunner) -> Option<bool> {
    let url = format!("repos/{repo}");
    let out: GhOutput = match runner.run(&["api", &url], None) {
        Ok(o) => o,
        Err(e) => {
            tracing::warn!("could not fetch metadata for '{repo}': {e:#}");
            return None;
        }
    };

    if !out.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        tracing::warn!(
            "gh api repos/{repo} exited with {:?}: {stderr}",
            out.exit_code
        );
        return None;
    }

    let meta: RepoMeta = match serde_json::from_slice(&out.stdout) {
        Ok(m) => m,
        Err(e) => {
            tracing::warn!("could not parse metadata JSON for '{repo}': {e:#}");
            return None;
        }
    };

    let fork_match = meta
        .fork
        .then_some(meta.parent)
        .flatten()
        .is_some_and(|p| p.full_name == upstream_repo);

    let template_match = meta
        .template_repository
        .is_some_and(|t| t.full_name == upstream_repo);

    Some(fork_match || template_match)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use std::collections::VecDeque;
    use std::sync::Mutex;

    use crate::sync::runner::{GhOutput, GhRunner};

    use super::*;

    // -----------------------------------------------------------------------
    // MockRunner
    // -----------------------------------------------------------------------

    struct MockRunner {
        queue: Mutex<VecDeque<anyhow::Result<GhOutput>>>,
    }

    impl MockRunner {
        fn new(responses: Vec<anyhow::Result<GhOutput>>) -> Self {
            Self {
                queue: Mutex::new(responses.into()),
            }
        }

        fn ok_stdout(stdout: impl Into<Vec<u8>>) -> GhOutput {
            GhOutput {
                exit_code: Some(0),
                stdout: stdout.into(),
                stderr: vec![],
            }
        }

        fn fail_stderr(stderr: impl Into<Vec<u8>>) -> GhOutput {
            GhOutput {
                exit_code: Some(1),
                stdout: vec![],
                stderr: stderr.into(),
            }
        }
    }

    impl GhRunner for MockRunner {
        fn run(&self, _args: &[&str], _stdin: Option<&[u8]>) -> anyhow::Result<GhOutput> {
            self.queue
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_else(|| Err(anyhow::anyhow!("MockRunner: no more responses queued")))
        }
    }

    // -----------------------------------------------------------------------
    // JSON helpers
    // -----------------------------------------------------------------------

    fn fork_meta(parent_full_name: &str) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "fork": true,
            "parent": { "full_name": parent_full_name, "default_branch": "main" },
            "template_repository": null
        }))
        .unwrap()
    }

    fn template_meta(tmpl_full_name: &str) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "fork": false,
            "parent": null,
            "template_repository": { "full_name": tmpl_full_name, "default_branch": "main" }
        }))
        .unwrap()
    }

    fn no_parent_meta() -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "fork": false,
            "parent": null,
            "template_repository": null
        }))
        .unwrap()
    }

    // -----------------------------------------------------------------------
    // discover_downstream_repos — happy path
    // -----------------------------------------------------------------------

    #[test]
    fn happy_path_three_repos_two_match() {
        let listing = "owner/fork-a\nowner/template-b\nowner/standalone-c\n";
        let runner = MockRunner::new(vec![
            Ok(MockRunner::ok_stdout(listing)),
            Ok(MockRunner::ok_stdout(fork_meta("upstream/repo"))),
            Ok(MockRunner::ok_stdout(template_meta("upstream/repo"))),
            Ok(MockRunner::ok_stdout(no_parent_meta())),
        ]);

        let result = discover_downstream_repos("owner", "upstream/repo", &[], &runner).unwrap();

        assert_eq!(result.len(), 2, "expected 2 matches, got: {result:?}");
        assert!(result.contains(&"owner/fork-a".to_owned()));
        assert!(result.contains(&"owner/template-b".to_owned()));
    }

    // -----------------------------------------------------------------------
    // filter_repos bypasses listing
    // -----------------------------------------------------------------------

    #[test]
    fn filter_repos_bypasses_listing() {
        let runner = MockRunner::new(vec![Ok(MockRunner::ok_stdout(fork_meta("upstream/repo")))]);

        let filter = vec!["owner/my-fork".to_owned()];
        let result = discover_downstream_repos("owner", "upstream/repo", &filter, &runner).unwrap();

        assert_eq!(result, vec!["owner/my-fork".to_owned()]);
    }

    #[test]
    fn filter_repos_excludes_non_matching() {
        let runner = MockRunner::new(vec![Ok(MockRunner::ok_stdout(no_parent_meta()))]);

        let filter = vec!["owner/unrelated".to_owned()];
        let result = discover_downstream_repos("owner", "upstream/repo", &filter, &runner).unwrap();

        assert!(result.is_empty());
    }

    // -----------------------------------------------------------------------
    // API listing failure returns Err
    // -----------------------------------------------------------------------

    #[test]
    fn listing_failure_propagates_err() {
        let runner = MockRunner::new(vec![
            Ok(MockRunner::fail_stderr("HTTP 403 Forbidden")),
            Ok(MockRunner::fail_stderr("HTTP 403 Forbidden")),
        ]);

        let result = discover_downstream_repos("owner", "upstream/repo", &[], &runner);
        assert!(result.is_err(), "expected Err, got Ok");
    }

    // -----------------------------------------------------------------------
    // Org endpoint fails → falls back to user endpoint
    // -----------------------------------------------------------------------

    #[test]
    fn listing_org_fail_falls_back_to_user() {
        let listing = "owner/repo-a\n";
        let runner = MockRunner::new(vec![
            Ok(MockRunner::fail_stderr("HTTP 404 Not Found")),
            Ok(MockRunner::ok_stdout(listing)),
            Ok(MockRunner::ok_stdout(fork_meta("upstream/repo"))),
        ]);

        let result = discover_downstream_repos("owner", "upstream/repo", &[], &runner).unwrap();

        assert_eq!(result, vec!["owner/repo-a".to_owned()]);
    }

    // -----------------------------------------------------------------------
    // Per-repo metadata failure is skipped (not propagated)
    // -----------------------------------------------------------------------

    #[test]
    fn per_repo_metadata_failure_is_skipped() {
        let listing = "owner/repo-a\nowner/repo-b\n";
        let runner = MockRunner::new(vec![
            Ok(MockRunner::ok_stdout(listing)),
            Ok(MockRunner::fail_stderr("HTTP 404 Not Found")),
            Ok(MockRunner::ok_stdout(fork_meta("upstream/repo"))),
        ]);

        let result = discover_downstream_repos("owner", "upstream/repo", &[], &runner).unwrap();

        assert_eq!(result, vec!["owner/repo-b".to_owned()]);
    }

    #[test]
    fn per_repo_spawn_failure_is_skipped() {
        let listing = "owner/repo-a\n";
        let runner = MockRunner::new(vec![
            Ok(MockRunner::ok_stdout(listing)),
            Err(anyhow::anyhow!("gh not found")),
        ]);

        let result = discover_downstream_repos("owner", "upstream/repo", &[], &runner).unwrap();

        assert!(result.is_empty());
    }

    // -----------------------------------------------------------------------
    // Upstream repo itself is excluded even if it matches
    // -----------------------------------------------------------------------

    #[test]
    fn upstream_repo_excluded_from_results() {
        let listing = "upstream/repo\nowner/downstream\n";
        let runner = MockRunner::new(vec![
            Ok(MockRunner::ok_stdout(listing)),
            Ok(MockRunner::ok_stdout(fork_meta("upstream/repo"))),
        ]);

        let result = discover_downstream_repos("owner", "upstream/repo", &[], &runner).unwrap();

        assert!(!result.contains(&"upstream/repo".to_owned()));
        assert!(result.contains(&"owner/downstream".to_owned()));
    }

    #[test]
    fn upstream_repo_excluded_in_filter_path() {
        let filter = vec!["upstream/repo".to_owned(), "owner/downstream".to_owned()];
        let runner = MockRunner::new(vec![Ok(MockRunner::ok_stdout(fork_meta("upstream/repo")))]);

        let result =
            discover_downstream_repos("upstream", "upstream/repo", &filter, &runner).unwrap();

        assert!(!result.contains(&"upstream/repo".to_owned()));
        assert!(result.contains(&"owner/downstream".to_owned()));
    }

    // -----------------------------------------------------------------------
    // Empty owner (no repos) returns empty vec
    // -----------------------------------------------------------------------

    #[test]
    fn empty_owner_returns_empty_vec() {
        let runner = MockRunner::new(vec![Ok(MockRunner::ok_stdout(""))]);

        let result =
            discover_downstream_repos("empty-owner", "upstream/repo", &[], &runner).unwrap();

        assert!(result.is_empty());
    }

    // -----------------------------------------------------------------------
    // resolve_upstream
    // -----------------------------------------------------------------------

    #[test]
    fn resolve_upstream_bare_name_prepends_owner() {
        assert_eq!(
            resolve_upstream("boilerplate-rust", "naa0yama"),
            "naa0yama/boilerplate-rust"
        );
    }

    #[test]
    fn resolve_upstream_full_name_kept_as_is() {
        assert_eq!(
            resolve_upstream("naa0yama/boilerplate-rust", "other"),
            "naa0yama/boilerplate-rust"
        );
    }
}
