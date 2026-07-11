/// Traefik devcontainer lifecycle management.
pub mod cli;
pub mod config;
pub mod routes;
pub mod runner;

use std::io::Write as _;
use std::path::Path;
use std::process::ExitCode;

use anyhow::Context as _;

struct TraefikRelease {
    tag: String,
    asset_url: String,
    digest: String,
}

use cli::{TraefikArgs, TraefikCommand};
use config::{
    TRAEFIK_PORT_DASHBOARD, TRAEFIK_PORT_ROUTER, traefik_bin, traefik_config, traefik_dynamic_dir,
    traefik_service, write_systemd_unit, write_traefik_yml,
};
use routes::{normalize_branch, remove_routes, write_routes};
use runner::{
    DockerRunner, GitBranchResolver, IpResolver, SystemDockerRunner, SystemGitBranchResolver,
    SystemIpResolver, run_checked,
};

/// Returns `true` when all three Traefik env vars are present and `TRAEFIK_MANAGED` is `"1"`.
fn routes_update_env_guard(managed: &str, project: &str, dynamic_dir: &str) -> bool {
    managed == "1" && !project.is_empty() && !dynamic_dir.is_empty()
}

/// Dispatch a Traefik subcommand and return an exit code.
#[cfg_attr(coverage_nightly, coverage(off))]
pub fn execute(args: &TraefikArgs) -> ExitCode {
    let docker = SystemDockerRunner;
    match &args.command {
        TraefikCommand::Setup => run(cmd_setup(&docker)),
    }
}

#[cfg_attr(coverage_nightly, coverage(off))]
pub fn execute_up() -> ExitCode {
    run(cmd_up(&SystemDockerRunner))
}

#[cfg_attr(coverage_nightly, coverage(off))]
pub fn execute_down() -> ExitCode {
    run(cmd_down(&SystemDockerRunner))
}

#[cfg_attr(coverage_nightly, coverage(off))]
pub fn execute_exec() -> ExitCode {
    run(cmd_exec(&SystemDockerRunner))
}

#[cfg_attr(coverage_nightly, coverage(off))]
pub fn execute_status() -> ExitCode {
    run(cmd_status(&SystemDockerRunner))
}

#[cfg_attr(coverage_nightly, coverage(off))]
pub fn execute_routes_update() -> ExitCode {
    run(cmd_routes_update(
        &SystemIpResolver,
        &SystemGitBranchResolver,
    ))
}

#[cfg_attr(coverage_nightly, coverage(off))]
fn run(result: anyhow::Result<()>) -> ExitCode {
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            tracing::error!("{e:#}");
            ExitCode::FAILURE
        }
    }
}

// ---------------------------------------------------------------------------
// Host check
// ---------------------------------------------------------------------------

