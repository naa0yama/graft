//! Traefik binary and configuration path helpers.
#![allow(clippy::module_name_repetitions)]
use std::io::Write as _;
use std::path::Path;

use anyhow::Context as _;

/// Traefik HTTP router port.
pub const TRAEFIK_PORT_ROUTER: u16 = 8080;
/// Traefik dashboard port.
pub const TRAEFIK_PORT_DASHBOARD: u16 = 8081;

/// Returns the path to the Traefik binary.
#[cfg_attr(coverage_nightly, coverage(off))]
pub fn traefik_bin() -> std::path::PathBuf {
    home_dir().join(".local/bin/traefik")
}

/// Returns the path to the Traefik static configuration file.
#[cfg_attr(coverage_nightly, coverage(off))]
pub fn traefik_config() -> std::path::PathBuf {
    home_dir().join(".config/traefik/traefik.yml")
}

/// Returns the path to the Traefik dynamic configuration directory.
#[cfg_attr(coverage_nightly, coverage(off))]
pub fn traefik_dynamic_dir() -> std::path::PathBuf {
    home_dir().join(".config/traefik/dynamic")
}

/// Returns the path to the Traefik systemd user service file.
#[cfg_attr(coverage_nightly, coverage(off))]
pub fn traefik_service() -> std::path::PathBuf {
    home_dir().join(".config/systemd/user/traefik.service")
}

#[cfg_attr(coverage_nightly, coverage(off))]
fn home_dir() -> std::path::PathBuf {
    std::env::var("HOME").map_or_else(
        |_| std::path::PathBuf::from("/tmp"),
        std::path::PathBuf::from,
    )
}

/// Writes the static Traefik configuration YAML to `config_path`.
///
/// # Errors
///
/// Returns an error if the parent directory cannot be created or the file cannot be written.
pub fn write_traefik_yml(config_path: &Path, dynamic_dir: &Path) -> anyhow::Result<()> {
    let content = format!(
        "\
entryPoints:
  web:
    address: \":{port_router}\"
  traefik:
    address: \":{port_dashboard}\"
providers:
  docker:
    endpoint: \"unix:///var/run/docker.sock\"
    exposedByDefault: false
    network: devcontainer-traefik
  file:
    directory: \"{dynamic_dir}\"
    watch: true
api:
  dashboard: true
  insecure: true
",
        port_router = TRAEFIK_PORT_ROUTER,
        port_dashboard = TRAEFIK_PORT_DASHBOARD,
        dynamic_dir = dynamic_dir.display(),
    );
    if let Some(parent) = config_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create dir {}", parent.display()))?;
    }
    std::fs::write(config_path, content)
        .with_context(|| format!("write {}", config_path.display()))?;
    let _ = writeln!(std::io::stdout(), "Wrote {}", config_path.display());
    Ok(())
}

/// Writes the systemd user service unit file for Traefik.
///
/// # Errors
///
/// Returns an error if the parent directory cannot be created or the file cannot be written.
pub fn write_systemd_unit(
    bin_path: &Path,
    config_path: &Path,
    service_path: &Path,
) -> anyhow::Result<()> {
    let content = format!(
        "\
[Unit]
Description=Traefik reverse proxy for devcontainers
After=network.target

[Service]
ExecStart={bin} --configfile={config}
Restart=on-failure
RestartSec=5

[Install]
WantedBy=default.target
",
        bin = bin_path.display(),
        config = config_path.display(),
    );
    if let Some(parent) = service_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create dir {}", parent.display()))?;
    }
    std::fs::write(service_path, content)
        .with_context(|| format!("write {}", service_path.display()))?;
    let _ = writeln!(std::io::stdout(), "Wrote {}", service_path.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_traefik_yml_contains_expected_fields() {
        let dir = tempfile::tempdir().expect("tempdir");
        let config = dir.path().join("traefik.yml");
        let dynamic = dir.path().join("dynamic");
        write_traefik_yml(&config, &dynamic).expect("write_traefik_yml");

        let content = std::fs::read_to_string(&config).expect("read config");
        assert!(content.contains("address: \":8080\""));
        assert!(content.contains("address: \":8081\""));
        assert!(content.contains("unix:///var/run/docker.sock"));
        assert!(content.contains("devcontainer-traefik"));
        assert!(content.contains(dynamic.to_str().expect("path to str")));
        assert!(content.contains("dashboard: true"));
    }

    #[test]
    fn write_systemd_unit_contains_expected_fields() {
        let dir = tempfile::tempdir().expect("tempdir");
        let bin = Path::new("/usr/local/bin/traefik");
        let config = dir.path().join("traefik.yml");
        let service = dir.path().join("traefik.service");
        write_systemd_unit(bin, &config, &service).expect("write_systemd_unit");

        let content = std::fs::read_to_string(&service).expect("read service");
        assert!(content.contains("ExecStart=/usr/local/bin/traefik"));
        assert!(content.contains("Restart=on-failure"));
        assert!(content.contains("WantedBy=default.target"));
    }
}
