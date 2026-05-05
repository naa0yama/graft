/// Traefik devcontainer lifecycle management.
pub mod cli;
pub mod config;
pub mod routes;
pub mod runner;

use std::io::Write as _;
use std::path::Path;
use std::process::ExitCode;

use anyhow::Context as _;

use cli::{TraefikArgs, TraefikCommand};
use config::{
    TRAEFIK_PORT_DASHBOARD, TRAEFIK_PORT_ROUTER, traefik_bin, traefik_config, traefik_dynamic_dir,
    traefik_service, write_systemd_unit, write_traefik_yml,
};
use routes::{normalize_branch, remove_routes, write_routes};
use runner::{DockerRunner, SystemDockerRunner, run_checked};

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

    fn set(&self, option: &str, value: &str) {
        if !self.active() {
            return;
        }
        let _ = std::process::Command::new("tmux")
            .args(["set-option", "-p", option, value])
            .status();
    }

    fn clear(&self, option: &str) {
        self.set(option, "");
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
fn traefik_latest() -> anyhow::Result<String> {
    let out = std::process::Command::new("curl")
        .args([
            "-sfSL",
            "--retry",
            "3",
            "https://api.github.com/repos/traefik/traefik/releases/latest",
        ])
        .output()
        .context("curl github API")?;
    if !out.status.success() {
        anyhow::bail!("curl failed: {}", String::from_utf8_lossy(&out.stderr));
    }
    let v: serde_json::Value =
        serde_json::from_slice(&out.stdout).context("parse github releases JSON")?;
    v.get("tag_name")
        .and_then(|t| t.as_str())
        .map(ToOwned::to_owned)
        .context("tag_name not found in github releases response")
}

#[cfg_attr(coverage_nightly, coverage(off))]
fn traefik_install_version(version: &str) -> anyhow::Result<()> {
    let arch = match std::env::consts::ARCH {
        "x86_64" => "amd64",
        "aarch64" => "arm64",
        a => anyhow::bail!("unsupported architecture: {a}"),
    };
    let filename = format!("traefik_{version}_linux_{arch}.tar.gz");
    let url = format!("https://github.com/traefik/traefik/releases/download/{version}/{filename}");
    let checksums_url = format!(
        "https://github.com/traefik/traefik/releases/download/{version}/traefik_{version}_checksums.txt"
    );

    let tmpfile = tempfile::NamedTempFile::new().context("create temp file")?;
    let checksums_file = tempfile::NamedTempFile::new().context("create checksums temp file")?;
    let tmpfile_path = tmpfile
        .path()
        .to_str()
        .context("temp file path not UTF-8")?
        .to_owned();
    let checksums_path = checksums_file
        .path()
        .to_str()
        .context("checksums temp file path not UTF-8")?
        .to_owned();

    tracing::info!("Downloading traefik {version} ({arch})...");
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
            &url,
        ])
        .status()
        .context("curl download")?;
    anyhow::ensure!(status.success(), "failed to download traefik {version}");

    tracing::info!("Verifying checksum...");
    let status = std::process::Command::new("curl")
        .args([
            "-fSL",
            "--retry",
            "3",
            "--retry-delay",
            "2",
            "--retry-connrefused",
            "-o",
            &checksums_path,
            &checksums_url,
        ])
        .status()
        .context("curl checksums")?;
    anyhow::ensure!(status.success(), "failed to download checksums file");

    let checksums =
        std::fs::read_to_string(checksums_file.path()).context("read checksums file")?;
    let expected = checksums
        .lines()
        .find(|l| l.ends_with(&filename))
        .and_then(|l| l.split_whitespace().next())
        .with_context(|| format!("{filename} not found in checksums"))?
        .to_owned();

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
        expected == actual,
        "checksum mismatch (expected {expected}, got {actual})"
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
    match installed {
        None => {
            traefik_install_version(&latest)?;
            Ok("installed")
        }
        Some(v) if v != latest => {
            tracing::info!("Updating traefik {v} -> {latest}");
            traefik_install_version(&latest)?;
            Ok("updated")
        }
        _ => {
            tracing::info!(
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
// Interactive exec helper
// ---------------------------------------------------------------------------

#[cfg_attr(coverage_nightly, coverage(off))]
fn exec_and_watch(docker: &dyn DockerRunner, workspace: &str) -> anyhow::Result<()> {
    std::process::Command::new("devcontainer")
        .args(["exec", "--workspace-folder", workspace, "bash"])
        .status()
        .context("devcontainer exec bash")?;

    let exited_cid = running_container_id(docker, workspace).unwrap_or_default();
    let exited_project = project(workspace);

    let _ = writeln!(
        std::io::stdout(),
        "Shell exited. Container stopping in 10s... (mise run dev:exec to reconnect)"
    );

    if !exited_cid.is_empty() {
        let dynamic_dir = traefik_dynamic_dir();
        let routes_file = dynamic_dir.join(format!(
            "{exited_project}-{}.yml",
            &exited_cid[..exited_cid.len().min(12)]
        ));
        let routes_path = routes_file.to_string_lossy().into_owned();
        // Spawn background cleanup without waiting — the sh process detaches after 10s.
        std::process::Command::new("sh")
            .arg("-c")
            .arg(format!(
                "sleep 10 && rm -f {routes_path} && docker rm -f {exited_cid} > /dev/null 2>&1 || true"
            ))
            .spawn()
            .context("spawn background cleanup")?;
    }
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

    cmd_down(docker)?;
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
        let target = format!("/home/{user_name}/.gnupg/S.gpg-agent");
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
        remove_routes(cid, &proj, &dynamic_dir)?;
        run_checked(docker, &["rm", "-f", cid])
            .with_context(|| format!("failed to remove container {cid}"))?;
        let _ = writeln!(std::io::stdout(), "Stopped: {proj} ({cid})");
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
}