#[cfg_attr(coverage_nightly, coverage(off))]
fn host_check() -> anyhow::Result<()> {
    let in_container = std::env::var("MISE_ENV").as_deref() == Ok("devcontainer")
        || Path::new("/.dockerenv").exists();
    if in_container {
        anyhow::bail!("must be run on the host, not inside a devcontainer");
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// tmux pane options
// ---------------------------------------------------------------------------

struct TmuxPane {
    /// Raw `$TMUX` value (`"socket_path,pid,pane_id"`); empty when tmux is absent.
    tmux_env: String,
    /// Raw `$TMUX_PANE` value (e.g., `"%31"`); required for `set-option -p` target resolution.
    tmux_pane: String,
}

impl TmuxPane {
    fn from_env() -> Self {
        Self {
            tmux_env: std::env::var("TMUX").unwrap_or_default(),
            tmux_pane: std::env::var("TMUX_PANE").unwrap_or_default(),
        }
    }

    #[cfg(test)]
    fn new(tmux_env: impl Into<String>, tmux_pane: impl Into<String>) -> Self {
        Self {
            tmux_env: tmux_env.into(),
            tmux_pane: tmux_pane.into(),
        }
    }

    const fn active(&self) -> bool {
        !self.tmux_env.is_empty()
    }

    fn env_value(&self) -> &str {
        &self.tmux_env
    }

    fn socket_path(&self) -> Option<&str> {
        if !self.active() {
            return None;
        }
        let path = self.tmux_env.split(',').next().unwrap_or_default();
        if path.is_empty() { None } else { Some(path) }
    }

    fn pane_id(&self) -> Option<&str> {
        if self.tmux_pane.is_empty() {
            None
        } else {
            Some(&self.tmux_pane)
        }
    }

    fn tmux_set_option_cmd(&self) -> Option<std::process::Command> {
        if !self.active() {
            return None;
        }
        let mut cmd = std::process::Command::new("tmux");
        cmd.arg("set-option");
        if let Some(pane) = self.pane_id() {
            cmd.args(["-t", pane]);
        }
        Some(cmd)
    }

    fn set(&self, option: &str, value: &str) {
        if let Some(mut cmd) = self.tmux_set_option_cmd() {
            cmd.args(["-p", option, value]);
            let _ = cmd.status();
        }
    }

    fn clear(&self, option: &str) {
        if let Some(mut cmd) = self.tmux_set_option_cmd() {
            cmd.args(["-pu", option]);
            let _ = cmd.status();
        }
    }

    fn set_session(&self, proj: &str, br: &str, ws: &str) {
        self.set("@role", "claude");
        self.set("@project-path", ws);
        self.set("@pane-name", &format!("{proj}:{br}"));
    }

    fn clear_session(&self) {
        self.clear("@role");
        self.clear("@project-path");
        self.clear("@pane-name");
    }
}

// ---------------------------------------------------------------------------
// Git helpers
// ---------------------------------------------------------------------------

#[cfg_attr(coverage_nightly, coverage(off))]
fn workspace() -> anyhow::Result<String> {
    let out = std::process::Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .context("git rev-parse failed")?;
    if !out.status.success() {
        anyhow::bail!("not inside a git repository");
    }
    Ok(String::from_utf8(out.stdout)
        .context("git output not UTF-8")?
        .trim()
        .to_owned())
}

#[cfg_attr(coverage_nightly, coverage(off))]
fn project(workspace: &str) -> String {
    Path::new(workspace)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown")
        .to_owned()
}

#[cfg_attr(coverage_nightly, coverage(off))]
fn branch(workspace: &str) -> anyhow::Result<String> {
    let out = std::process::Command::new("git")
        .args(["-C", workspace, "branch", "--show-current"])
        .output()
        .context("git branch failed")?;
    let raw = String::from_utf8(out.stdout)
        .context("git output not UTF-8")?
        .trim()
        .to_owned();
    if raw.is_empty() {
        let hash_out = std::process::Command::new("git")
            .args(["-C", workspace, "rev-parse", "--short", "HEAD"])
            .output()
            .context("git rev-parse HEAD failed")?;
        let hash = String::from_utf8(hash_out.stdout)
            .context("git output not UTF-8")?
            .trim()
            .to_owned();
        Ok(normalize_branch(&format!("detached-{hash}")))
    } else {
        Ok(normalize_branch(&raw))
    }
}

// ---------------------------------------------------------------------------
// devcontainer.json helpers
// ---------------------------------------------------------------------------

fn strip_jsonc_comments(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let mut chars = src.chars().peekable();
    let mut in_string = false;

    while let Some(ch) = chars.next() {
        if in_string {
            out.push(ch);
            if ch == '\\' {
                if let Some(escaped) = chars.next() {
                    out.push(escaped);
                }
            } else if ch == '"' {
                in_string = false;
            }
        } else {
            match ch {
                '"' => {
                    in_string = true;
                    out.push(ch);
                }
                '/' => match chars.peek() {
                    Some('/') => {
                        chars.next();
                        for c in chars.by_ref() {
                            if c == '\n' {
                                out.push('\n');
                                break;
                            }
                        }
                    }
                    Some('*') => {
                        chars.next();
                        let mut prev = '\0';
                        for c in chars.by_ref() {
                            if c == '\n' {
                                out.push('\n');
                            }
                            if prev == '*' && c == '/' {
                                break;
                            }
                            prev = c;
                        }
                    }
                    _ => out.push(ch),
                },
                _ => out.push(ch),
            }
        }
    }
    out
}

struct DevcontainerMeta {
    ports: Vec<String>,
    container_uid: String,
    user_name: String,
    config: serde_json::Value,
}

fn read_devcontainer(workspace: &str) -> anyhow::Result<DevcontainerMeta> {
    let path = Path::new(workspace)
        .join(".devcontainer")
        .join("devcontainer.json");
    let raw = std::fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
    let clean = strip_jsonc_comments(&raw);
    let config: serde_json::Value =
        serde_json::from_str(&clean).context("parse devcontainer.json")?;
    let ports = config
        .get("portsAttributes")
        .and_then(|pa| pa.as_object())
        .map(|obj| obj.keys().cloned().collect())
        .unwrap_or_default();
    let container_uid = config
        .pointer("/build/args/USER_UID")
        .and_then(|u| u.as_str())
        .unwrap_or("1000")
        .to_owned();
    let user_name = config
        .get("remoteUser")
        .and_then(|u| u.as_str())
        .unwrap_or("user")
        .to_owned();
    Ok(DevcontainerMeta {
        ports,
        container_uid,
        user_name,
        config,
    })
}

// ---------------------------------------------------------------------------
// Traefik binary management
// ---------------------------------------------------------------------------

#[cfg_attr(coverage_nightly, coverage(off))]
fn traefik_installed_version() -> Option<String> {
    let bin = traefik_bin();
    if !bin.exists() {
        return None;
    }
    let out = std::process::Command::new(&bin)
        .arg("version")
        .output()
        .ok()?;
    let text = String::from_utf8(out.stdout).ok()?;
    // Output: "Version:      3.4.1\n..."
    text.lines()
        .find(|l| l.trim_start().starts_with("Version:"))
        .and_then(|l| l.split_whitespace().nth(1).map(|v| format!("v{v}")))
}

#[cfg_attr(coverage_nightly, coverage(off))]
fn traefik_latest() -> anyhow::Result<TraefikRelease> {
    let arch = match std::env::consts::ARCH {
        "x86_64" => "amd64",
        "aarch64" => "arm64",
        a => anyhow::bail!("unsupported architecture: {a}"),
    };
    let out = std::process::Command::new("curl")
        .args([
            "-sfSL",
            "--retry",
            "3",
            "https://api.github.com/repos/traefik/traefik/releases?per_page=30",
        ])
        .output()
        .context("curl github API")?;
    if !out.status.success() {
        anyhow::bail!("curl failed: {}", String::from_utf8_lossy(&out.stderr));
    }
    let releases: serde_json::Value =
        serde_json::from_slice(&out.stdout).context("parse github releases JSON")?;
    let releases = releases.as_array().context("expected JSON array")?;

    let now = jiff::Timestamp::now();
    let min_age = jiff::SignedDuration::from_hours(7 * 24);

    for release in releases {
        let Some(tag) = release.get("tag_name").and_then(|t| t.as_str()) else {
            continue;
        };
        if !tag.starts_with("v3.") {
            continue;
        }
        if release
            .get("draft")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
        {
            continue;
        }
        if release
            .get("prerelease")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
        {
            continue;
        }
        let published_at = release
            .get("published_at")
            .and_then(|v| v.as_str())
            .context("missing published_at")?;
        let ts: jiff::Timestamp = published_at
            .parse()
            .with_context(|| format!("parse published_at: {published_at}"))?;
        if now.duration_since(ts) < min_age {
            continue;
        }
        let expected_name = format!("traefik_{tag}_linux_{arch}.tar.gz");
        let assets = release
            .get("assets")
            .and_then(|a| a.as_array())
            .context("missing assets")?;
        let asset = assets
            .iter()
            .find(|a| a.get("name").and_then(|n| n.as_str()) == Some(expected_name.as_str()))
            .with_context(|| format!("{expected_name} not found in release assets"))?;
        let asset_url = asset
            .get("browser_download_url")
            .and_then(|u| u.as_str())
            .context("missing browser_download_url")?
            .to_owned();
        let digest = asset
            .get("digest")
            .and_then(|d| d.as_str())
            .context("missing digest")?
            .strip_prefix("sha256:")
            .context("digest is not sha256")?
            .to_owned();
        return Ok(TraefikRelease {
            tag: tag.to_owned(),
            asset_url,
            digest,
        });
    }
    anyhow::bail!("no stable traefik v3 release older than 7 days found")
}

#[cfg_attr(coverage_nightly, coverage(off))]
fn traefik_install_version(release: &TraefikRelease) -> anyhow::Result<()> {
    let version = &release.tag;

    let tmpfile = tempfile::NamedTempFile::new().context("create temp file")?;
    let tmpfile_path = tmpfile
        .path()
        .to_str()
        .context("temp file path not UTF-8")?
        .to_owned();

    let status = std::process::Command::new("curl")
        .args([
            "-fSL",
            "--retry",
            "3",
            "--retry-delay",
            "2",
            "--retry-connrefused",
            "-o",
            &tmpfile_path,
            &release.asset_url,
        ])
        .status()
        .context("curl download")?;
    anyhow::ensure!(status.success(), "failed to download traefik {version}");

    tracing::info!("Verifying checksum...");
    let sha_out = std::process::Command::new("sha256sum")
        .arg(tmpfile.path())
        .output()
        .context("sha256sum")?;
    let actual = String::from_utf8(sha_out.stdout)
        .context("sha256sum output")?
        .split_whitespace()
        .next()
        .context("sha256sum empty output")?
        .to_owned();
    anyhow::ensure!(
        release.digest == actual,
        "checksum mismatch (expected {}, got {actual})",
        release.digest
    );

    let bin_dir = traefik_bin()
        .parent()
        .context("traefik bin has no parent")?
        .to_path_buf();
    std::fs::create_dir_all(&bin_dir)
        .with_context(|| format!("create dir {}", bin_dir.display()))?;
    let bin_dir_str = bin_dir
        .to_str()
        .context("traefik bin dir path not UTF-8")?
        .to_owned();

    let status = std::process::Command::new("tar")
        .args(["-xzf", &tmpfile_path, "-C", &bin_dir_str, "traefik"])
        .status()
        .context("tar extract")?;
    anyhow::ensure!(status.success(), "failed to extract traefik binary");

    std::fs::set_permissions(
        traefik_bin(),
        std::os::unix::fs::PermissionsExt::from_mode(0o755),
    )
    .context("chmod traefik")?;

    tracing::info!("Installed traefik {version} to {}", traefik_bin().display());
    Ok(())
}

/// Returns "installed", "updated", or "already-latest".
#[cfg_attr(coverage_nightly, coverage(off))]
fn traefik_ensure_latest() -> anyhow::Result<&'static str> {
    let installed = traefik_installed_version();
    let latest = match traefik_latest() {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!("could not fetch latest traefik version: {e:#}");
            return if installed.is_some() {
                Ok("already-latest")
            } else {
                anyhow::bail!("traefik is not installed and version check failed");
            };
        }
    };
    let _ = writeln!(
        std::io::stdout(),
        "local:  {}",
        installed.as_deref().unwrap_or("not installed")
    );
    let _ = writeln!(std::io::stdout(), "latest: {}", latest.tag);
    match installed {
        None => {
            let _ = writeln!(std::io::stdout(), "Downloading traefik {}...", latest.tag);
            traefik_install_version(&latest)?;
            Ok("installed")
        }
        Some(ref v) if v != &latest.tag => {
            let _ = writeln!(std::io::stdout(), "Downloading traefik {}...", latest.tag);
            match traefik_install_version(&latest) {
                Ok(()) => Ok("updated"),
                Err(e) => {
                    tracing::warn!(
                        "failed to update traefik to {}: {e:#}; continuing with {v}",
                        latest.tag
                    );
                    Ok("already-latest")
                }
            }
        }
        _ => {
            let _ = writeln!(
                std::io::stdout(),
                "traefik {}: already at latest",
                installed.as_deref().unwrap_or("unknown")
            );
            Ok("already-latest")
        }
    }
}

