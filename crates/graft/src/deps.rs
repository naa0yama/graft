pub fn require(cmds: &[&str]) -> anyhow::Result<()> {
    let missing: Vec<&str> = cmds.iter().copied().filter(|&c| !available(c)).collect();
    if missing.is_empty() {
        return Ok(());
    }
    anyhow::bail!(
        "required command(s) not found in PATH: {}\nInstall the missing tool(s) and retry.",
        missing.join(", ")
    )
}

fn available(cmd: &str) -> bool {
    std::process::Command::new("sh")
        .args(["-c", &format!("command -v {cmd}")])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg_attr(miri, ignore)]
    fn require_existing_command() {
        assert!(require(&["sh"]).is_ok());
    }

    #[test]
    #[cfg_attr(miri, ignore)]
    fn require_missing_command() {
        let err = require(&["__graft_nonexistent_cmd__"]).unwrap_err();
        assert!(err.to_string().contains("__graft_nonexistent_cmd__"));
    }

    #[test]
    #[cfg_attr(miri, ignore)]
    fn require_mixed_lists_all_missing_reported() {
        let err = require(&["sh", "__missing_a__", "__missing_b__"]).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("__missing_a__"));
        assert!(msg.contains("__missing_b__"));
        assert!(!msg.contains("sh"));
    }
}
