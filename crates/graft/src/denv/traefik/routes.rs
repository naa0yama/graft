//! Traefik file-provider route YAML helpers.
#![allow(clippy::module_name_repetitions)]
use std::fmt::Write as _;
use std::io::Write as _;
use std::path::Path;

use anyhow::Context as _;

/// Normalize a branch name to a DNS-safe label:
/// lowercase, non-alphanumeric/hyphen replaced with `-`, deduplicated,
/// leading/trailing hyphens stripped, truncated to 63 chars.
pub fn normalize_branch(raw: &str) -> String {
    let s: String = raw
        .to_lowercase()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let s = s.trim_matches('-').to_owned();
    // collapse consecutive hyphens
    let mut result = String::with_capacity(s.len());
    let mut prev_hyphen = false;
    for c in s.chars() {
        if c == '-' {
            if !prev_hyphen {
                result.push(c);
            }
            prev_hyphen = true;
        } else {
            result.push(c);
            prev_hyphen = false;
        }
    }
    result.truncate(63);
    result.trim_end_matches('-').to_owned()
}

/// Write a Traefik file-provider YAML for all ports of one container.
///
/// # Errors
///
/// Returns an error if the file cannot be written.
pub fn write_routes(
    cid: &str,
    project: &str,
    branch: &str,
    ip: &str,
    ports: &[String],
    dynamic_dir: &Path,
) -> anyhow::Result<()> {
    let cid_short = &cid[..cid.len().min(12)];
    let dest = dynamic_dir.join(format!("{project}-{cid_short}.yml"));

    let mut content = String::from("http:\n  routers:\n");
    for port in ports {
        if port.is_empty() {
            continue;
        }
        let router = format!("p{port}-{branch}--{project}");
        let fqdn = format!("p{port}.{branch}.{project}.localhost");
        let _ = write!(
            content,
            "    {router}:\n      rule: \"Host(`{fqdn}`)\"\n      entryPoints:\n        - web\n      service: {router}\n"
        );
    }
    content.push_str("  services:\n");
    for port in ports {
        if port.is_empty() {
            continue;
        }
        let router = format!("p{port}-{branch}--{project}");
        let _ = write!(
            content,
            "    {router}:\n      loadBalancer:\n        servers:\n          - url: \"http://{ip}:{port}\"\n"
        );
    }

    std::fs::write(&dest, content).with_context(|| format!("write routes {}", dest.display()))?;
    let _ = writeln!(std::io::stdout(), "Wrote {}", dest.display());
    Ok(())
}

/// Remove the file-provider YAML for a container.
///
/// # Errors
///
/// Returns an error if the file cannot be removed.
pub fn remove_routes(cid: &str, project: &str, dynamic_dir: &Path) -> anyhow::Result<()> {
    let cid_short = &cid[..cid.len().min(12)];
    let dest = dynamic_dir.join(format!("{project}-{cid_short}.yml"));
    if dest.exists() {
        std::fs::remove_file(&dest).with_context(|| format!("remove {}", dest.display()))?;
        let _ = writeln!(std::io::stdout(), "Removed {}", dest.display());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_branch_lowercase() {
        assert_eq!(normalize_branch("Main"), "main");
    }

    #[test]
    fn normalize_branch_special_chars() {
        assert_eq!(normalize_branch("feature/my-branch"), "feature-my-branch");
    }

    #[test]
    fn normalize_branch_consecutive_hyphens() {
        assert_eq!(normalize_branch("feat//test"), "feat-test");
    }

    #[test]
    fn normalize_branch_leading_trailing() {
        assert_eq!(normalize_branch("/main/"), "main");
    }

    #[test]
    fn normalize_branch_truncate_63() {
        let long = "a".repeat(70);
        let result = normalize_branch(&long);
        assert_eq!(result.len(), 63);
    }

    #[test]
    fn write_routes_generates_correct_yaml() {
        let dir = tempfile::tempdir().expect("tempdir");
        let cid = "abc123def456789";
        let ports = vec!["5080".to_owned(), "8080".to_owned()];
        write_routes(cid, "myproject", "main", "172.20.0.2", &ports, dir.path())
            .expect("write_routes");

        let path = dir.path().join("myproject-abc123def456.yml");
        let content = std::fs::read_to_string(&path).expect("read routes");
        assert!(content.contains("p5080.main.myproject.localhost"));
        assert!(content.contains("http://172.20.0.2:5080"));
        assert!(content.contains("p8080.main.myproject.localhost"));
    }

    #[test]
    fn remove_routes_deletes_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let cid = "abc123def456789";
        let ports = vec!["5080".to_owned()];
        write_routes(cid, "proj", "main", "172.20.0.2", &ports, dir.path()).expect("write_routes");

        let path = dir.path().join("proj-abc123def456.yml");
        assert!(path.exists());

        remove_routes(cid, "proj", dir.path()).expect("remove_routes");
        assert!(!path.exists());
    }
}