// ---------------------------------------------------------------------------
// Docker helpers
// ---------------------------------------------------------------------------

#[cfg_attr(coverage_nightly, coverage(off))]
fn ensure_network(docker: &dyn DockerRunner) -> anyhow::Result<()> {
    let out = docker.run(&["network", "inspect", "devcontainer-traefik"])?;
    if out.exit_code != 0 {
        run_checked(docker, &["network", "create", "devcontainer-traefik"])
            .context("create devcontainer-traefik network")?;
    }
    Ok(())
}

#[cfg_attr(coverage_nightly, coverage(off))]
fn container_id(docker: &dyn DockerRunner, workspace: &str) -> anyhow::Result<Vec<String>> {
    let out = run_checked(
        docker,
        &[
            "ps",
            "-aq",
            "--filter",
            &format!("label=devcontainer.local_folder={workspace}"),
        ],
    )?;
    let ids: Vec<String> = out
        .stdout_str()?
        .lines()
        .map(|l| l.trim().to_owned())
        .filter(|l| !l.is_empty())
        .collect();
    Ok(ids)
}

#[cfg_attr(coverage_nightly, coverage(off))]
fn running_container_id(docker: &dyn DockerRunner, workspace: &str) -> anyhow::Result<String> {
    let out = run_checked(
        docker,
        &[
            "ps",
            "-q",
            "--filter",
            &format!("label=devcontainer.local_folder={workspace}"),
        ],
    )?;
    Ok(out.stdout_str()?.trim().to_owned())
}

#[cfg_attr(coverage_nightly, coverage(off))]
fn container_network_ip(
    docker: &dyn DockerRunner,
    cid: &str,
    network: &str,
) -> anyhow::Result<String> {
    let fmt = format!("{{{{(index .NetworkSettings.Networks \"{network}\").IPAddress}}}}");
    let out = run_checked(docker, &["inspect", cid, "--format", &fmt])?;
    Ok(out.stdout_str()?.trim().to_owned())
}

// ---------------------------------------------------------------------------
// Session reference counting (PID-file based, /tmp/graft-denv-<hash>/<pid>)
// ---------------------------------------------------------------------------

fn workspace_hash(workspace: &str) -> u64 {
    // FNV-1a 64-bit — fixed seed, stable across runs
    let mut hash: u64 = 14_695_981_039_346_656_037;
    for byte in workspace.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(1_099_511_628_211);
    }
    hash
}

fn session_dir_in(base: &std::path::Path, workspace: &str) -> std::path::PathBuf {
    base.join(format!("graft-denv-{:016x}", workspace_hash(workspace)))
}

fn session_dir(workspace: &str) -> std::path::PathBuf {
    session_dir_in(&std::env::temp_dir(), workspace)
}

fn acquire_session_in(dir: &std::path::Path) -> anyhow::Result<std::path::PathBuf> {
    std::fs::create_dir_all(dir).context("create session dir")?;
    let lock = dir.join(std::process::id().to_string());
    std::fs::write(&lock, b"").context("create session lock")?;
    Ok(lock)
}

/// Removes `lock`, then returns `true` when no other live sessions remain.
/// Stale PID files (process no longer exists) are ignored.
fn release_session_in(lock: &std::path::Path, dir: &std::path::Path) -> bool {
    let _ = std::fs::remove_file(lock);
    let Ok(entries) = std::fs::read_dir(dir) else {
        return true;
    };
    !entries
        .filter_map(Result::ok)
        .filter_map(|e| e.file_name().into_string().ok())
        .filter_map(|name| name.parse::<u32>().ok())
        .any(|pid| std::path::Path::new(&format!("/proc/{pid}")).exists())
}

// ---------------------------------------------------------------------------
// Interactive exec helper
// ---------------------------------------------------------------------------

