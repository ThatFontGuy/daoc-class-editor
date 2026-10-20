//! Talks to the Offline DAoC server either over plain `ssh`/`scp` (shelling out to the system
//! binaries, reusing whatever SSH config/keys/agent the user already has) or, when the server
//! runs on the same machine as this app, directly against the local filesystem - no SSH at all.
//!
//! Which mode applies is decided by one thing: whether `host` is set. Leave it blank and
//! `server_dir` is treated as a local path; set it and everything goes over SSH/SCP instead.
//! Both macOS and modern Windows ship an OpenSSH client by default, so the SSH path needs no
//! extra native-library cross-compilation work for the Windows build.

use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SshConfig {
    /// Leave blank to use `server_dir` as a local path on this machine instead of SSH.
    pub host: String,
    pub user: String,
    pub port: u16,
    /// Path to a private key file; empty means "use the default SSH agent/keys." Ignored in
    /// local mode.
    pub identity_file: String,
    /// The server's own root folder (containing scripts/, data/, config/) - on the remote
    /// machine if `host` is set, or directly on this machine if it's blank.
    pub remote_server_dir: String,
    /// The server start/stop script's filename - configurable since it can be renamed
    /// (daoc-server.sh by default, but a server owner may have renamed it, e.g. startOthlyn.sh).
    pub start_script_name: String,
}

fn is_local(cfg: &SshConfig) -> bool {
    cfg.host.trim().is_empty()
}

fn config_path() -> PathBuf {
    let base = dirs::config_dir().unwrap_or_else(std::env::temp_dir);
    let dir = base.join("daoc-class-editor");
    let _ = std::fs::create_dir_all(&dir);
    dir.join("ssh-config.json")
}

pub fn load_config() -> SshConfig {
    std::fs::read_to_string(config_path())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save_config(cfg: &SshConfig) -> Result<(), String> {
    let json = serde_json::to_string_pretty(cfg).map_err(|e| e.to_string())?;
    std::fs::write(config_path(), json).map_err(|e| e.to_string())
}

fn ssh_base_args(cfg: &SshConfig) -> Vec<String> {
    let mut args = vec!["-p".to_string(), cfg.port.to_string()];
    if !cfg.identity_file.is_empty() {
        args.push("-i".to_string());
        args.push(cfg.identity_file.clone());
    }
    args.push("-o".to_string());
    args.push("BatchMode=yes".to_string());
    args.push("-o".to_string());
    args.push("ConnectTimeout=10".to_string());
    args
}

fn ssh_base_scp_args(cfg: &SshConfig) -> Vec<String> {
    let mut args = vec!["-P".to_string(), cfg.port.to_string()];
    if !cfg.identity_file.is_empty() {
        args.push("-i".to_string());
        args.push(cfg.identity_file.clone());
    }
    args
}

fn run_command(mut cmd: Command) -> Result<String, String> {
    let output = cmd.output().map_err(|e| format!("failed to run: {e}"))?;
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    if output.status.success() {
        Ok(if stdout.trim().is_empty() {
            stderr
        } else {
            stdout
        })
    } else {
        Err(if stderr.trim().is_empty() {
            format!("command failed (exit {:?}): {stdout}", output.status.code())
        } else {
            stderr
        })
    }
}

fn tempfile_path(filename: &str) -> PathBuf {
    std::env::temp_dir().join(format!("daoc-class-editor-{filename}"))
}

fn shell_escape(s: &str) -> String {
    s.replace('\'', "'\"'\"'")
}

/// Validates the configuration before anything else: in SSH mode, `ssh host "echo connected"`;
/// in local mode, just confirms the server folder actually exists and looks right.
pub fn test_connection(cfg: &SshConfig) -> Result<String, String> {
    if is_local(cfg) {
        let dir = Path::new(&cfg.remote_server_dir);
        if !dir.is_dir() {
            return Err(format!(
                "'{}' is not a folder on this machine",
                cfg.remote_server_dir
            ));
        }
        let script = dir.join(&cfg.start_script_name);
        if !script.is_file() {
            return Err(format!(
                "found the folder, but no '{}' inside it - check the start/stop script filename",
                cfg.start_script_name
            ));
        }
        return Ok(format!("local folder OK: {}", dir.display()));
    }
    let mut cmd = Command::new("ssh");
    cmd.args(ssh_base_args(cfg))
        .arg(format!("{}@{}", cfg.user, cfg.host))
        .arg("echo connected && pwd");
    run_command(cmd)
}

/// Writes `content` as `<server_dir>/scripts/<filename>`, either directly on disk (local mode)
/// or via a temp file + `scp` (SSH mode).
pub fn deploy_class_file(cfg: &SshConfig, filename: &str, content: &str) -> Result<String, String> {
    if is_local(cfg) {
        let scripts_dir = Path::new(&cfg.remote_server_dir).join("scripts");
        std::fs::create_dir_all(&scripts_dir)
            .map_err(|e| format!("could not create {}: {e}", scripts_dir.display()))?;
        let target = scripts_dir.join(filename);
        std::fs::write(&target, content)
            .map_err(|e| format!("could not write {}: {e}", target.display()))?;
        return Ok(format!("wrote {}", target.display()));
    }

    let mut tmp = tempfile_path(filename);
    {
        let mut f = std::fs::File::create(&tmp).map_err(|e| e.to_string())?;
        f.write_all(content.as_bytes()).map_err(|e| e.to_string())?;
    }
    // Make sure the remote scripts/ folder exists first (the server also auto-creates it on
    // startup, but a stopped server that has never run with EnableCompilation won't have it yet).
    let mkdir_result = {
        let mut cmd = Command::new("ssh");
        cmd.args(ssh_base_args(cfg))
            .arg(format!("{}@{}", cfg.user, cfg.host))
            .arg(format!(
                "mkdir -p '{}/scripts'",
                shell_escape(&cfg.remote_server_dir)
            ));
        run_command(cmd)
    };
    if let Err(e) = mkdir_result {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("could not prepare remote scripts/ folder: {e}"));
    }

    let mut cmd = Command::new("scp");
    cmd.args(ssh_base_scp_args(cfg));
    cmd.arg(&tmp);
    cmd.arg(format!(
        "{}@{}:{}/scripts/{}",
        cfg.user, cfg.host, cfg.remote_server_dir, filename
    ));
    let result = run_command(cmd);
    let _ = std::fs::remove_file(&tmp);
    tmp.pop();
    result
}