#[cfg_attr(coverage_nightly, coverage(off))]
fn exec_and_watch(docker: &dyn DockerRunner, workspace: &str) -> anyhow::Result<()> {
    let dir = session_dir(workspace);
    let lock = acquire_session_in(&dir)?;

    std::process::Command::new("devcontainer")
        .args(["exec", "--workspace-folder", workspace, "bash"])
        .status()
        .context("devcontainer exec bash")?;

    let is_last = release_session_in(&lock, &dir);

    if !is_last {
        let _ = writeln!(
            std::io::stdout(),
            "Shell exited. Container still running. (mise run dev:exec to reconnect, mise run dev:down to stop)"
        );
        return Ok(());
    }

    // Last session: stop gracefully, remove routes, clean up old stopped containers.
    let cid = running_container_id(docker, workspace).unwrap_or_default();
    let proj = project(workspace);
    let dynamic_dir = traefik_dynamic_dir();

    let _ = writeln!(std::io::stdout(), "Shell exited. Stopping container...");

    if !cid.is_empty() {
        let _ = docker.run(&["stop", &cid]);
        remove_routes(&cid, &proj, &dynamic_dir).unwrap_or(());
    }

    // Remove any other stopped containers left from previous sessions.
    let all_cids = container_id(docker, workspace).unwrap_or_default();
    for old_cid in &all_cids {
        if old_cid.is_empty() || old_cid == &cid {
            continue;
        }
        remove_routes(old_cid, &proj, &dynamic_dir).unwrap_or(());
        let _ = docker.run(&["rm", old_cid]);
    }

    let _ = std::fs::remove_dir(&dir);
    let _ = writeln!(
        std::io::stdout(),
        "Container stopped. Use 'mise run dev:exec' to reconnect."
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

#[cfg_attr(coverage_nightly, coverage(off))]
fn cmd_setup(docker: &dyn DockerRunner) -> anyhow::Result<()> {
    host_check()?;
    crate::deps::require(&["curl", "sha256sum", "systemctl", "tar"])?;

    traefik_ensure_latest()?;
    let _ = writeln!(
        std::io::stdout(),
        "traefik: {}",
        traefik_installed_version().unwrap_or_default()
    );

    ensure_network(docker)?;

    let config_path = traefik_config();
    let dynamic_dir = traefik_dynamic_dir();
    let service_path = traefik_service();
    let bin_path = traefik_bin();

    std::fs::create_dir_all(&dynamic_dir)
        .with_context(|| format!("create {}", dynamic_dir.display()))?;

    write_traefik_yml(&config_path, &dynamic_dir)?;
    write_systemd_unit(&bin_path, &config_path, &service_path)?;

    std::process::Command::new("systemctl")
        .args(["--user", "daemon-reload"])
        .status()
        .context("systemctl daemon-reload")?;
    std::process::Command::new("systemctl")
        .args(["--user", "enable", "traefik.service"])
        .status()
        .context("systemctl enable")?;

    let active = std::process::Command::new("systemctl")
        .args(["--user", "is-active", "--quiet", "traefik.service"])
        .status()
        .context("systemctl is-active")?
        .success();
    if active {
        let _ = writeln!(std::io::stdout(), "Restarting traefik to apply config...");
        std::process::Command::new("systemctl")
            .args(["--user", "restart", "traefik.service"])
            .status()
            .context("systemctl restart")?;
    } else {
        std::process::Command::new("systemctl")
            .args(["--user", "start", "traefik.service"])
            .status()
            .context("systemctl start")?;
    }

    std::process::Command::new("systemctl")
        .args(["--user", "status", "traefik.service", "--no-pager"])
        .status()
        .context("systemctl status")?;

    let mut out = std::io::stdout();
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "Traefik router:    http://localhost:{TRAEFIK_PORT_ROUTER}"
    );
    let _ = writeln!(
        out,
        "Traefik dashboard: http://localhost:{TRAEFIK_PORT_DASHBOARD}/dashboard/"
    );
    Ok(())
}

#[allow(clippy::too_many_lines)]
#[cfg_attr(coverage_nightly, coverage(off))]
fn cmd_up(docker: &dyn DockerRunner) -> anyhow::Result<()> {
    host_check()?;
    crate::deps::require(&["docker", "devcontainer", "git", "curl", "sha256sum", "tar"])?;
    let tmux = TmuxPane::from_env();
    if tmux.active() {
        crate::deps::require(&["tmux"])?;
    }

    match traefik_ensure_latest()? {
        "installed" => {
            let _ = writeln!(
                std::io::stdout(),
                "traefik installed: {}",
                traefik_installed_version().unwrap_or_default()
            );
        }
        "updated" => {
            let _ = writeln!(
                std::io::stdout(),
                "traefik updated: {}",
                traefik_installed_version().unwrap_or_default()
            );
        }
        _ => {}
    }

    cleanup_stopped_containers(docker)?;
    ensure_network(docker)?;

    let ws = workspace()?;
    let proj = project(&ws);
    let br = branch(&ws)?;
    let dc = read_devcontainer(&ws)?;

    tmux.set_session(&proj, &br, &ws);

    if dc.ports.is_empty() {
        tracing::warn!("no portsAttributes found in devcontainer.json");
    }

    // Build runArgs JSON to merge into devcontainer.json
    let dynamic_dir_host = traefik_dynamic_dir();
    let dynamic_dir_str = dynamic_dir_host
        .to_str()
        .context("traefik dynamic dir path not UTF-8")?
        .to_owned();
    let container_dynamic_path = "/traefik-dynamic";
    let container_uid = dc.container_uid;
    let user_name = dc.user_name;
    let colorterm = std::env::var("COLORTERM").unwrap_or_default();
    let term = std::env::var("TERM").unwrap_or_else(|_| "xterm-256color".to_owned());

    let mut run_args: Vec<serde_json::Value> = vec![
        serde_json::json!(format!("--label=devcontainer.project={proj}")),
        serde_json::json!(format!("--env=COLORTERM={colorterm}")),
        serde_json::json!(format!("--env=TERM={term}")),
        serde_json::json!("--env=TRAEFIK_MANAGED=1"),
        serde_json::json!(format!("--env=TRAEFIK_PROJECT={proj}")),
        serde_json::json!(format!(
            "--env=TRAEFIK_DYNAMIC_DIR={container_dynamic_path}"
        )),
        serde_json::json!(format!(
            "--env=TRAEFIK_API_BASE=http://host.docker.internal:{TRAEFIK_PORT_DASHBOARD}"
        )),
        serde_json::json!(format!(
            "--mount=type=bind,source={dynamic_dir_str},target={container_dynamic_path}"
        )),
        serde_json::json!("--add-host=host.docker.internal:host-gateway"),
    ];

    // SSH agent forwarding
    let ssh_sock = std::env::var("SSH_AUTH_SOCK").unwrap_or_default();
    if !ssh_sock.is_empty() && Path::new(&ssh_sock).exists() {
        let target = format!("/run/user/{container_uid}/ssh-agent.sock");
        run_args.push(serde_json::json!(format!(
            "--mount=type=bind,source={ssh_sock},target={target}"
        )));
        run_args.push(serde_json::json!(format!("--env=SSH_AUTH_SOCK={target}")));
    } else {
        tracing::info!("SSH_AUTH_SOCK not available; skipping SSH agent forwarding");
    }

    // GPG agent socket forwarding
    let gpg_sock = std::process::Command::new("gpgconf")
        .args(["--list-dirs", "agent-socket"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_owned())
        .unwrap_or_default();
    if !gpg_sock.is_empty() && Path::new(&gpg_sock).exists() {
        let target = format!("/run/user/{container_uid}/gnupg/S.gpg-agent");
        run_args.push(serde_json::json!(format!(
            "--mount=type=bind,source={gpg_sock},target={target}"
        )));
    } else {
        tracing::info!("GPG agent socket not available; skipping GPG forwarding");
    }

    // tmux socket forwarding — enables hooks inside the container to update @pane-name
    match tmux.socket_path() {
        Some(tmux_sock) if Path::new(tmux_sock).exists() => {
            run_args.push(serde_json::json!(format!(
                "--mount=type=bind,source={tmux_sock},target={tmux_sock}"
            )));
            run_args.push(serde_json::json!(format!(
                "--env=TMUX={}",
                tmux.env_value()
            )));
            if let Some(pane) = tmux.pane_id() {
                run_args.push(serde_json::json!(format!("--env=TMUX_PANE={pane}")));
            }
        }
        Some(_) => tracing::info!("tmux socket not available; skipping tmux forwarding"),
        None => tracing::info!("TMUX not set; skipping tmux forwarding"),
    }

    // GPG public keyring (readonly — needed for key lookup without trustdb)
    let gpg_home = std::env::var("GNUPGHOME")
        .unwrap_or_else(|_| format!("{}/.gnupg", std::env::var("HOME").unwrap_or_default()));
    let pubring = Path::new(&gpg_home).join("pubring.kbx");
    if pubring.exists() {
        let pubring_str = pubring.to_string_lossy();
        run_args.push(serde_json::json!(format!(
            "--mount=type=bind,source={pubring_str},target=/home/{user_name}/.gnupg/pubring.kbx,readonly"
        )));
    }
    // GPG trust database (readonly — needed for correct key trust display)
    let trustdb = Path::new(&gpg_home).join("trustdb.gpg");
    if trustdb.exists() {
        let trustdb_str = trustdb.to_string_lossy();
        run_args.push(serde_json::json!(format!(
            "--mount=type=bind,source={trustdb_str},target=/home/{user_name}/.gnupg/trustdb.gpg,readonly"
        )));
    }

    // Merge runArgs into the already-parsed devcontainer config
    let mut config = dc.config;
    if let Some(obj) = config.as_object_mut() {
        obj.insert("runArgs".to_owned(), serde_json::Value::Array(run_args));
    }

    let tmpfile = tempfile::Builder::new()
        .suffix(".json")
        .tempfile()
        .context("create temp devcontainer config")?;
    let tmpfile_path = tmpfile
        .path()
        .to_str()
        .context("temp file path not UTF-8")?
        .to_owned();
    serde_json::to_writer(&tmpfile, &config).context("write merged devcontainer config")?;

    // devcontainer up — capture stdout for JSON parsing, inherit stderr for progress output
    let up_out = std::process::Command::new("devcontainer")
        .args([
            "up",
            "--workspace-folder",
            &ws,
            "--override-config",
            &tmpfile_path,
        ])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::inherit())
        .output()
        .context("devcontainer up")?;

    let result: serde_json::Value =
        serde_json::from_slice(&up_out.stdout).context("parse devcontainer up output")?;
    let cid = result
        .get("containerId")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty() && *s != "null")
        .context("containerId missing from devcontainer up output")?
        .to_owned();

    // Ensure network connection and get IP
    let mut ip = container_network_ip(docker, &cid, "devcontainer-traefik").unwrap_or_default();
    if ip.is_empty() {
        run_checked(
            docker,
            &["network", "connect", "devcontainer-traefik", &cid],
        )
        .with_context(|| {
            format!("failed to connect container {cid} to devcontainer-traefik network")
        })?;
        ip = container_network_ip(docker, &cid, "devcontainer-traefik").unwrap_or_default();
    }

    if !dc.ports.is_empty() && !ip.is_empty() {
        write_routes(&cid, &proj, &br, &ip, &dc.ports, &dynamic_dir_host)?;
    }

    // Splash
    let cid_short = &cid[..cid.len().min(12)];
    let mut out = std::io::stdout();
    let _ = writeln!(out);
    let _ = writeln!(out, "==================================================");
    let _ = writeln!(out, "  {proj} / {br}");
    let _ = writeln!(out, "==================================================");
    let _ = writeln!(out);
    let _ = writeln!(out, "  Container: {cid_short}");
    let _ = writeln!(out);
    if !dc.ports.is_empty() {
        let _ = writeln!(out, "  URLs:");
        for port in &dc.ports {
            let _ = writeln!(
                out,
                "    http://p{port}.{br}.{proj}.localhost:{TRAEFIK_PORT_ROUTER}"
            );
        }
        let _ = writeln!(out);
    }
    let _ = writeln!(
        out,
        "  Traefik router:    http://localhost:{TRAEFIK_PORT_ROUTER}"
    );
    let _ = writeln!(
        out,
        "  Traefik dashboard: http://localhost:{TRAEFIK_PORT_DASHBOARD}/dashboard/"
    );
    let _ = writeln!(out);
    let _ = writeln!(out, "  Reconnect: mise run dev:exec");
    let _ = writeln!(out, "==================================================");
    let _ = writeln!(out);

    exec_and_watch(docker, &ws)?;
    tmux.clear_session();
    Ok(())
}

#[cfg_attr(coverage_nightly, coverage(off))]
fn cleanup_stopped_containers(docker: &dyn DockerRunner) -> anyhow::Result<()> {
    let ws = workspace()?;
    let proj = project(&ws);
    let cids = container_id(docker, &ws)?;
    let dynamic_dir = traefik_dynamic_dir();

    for cid in &cids {
        if cid.is_empty() {
            continue;
        }
        remove_routes(cid, &proj, &dynamic_dir)?;
        // stop if still running (e.g. leftover from a previous dev:up), then rm
        let _ = docker.run(&["stop", cid]);
        run_checked(docker, &["rm", cid])
            .with_context(|| format!("failed to remove container {cid}"))?;
        let _ = writeln!(
            std::io::stdout(),
            "Cleaned up previous container: {proj} ({cid})"
        );
    }
    Ok(())
}

#[cfg_attr(coverage_nightly, coverage(off))]
fn cmd_down(docker: &dyn DockerRunner) -> anyhow::Result<()> {
    host_check()?;
    crate::deps::require(&["docker", "git"])?;

    let ws = workspace()?;
    let proj = project(&ws);
    let cids = container_id(docker, &ws)?;

    if cids.is_empty() {
        let _ = writeln!(std::io::stdout(), "No devcontainer found for {proj}");
        return Ok(());
    }

    let dynamic_dir = traefik_dynamic_dir();
    for cid in &cids {
        if cid.is_empty() {
            continue;
        }
        let image_id = docker
            .run(&["inspect", cid, "--format", "{{.Image}}"])
            .ok()
            .and_then(|o| o.stdout_str().ok())
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty());

        remove_routes(cid, &proj, &dynamic_dir)?;
        run_checked(docker, &["rm", "-f", cid])
            .with_context(|| format!("failed to remove container {cid}"))?;
        let _ = writeln!(std::io::stdout(), "Removed: {proj} ({cid})");

        // Remove the devcontainer image. Fails silently if still in use elsewhere.
        if let Some(ref img) = image_id {
            match docker.run(&["rmi", img]) {
                Ok(out) if out.exit_code == 0 => {
                    let _ = writeln!(std::io::stdout(), "Removed image: {img}");
                }
                _ => {
                    let _ = writeln!(
                        std::io::stdout(),
                        "Image not removed (in use or already gone): {img}"
                    );
                }
            }
        }
    }
    Ok(())
}

#[cfg_attr(coverage_nightly, coverage(off))]
fn cmd_exec(docker: &dyn DockerRunner) -> anyhow::Result<()> {
    host_check()?;
    crate::deps::require(&["docker", "devcontainer", "git"])?;
    let tmux = TmuxPane::from_env();
    if tmux.active() {
        crate::deps::require(&["tmux"])?;
    }

    let ws = workspace()?;
    let cid = running_container_id(docker, &ws)?;

    if cid.is_empty() {
        let _ = writeln!(std::io::stdout(), "Container not running. Starting...");
        cmd_up(docker)?;
    } else {
        let br = branch(&ws)?;
        let proj = project(&ws);
        tmux.set_session(&proj, &br, &ws);
        exec_and_watch(docker, &ws)?;
        tmux.clear_session();
    }
    Ok(())
}

struct RoutesUpdateEnv<'a> {
    managed: &'a str,
    project: &'a str,
    dynamic_dir: &'a str,
    hostname: &'a str,
    workspace: &'a str,
}