/// Runs one SQL statement against the server's live database, via the local `sqlite3` CLI
/// (local mode) or that same CLI run over SSH (SSH mode). Intended for the small, well-known
/// rows this tool manages (ServerProperty, a trainer NPC spawn, starter equipment, a startup
/// location) - not general-purpose SQL.
pub fn run_remote_sql(cfg: &SshConfig, sql: &str) -> Result<String, String> {
    let db_path = Path::new(&cfg.remote_server_dir).join("data/opendaoc.sqlite3.db");
    if is_local(cfg) {
        // Passed as separate argv entries, not a shell string, so no escaping is needed at all.
        let mut cmd = Command::new("sqlite3");
        cmd.arg(&db_path).arg(sql);
        return run_command(cmd);
    }
    let remote_cmd = format!(
        "sqlite3 '{}' '{}'",
        shell_escape(&db_path.to_string_lossy()),
        shell_escape(sql)
    );
    let mut cmd = Command::new("ssh");
    cmd.args(ssh_base_args(cfg))
        .arg(format!("{}@{}", cfg.user, cfg.host))
        .arg(remote_cmd);
    run_command(cmd)
}

/// Checks the server is actually stopped before a database write, and reports if something is
/// still holding the file (mirrors the manual lsof/fuser check this project's own docs recommend).
pub fn remote_server_status(cfg: &SshConfig) -> Result<String, String> {
    remote_server_control(cfg, "status")
}