fn cmd_routes_update_inner(
    env: &RoutesUpdateEnv<'_>,
    ip_resolver: &dyn IpResolver,
    branch_resolver: &dyn GitBranchResolver,
) -> anyhow::Result<()> {
    if !routes_update_env_guard(env.managed, env.project, env.dynamic_dir) {
        tracing::warn!(
            "TRAEFIK_MANAGED/TRAEFIK_PROJECT/TRAEFIK_DYNAMIC_DIR not set; skipping routes update"
        );
        return Ok(());
    }

    if env.hostname.is_empty() {
        tracing::warn!("HOSTNAME not set; skipping routes update");
        return Ok(());
    }

    let ip = match ip_resolver.resolve() {
        Ok(s) if s.is_empty() => {
            tracing::warn!("could not resolve container IP; skipping routes update");
            return Ok(());
        }
        Ok(s) => s,
        Err(e) => {
            tracing::warn!("IP resolver error; skipping routes update: {e:#}");
            return Ok(());
        }
    };

    let branch = branch_resolver.current_branch(env.workspace)?;

    tracing::debug!(
        project = %env.project,
        hostname = %env.hostname,
        ip = %ip,
        branch = %branch,
        "routes_update: resolved IP and branch"
    );

    let dc = match read_devcontainer(env.workspace) {
        Ok(d) => d,
        Err(e) => {
            tracing::warn!("devcontainer.json not found or invalid; skipping routes update: {e:#}");
            return Ok(());
        }
    };
    if dc.ports.is_empty() {
        tracing::warn!("no portsAttributes found in devcontainer.json; skipping routes update");
        return Ok(());
    }

    write_routes(
        env.hostname,
        env.project,
        &branch,
        &ip,
        &dc.ports,
        Path::new(env.dynamic_dir),
    )?;
    Ok(())
}

fn cmd_routes_update(
    ip_resolver: &dyn IpResolver,
    branch_resolver: &dyn GitBranchResolver,
) -> anyhow::Result<()> {
    let managed = std::env::var("TRAEFIK_MANAGED").unwrap_or_default();
    let project = std::env::var("TRAEFIK_PROJECT").unwrap_or_default();
    let dynamic_dir = std::env::var("TRAEFIK_DYNAMIC_DIR").unwrap_or_default();
    let hostname = std::env::var("HOSTNAME").unwrap_or_default();
    let workspace = std::env::var("GIT_WORK_TREE")
        .or_else(|_| std::env::var("PWD"))
        .unwrap_or_else(|_| ".".to_owned());
    cmd_routes_update_inner(
        &RoutesUpdateEnv {
            managed: &managed,
            project: &project,
            dynamic_dir: &dynamic_dir,
            hostname: &hostname,
            workspace: &workspace,
        },
        ip_resolver,
        branch_resolver,
    )
}