pub fn remote_server_control(cfg: &SshConfig, action: &str) -> Result<String, String> {
    if action != "start" && action != "stop" && action != "status" {
        return Err(format!(
            "unknown action '{action}' (expected start, stop, or status)"
        ));
    }
    if is_local(cfg) {
        let script = Path::new(&cfg.remote_server_dir).join(&cfg.start_script_name);
        let mut cmd = Command::new(&script);
        cmd.current_dir(&cfg.remote_server_dir).arg(action);
        return run_command(cmd);
    }
    let remote_cmd = format!(
        "cd '{}' && ./{} {}",
        shell_escape(&cfg.remote_server_dir),
        shell_escape(&cfg.start_script_name),
        action
    );
    let mut cmd = Command::new("ssh");
    cmd.args(ssh_base_args(cfg))
        .arg(format!("{}@{}", cfg.user, cfg.host))
        .arg(remote_cmd);
    run_command(cmd)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(host: &str, dir: &str) -> SshConfig {
        SshConfig {
            host: host.to_string(),
            user: "someone".to_string(),
            port: 22,
            identity_file: String::new(),
            remote_server_dir: dir.to_string(),
            start_script_name: "daoc-server.sh".to_string(),
        }
    }

    #[test]
    fn blank_host_is_local_mode() {
        assert!(is_local(&cfg("", "/anything")));
        assert!(is_local(&cfg("   ", "/anything")));
        assert!(!is_local(&cfg("192.168.1.69", "/anything")));
    }

    #[test]
    fn local_test_connection_reports_missing_folder() {
        let result = test_connection(&cfg("", "/this/does/not/exist/hopefully"));
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not a folder"));
    }

    #[test]
    fn local_test_connection_reports_missing_script() {
        let tmp = std::env::temp_dir().join("daoc-class-editor-test-empty-dir");
        std::fs::create_dir_all(&tmp).unwrap();
        let result = test_connection(&cfg("", tmp.to_str().unwrap()));
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("daoc-server.sh"));
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn local_test_connection_succeeds_with_script_present() {
        let tmp = std::env::temp_dir().join("daoc-class-editor-test-dir-with-script");
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(tmp.join("daoc-server.sh"), "#!/bin/sh\necho ok\n").unwrap();
        let result = test_connection(&cfg("", tmp.to_str().unwrap()));
        assert!(result.is_ok(), "{result:?}");
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn local_sql_runs_against_a_real_database() {
        let tmp = std::env::temp_dir().join("daoc-class-editor-test-sql-dir");
        std::fs::remove_dir_all(&tmp).ok();
        std::fs::create_dir_all(tmp.join("data")).unwrap();
        let db = tmp.join("data/opendaoc.sqlite3.db");
        let setup = Command::new("sqlite3")
            .arg(&db)
            .arg("CREATE TABLE ServerProperty (Key TEXT, Value TEXT); INSERT INTO ServerProperty VALUES ('enable_sluaghbinder','False');")
            .status()
            .expect("sqlite3 must be on PATH for this test");
        assert!(setup.success());

        let result = run_remote_sql(
            &cfg("", tmp.to_str().unwrap()),
            "SELECT Value FROM ServerProperty WHERE Key='enable_sluaghbinder';",
        );
        assert_eq!(result.unwrap().trim(), "False");
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn local_server_control_runs_the_real_script() {
        let tmp = std::env::temp_dir().join("daoc-class-editor-test-script-dir");
        std::fs::remove_dir_all(&tmp).ok();
        std::fs::create_dir_all(&tmp).unwrap();
        let script = tmp.join("daoc-server.sh");
        std::fs::write(&script, "#!/bin/sh\necho \"$1 ok\"\n").unwrap();
        let mut perms = std::fs::metadata(&script).unwrap().permissions();
        std::os::unix::fs::PermissionsExt::set_mode(&mut perms, 0o755);
        std::fs::set_permissions(&script, perms).unwrap();

        let result = remote_server_control(&cfg("", tmp.to_str().unwrap()), "status");
        assert_eq!(result.unwrap().trim(), "status ok");
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn local_deploy_writes_file_directly_to_disk() {
        let tmp = std::env::temp_dir().join("daoc-class-editor-test-deploy-dir");
        std::fs::remove_dir_all(&tmp).ok();
        let result = deploy_class_file(&cfg("", tmp.to_str().unwrap()), "ClassTest.cs", "// hi");
        assert!(result.is_ok(), "{result:?}");
        let written = std::fs::read_to_string(tmp.join("scripts/ClassTest.cs")).unwrap();
        assert_eq!(written, "// hi");
        std::fs::remove_dir_all(&tmp).ok();
    }
}