#[cfg_attr(coverage_nightly, coverage(off))]
fn cmd_status(docker: &dyn DockerRunner) -> anyhow::Result<()> {
    host_check()?;
    crate::deps::require(&["docker"])?;
    let out = run_checked(
        docker,
        &[
            "ps",
            "--filter",
            "label=devcontainer.project",
            "--format",
            r#"table {{.ID}}\t{{.Status}}\t{{.Label "devcontainer.project"}}"#,
        ],
    )
    .context("docker ps")?;
    let _ = write!(std::io::stdout(), "{}", out.stdout_str()?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- routes_update_env_guard ---

    #[test]
    fn routes_update_guard_false_when_managed_unset() {
        assert!(!routes_update_env_guard("", "proj", "/traefik-dynamic"));
    }

    #[test]
    fn routes_update_guard_false_when_managed_not_one() {
        assert!(!routes_update_env_guard("0", "proj", "/traefik-dynamic"));
    }

    #[test]
    fn routes_update_guard_false_when_project_empty() {
        assert!(!routes_update_env_guard("1", "", "/traefik-dynamic"));
    }

    #[test]
    fn routes_update_guard_false_when_dynamic_dir_empty() {
        assert!(!routes_update_env_guard("1", "proj", ""));
    }

    #[test]
    fn routes_update_guard_true_when_all_set() {
        assert!(routes_update_env_guard("1", "proj", "/traefik-dynamic"));
    }

    #[test]
    fn strip_jsonc_removes_full_line_comments() {
        let src = "{\n  // a comment\n  \"name\": \"traefik\"\n}";
        let result = strip_jsonc_comments(src);
        assert!(!result.contains("// a comment"));
        assert!(result.contains("\"name\": \"traefik\""));
    }

    #[test]
    fn strip_jsonc_removes_inline_comments() {
        let src = "{\n  \"name\": \"traefik\"  // inline\n}";
        let result = strip_jsonc_comments(src);
        assert!(!result.contains("inline"));
        assert!(result.contains("\"name\": \"traefik\""));
    }

    #[test]
    fn strip_jsonc_inline_comment_single_space() {
        // devcontainer.json extension entries use a single space before //
        let src = "{\n  \"exts\": [\n    \"dprint.dprint\", // Formatter\n    \"rust-lang.rust-analyzer\"\n  ]\n}";
        let result = strip_jsonc_comments(src);
        assert!(!result.contains("Formatter"));
        let v: serde_json::Value = serde_json::from_str(&result).expect("valid json");
        let exts = v.get("exts").and_then(|e| e.as_array()).expect("array");
        assert_eq!(exts.first(), Some(&serde_json::json!("dprint.dprint")));
        assert_eq!(
            exts.get(1),
            Some(&serde_json::json!("rust-lang.rust-analyzer"))
        );
    }

    #[test]
    fn strip_jsonc_preserves_url_with_double_slash() {
        // // inside a string must not be stripped
        let src = r#"{ "url": "https://example.com" }"#;
        let result = strip_jsonc_comments(src);
        assert_eq!(result, src);
        let v: serde_json::Value = serde_json::from_str(&result).expect("valid json");
        assert_eq!(
            v.get("url"),
            Some(&serde_json::json!("https://example.com"))
        );
    }

    #[test]
    fn strip_jsonc_preserves_double_slash_in_string_value() {
        // a value that contains // should be left intact
        let src = r#"{ "path": "src // not a comment" }"#;
        let result = strip_jsonc_comments(src);
        let v: serde_json::Value = serde_json::from_str(&result).expect("valid json");
        assert_eq!(
            v.get("path"),
            Some(&serde_json::json!("src // not a comment"))
        );
    }

    #[test]
    fn strip_jsonc_escaped_quote_in_string() {
        // \" inside a string must not end the string state
        let src = r#"{ "msg": "say \"hi\" // still in string", "x": 1 }"#;
        let result = strip_jsonc_comments(src);
        let v: serde_json::Value = serde_json::from_str(&result).expect("valid json");
        assert_eq!(
            v.get("msg"),
            Some(&serde_json::json!("say \"hi\" // still in string"))
        );
        assert_eq!(v.get("x"), Some(&serde_json::json!(1)));
    }

    #[test]
    fn strip_jsonc_removes_block_comment() {
        let src = "{ \"a\": /* removed */ 1 }";
        let result = strip_jsonc_comments(src);
        assert!(!result.contains("removed"));
        let v: serde_json::Value = serde_json::from_str(&result).expect("valid json");
        assert_eq!(v.get("a"), Some(&serde_json::json!(1)));
    }

    #[test]
    fn strip_jsonc_removes_multiline_block_comment() {
        let src = "{\n  /* line1\n     line2 */\n  \"a\": 1\n}";
        let result = strip_jsonc_comments(src);
        assert!(!result.contains("line1"));
        assert!(!result.contains("line2"));
        // newlines inside block comment are preserved to keep line numbers stable
        assert_eq!(result.lines().count(), src.lines().count());
        let v: serde_json::Value = serde_json::from_str(&result).expect("valid json");
        assert_eq!(v.get("a"), Some(&serde_json::json!(1)));
    }

    #[test]
    fn strip_jsonc_no_comments_is_passthrough() {
        let src = r#"{ "name": "graft", "version": "0.4.1" }"#;
        assert_eq!(strip_jsonc_comments(src), src);
    }

    #[test]
    fn strip_jsonc_empty_input() {
        assert_eq!(strip_jsonc_comments(""), "");
    }

    #[test]
    fn strip_jsonc_array_with_inline_comments() {
        // mirrors the real devcontainer.json mounts section
        let src = "{\n  \"mounts\": [\n    // Claude Code\n    \"type=bind,source=~/.claude,target=/home/cuser/.claude\",\n    \"type=bind,source=~/.claude.json,target=/home/cuser/.claude.json\", // json\n    // Git\n    \"type=bind,source=~/.gitconfig,target=/home/cuser/.gitconfig\"\n  ]\n}";
        let result = strip_jsonc_comments(src);
        let v: serde_json::Value = serde_json::from_str(&result).expect("valid json");
        let mounts = v.get("mounts").and_then(|m| m.as_array()).expect("array");
        assert_eq!(mounts.len(), 3);
    }

    #[test]
    fn strip_jsonc_comment_at_eof_without_newline() {
        let src = "{ \"a\": 1 } // trailing";
        let result = strip_jsonc_comments(src);
        assert!(!result.contains("trailing"));
        // must still be parseable
        serde_json::from_str::<serde_json::Value>(&result).expect("valid json");
    }

    #[test]
    fn read_devcontainer_parses_ports_and_uid() {
        let dir = tempfile::tempdir().expect("tempdir");
        let dc_dir = dir.path().join(".devcontainer");
        std::fs::create_dir_all(&dc_dir).expect("create .devcontainer");
        let json = r#"{
  "portsAttributes": {
    "5080": { "label": "OpenObserve" },
    "8080": { "label": "App" }
  },
  "build": {
    "args": { "USER_UID": "1001" }
  },
  "remoteUser": "devuser"
}"#;
        std::fs::write(dc_dir.join("devcontainer.json"), json).expect("write devcontainer.json");

        let meta = read_devcontainer(dir.path().to_str().expect("path to str"))
            .expect("read_devcontainer");
        assert_eq!(meta.ports.len(), 2);
        assert!(meta.ports.contains(&"5080".to_owned()));
        assert!(meta.ports.contains(&"8080".to_owned()));
        assert_eq!(meta.container_uid, "1001");
        assert_eq!(meta.user_name, "devuser");
    }

    #[test]
    fn read_devcontainer_user_name_defaults_when_absent() {
        let dir = tempfile::tempdir().expect("tempdir");
        let dc_dir = dir.path().join(".devcontainer");
        std::fs::create_dir_all(&dc_dir).expect("create .devcontainer");
        let json = r#"{ "build": { "args": { "USER_UID": "1000" } } }"#;
        std::fs::write(dc_dir.join("devcontainer.json"), json).expect("write devcontainer.json");

        let meta = read_devcontainer(dir.path().to_str().expect("path to str"))
            .expect("read_devcontainer");
        assert_eq!(meta.user_name, "user");
    }

    #[test]
    fn workspace_hash_known_value() {
        // Pin the FNV-1a output so accidental constant changes are caught.
        assert_eq!(
            workspace_hash("/home/user/projects/graft"),
            0x7fde_6d76_91cb_6c99
        );
    }

    #[test]
    fn workspace_hash_differs_for_different_paths() {
        let h1 = workspace_hash("/home/user/projects/graft");
        let h2 = workspace_hash("/home/user/projects/other");
        assert_ne!(h1, h2);
    }

    #[test]
    fn session_acquire_creates_lock_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let lock = acquire_session_in(dir.path()).expect("acquire");
        assert!(lock.exists());
    }

    #[test]
    fn session_release_is_last_when_sole_session() {
        let dir = tempfile::tempdir().expect("tempdir");
        let lock = acquire_session_in(dir.path()).expect("acquire");
        let is_last = release_session_in(&lock, dir.path());
        assert!(is_last);
        assert!(!lock.exists());
    }

    #[test]
    fn session_not_last_when_another_live_session_exists() {
        let dir = tempfile::tempdir().expect("tempdir");
        let my_lock = acquire_session_in(dir.path()).expect("acquire");
        // PID 1 (init) always exists on Linux
        let other_lock = dir.path().join("1");
        std::fs::write(&other_lock, b"").expect("write");
        let is_last = release_session_in(&my_lock, dir.path());
        assert!(!is_last, "PID 1 is alive so we are not last");
        let _ = std::fs::remove_file(&other_lock);
    }

    #[test]
    fn session_stale_lock_ignored() {
        let dir = tempfile::tempdir().expect("tempdir");
        // PID 9999999 almost certainly does not exist
        std::fs::write(dir.path().join("9999999"), b"").expect("write stale");
        let lock = acquire_session_in(dir.path()).expect("acquire");
        let is_last = release_session_in(&lock, dir.path());
        assert!(is_last, "stale PID should not count as live session");
    }

    #[test]
    fn session_dir_in_uses_hex_hash_suffix() {
        let base = std::path::Path::new("/tmp");
        let dir = session_dir_in(base, "/workspace/foo");
        let name = dir.file_name().unwrap().to_str().unwrap();
        // Directory name must start with the prefix and be followed by a
        // 16-hex-digit FNV-1a hash so that it is both human-readable and
        // stable across runs.
        assert!(name.starts_with("graft-denv-"), "unexpected prefix: {name}");
        let suffix = &name["graft-denv-".len()..];
        assert_eq!(suffix.len(), 16, "expected 16 hex digits, got: {suffix}");
        assert!(
            suffix.chars().all(|c| c.is_ascii_hexdigit()),
            "non-hex: {suffix}"
        );
    }

    #[test]
    fn tmux_pane_inactive_when_tmux_env_empty() {
        let pane = TmuxPane::new("", "%31");
        assert!(!pane.active());
    }

    #[test]
    fn tmux_pane_active_when_tmux_env_set() {
        let pane = TmuxPane::new("/tmp/tmux-1000/default,12345,0", "%31");
        assert!(pane.active());
    }

    #[test]
    fn tmux_pane_socket_path_extracted() {
        let pane = TmuxPane::new("/tmp/tmux-1000/default,12345,0", "%31");
        assert_eq!(pane.socket_path(), Some("/tmp/tmux-1000/default"));
    }

    #[test]
    fn tmux_pane_socket_path_none_when_inactive() {
        let pane = TmuxPane::new("", "%31");
        assert_eq!(pane.socket_path(), None);
    }

    #[test]
    fn tmux_pane_pane_id_returns_none_when_empty() {
        let pane = TmuxPane::new("/tmp/tmux-1000/default,12345,0", "");
        assert_eq!(pane.pane_id(), None);
    }

    #[test]
    fn tmux_pane_pane_id_returns_value() {
        let pane = TmuxPane::new("/tmp/tmux-1000/default,12345,0", "%31");
        assert_eq!(pane.pane_id(), Some("%31"));
    }

    #[test]
    fn tmux_pane_env_value() {
        let env = "/tmp/tmux-1000/default,12345,0";
        let pane = TmuxPane::new(env, "%31");
        assert_eq!(pane.env_value(), env);
    }

    // --- Cycle 3: IpResolver + GitBranchResolver ---

    struct MockIpResolver(anyhow::Result<String>);
    impl runner::IpResolver for MockIpResolver {
        fn resolve(&self) -> anyhow::Result<String> {
            match &self.0 {
                Ok(s) => Ok(s.clone()),
                Err(e) => anyhow::bail!("{e}"),
            }
        }
    }

    struct MockBranchResolver(anyhow::Result<String>);
    impl runner::GitBranchResolver for MockBranchResolver {
        fn current_branch(&self, _workspace: &str) -> anyhow::Result<String> {
            match &self.0 {
                Ok(s) => Ok(s.clone()),
                Err(e) => anyhow::bail!("{e}"),
            }
        }
    }

    fn env_all_set<'a>() -> RoutesUpdateEnv<'a> {
        RoutesUpdateEnv {
            managed: "1",
            project: "testproj",
            dynamic_dir: "/traefik-dynamic",
            hostname: "abc123def456",
            workspace: "/workspace",
        }
    }

    fn make_workspace_with_ports(ports: &[&str]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        let dc_dir = dir.path().join(".devcontainer");
        std::fs::create_dir_all(&dc_dir).expect("create .devcontainer");
        let ports_json: String = ports
            .iter()
            .map(|p| format!("    \"{p}\": {{\"label\": \"{p}\"}}"))
            .collect::<Vec<_>>()
            .join(",\n");
        let json = format!("{{\n  \"portsAttributes\": {{\n{ports_json}\n  }}\n}}");
        std::fs::write(dc_dir.join("devcontainer.json"), json).expect("write devcontainer.json");
        dir
    }

    #[test]
    fn routes_update_inner_ok_with_valid_ip_and_branch() {
        let ws = make_workspace_with_ports(&["8080"]);
        let dyn_dir = tempfile::tempdir().expect("dynamic tempdir");
        let env = RoutesUpdateEnv {
            managed: "1",
            project: "testproj",
            dynamic_dir: dyn_dir.path().to_str().expect("path"),
            hostname: "abc123def456789",
            workspace: ws.path().to_str().expect("path"),
        };
        let result = cmd_routes_update_inner(
            &env,
            &MockIpResolver(Ok("172.20.0.2".to_owned())),
            &MockBranchResolver(Ok("main".to_owned())),
        );
        assert!(result.is_ok());
    }

    #[test]
    fn routes_update_inner_writes_yaml_with_correct_branch() {
        let ws = make_workspace_with_ports(&["5080", "8080"]);
        let dyn_dir = tempfile::tempdir().expect("dynamic tempdir");
        let env = RoutesUpdateEnv {
            managed: "1",
            project: "testproj",
            dynamic_dir: dyn_dir.path().to_str().expect("path"),
            hostname: "abc123def456789",
            workspace: ws.path().to_str().expect("path"),
        };

        let result = cmd_routes_update_inner(
            &env,
            &MockIpResolver(Ok("172.20.0.2".to_owned())),
            &MockBranchResolver(Ok("feature-foo".to_owned())),
        );
        assert!(result.is_ok(), "cmd_routes_update_inner failed: {result:?}");

        let expected_path = dyn_dir.path().join("testproj-abc123def456.yml");
        assert!(
            expected_path.exists(),
            "YAML file not found: {}",
            expected_path.display()
        );

        let content = std::fs::read_to_string(&expected_path).expect("read yaml");
        assert!(
            content.contains("feature-foo"),
            "branch not in YAML: {content}"
        );
        assert!(content.contains("172.20.0.2"), "IP not in YAML: {content}");
        assert!(content.contains("5080"), "port 5080 not in YAML: {content}");
        assert!(content.contains("8080"), "port 8080 not in YAML: {content}");
    }

    #[test]
    fn routes_update_inner_warns_on_ip_resolver_err() {
        // IP resolver failure is non-fatal: warn + Ok(())
        let result = cmd_routes_update_inner(
            &env_all_set(),
            &MockIpResolver(Err(anyhow::anyhow!("ip-resolver-failed"))),
            &MockBranchResolver(Ok("main".to_owned())),
        );
        assert!(result.is_ok());
    }

    #[test]
    fn routes_update_inner_warns_on_missing_devcontainer() {
        // devcontainer.json absent is non-fatal: warn + Ok(())
        let dyn_dir = tempfile::tempdir().expect("dynamic tempdir");
        let ws_dir = tempfile::tempdir().expect("ws tempdir"); // no .devcontainer here
        let env = RoutesUpdateEnv {
            managed: "1",
            project: "testproj",
            dynamic_dir: dyn_dir.path().to_str().expect("path"),
            hostname: "abc123def456",
            workspace: ws_dir.path().to_str().expect("path"),
        };
        let result = cmd_routes_update_inner(
            &env,
            &MockIpResolver(Ok("172.20.0.2".to_owned())),
            &MockBranchResolver(Ok("main".to_owned())),
        );
        assert!(result.is_ok());
    }

    #[test]
    fn routes_update_inner_errors_on_write_routes_failure() {
        // write_routes failure is fatal: Err propagated
        let ws = make_workspace_with_ports(&["8080"]);
        let env = RoutesUpdateEnv {
            managed: "1",
            project: "testproj",
            dynamic_dir: "/nonexistent-dir-that-cannot-be-written",
            hostname: "abc123def456",
            workspace: ws.path().to_str().expect("path"),
        };
        let result = cmd_routes_update_inner(
            &env,
            &MockIpResolver(Ok("172.20.0.2".to_owned())),
            &MockBranchResolver(Ok("main".to_owned())),
        );
        assert!(result.is_err());
    }

    #[test]
    fn routes_update_inner_calls_branch_resolver() {
        let result = cmd_routes_update_inner(
            &env_all_set(),
            &MockIpResolver(Ok("172.20.0.2".to_owned())),
            &MockBranchResolver(Err(anyhow::anyhow!("branch-resolver-called"))),
        );
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("branch-resolver-called")
        );
    }

    #[test]
    fn routes_update_inner_skips_when_ip_empty() {
        let result = cmd_routes_update_inner(
            &env_all_set(),
            &MockIpResolver(Ok(String::new())),
            &MockBranchResolver(Err(anyhow::anyhow!("should-not-be-called"))),
        );
        assert!(result.is_ok());
    }

    #[test]
    fn routes_update_inner_skips_when_guard_fails() {
        let env = RoutesUpdateEnv {
            managed: "0",
            project: "testproj",
            dynamic_dir: "/traefik-dynamic",
            hostname: "abc123",
            workspace: "/workspace",
        };
        let result = cmd_routes_update_inner(
            &env,
            &MockIpResolver(Err(anyhow::anyhow!("should-not-be-called"))),
            &MockBranchResolver(Err(anyhow::anyhow!("should-not-be-called"))),
        );
        assert!(result.is_ok());
    }

    #[test]
    fn routes_update_inner_skips_when_hostname_empty() {
        let env = RoutesUpdateEnv {
            managed: "1",
            project: "testproj",
            dynamic_dir: "/traefik-dynamic",
            hostname: "",
            workspace: "/workspace",
        };
        let result = cmd_routes_update_inner(
            &env,
            &MockIpResolver(Err(anyhow::anyhow!("should-not-be-called"))),
            &MockBranchResolver(Err(anyhow::anyhow!("should-not-be-called"))),
        );
        assert!(result.is_ok());
    }
}
