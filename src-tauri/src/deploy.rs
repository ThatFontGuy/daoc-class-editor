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
    /// The server's own root folder (containing data/, config/, and bin/ where the server
    /// executable itself lives) - on the remote machine if `host` is set, or directly on this
    /// machine if it's blank.
    pub remote_server_dir: String,
    /// The server start/stop script's filename - configurable since it can be renamed
    /// (daoc-server.sh by default, but a server owner may have renamed it, e.g. startOthlyn.sh).
    pub start_script_name: String,
}

pub(crate) fn is_local(cfg: &SshConfig) -> bool {
    cfg.host.trim().is_empty()
}

/// The server's *real* scripts folder - confirmed directly in the server's own source
/// (`GameServerConfiguration`'s constructor always resolves `RootDirectory` to wherever its own
/// executable lives, with no config override path actually wired up), `RootDirectory` is always
/// `bin/`, not the top-level server folder. `tools/build-release.sh` only symlinks `bin/config`,
/// `bin/navmesh`, and `bin/logs` back out to the top-level folder - not `bin/scripts` - so
/// `bin/scripts/` is a real, separate directory (pre-populated with the server's own shipped
/// `dbupdater/` scripts), distinct from a top-level `scripts/` folder that the running server
/// never actually looks at. A custom class deployed to the wrong one compiles fine in isolation
/// and the server logs "Script compilation: True" either way (that's just "no files = trivial
/// success" when the real folder doesn't contain it), but `ScriptMgr.FindCharacterClass` then
/// can't find it at all - confirmed by reproducing exactly that "No CharacterClass with ID ...
/// found" error against a real deployment.
pub(crate) fn scripts_dir(remote_server_dir: &str) -> PathBuf {
    Path::new(remote_server_dir).join("bin").join("scripts")
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

pub(crate) fn ssh_base_args(cfg: &SshConfig) -> Vec<String> {
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

pub(crate) fn run_command(mut cmd: Command) -> Result<String, String> {
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

/// Writes `content` as `<server_dir>/bin/scripts/<filename>` (the server's real scripts folder -
/// see `scripts_dir`'s doc comment), either directly on disk (local mode) or via a temp file +
/// `scp` (SSH mode).
pub fn deploy_class_file(cfg: &SshConfig, filename: &str, content: &str) -> Result<String, String> {
    if is_local(cfg) {
        let dir = scripts_dir(&cfg.remote_server_dir);
        std::fs::create_dir_all(&dir)
            .map_err(|e| format!("could not create {}: {e}", dir.display()))?;
        let target = dir.join(filename);
        std::fs::write(&target, content)
            .map_err(|e| format!("could not write {}: {e}", target.display()))?;
        return Ok(format!("wrote {}", target.display()));
    }

    let mut tmp = tempfile_path(filename);
    {
        let mut f = std::fs::File::create(&tmp).map_err(|e| e.to_string())?;
        f.write_all(content.as_bytes()).map_err(|e| e.to_string())?;
    }
    // Make sure the remote scripts folder exists first (the server also auto-creates it on
    // startup, but a stopped server that has never run with EnableCompilation won't have it yet).
    let mkdir_result = {
        let mut cmd = Command::new("ssh");
        cmd.args(ssh_base_args(cfg))
            .arg(format!("{}@{}", cfg.user, cfg.host))
            .arg(format!(
                "mkdir -p '{}/bin/scripts'",
                shell_escape(&cfg.remote_server_dir)
            ));
        run_command(cmd)
    };
    if let Err(e) = mkdir_result {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("could not prepare remote scripts folder: {e}"));
    }

    let mut cmd = Command::new("scp");
    cmd.args(ssh_base_scp_args(cfg));
    cmd.arg(&tmp);
    cmd.arg(format!(
        "{}@{}:{}/bin/scripts/{}",
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
    run_sql_impl(cfg, sql, false)
}

/// Same as `run_remote_sql`, but requests sqlite3's `-json` output mode and parses it, for
/// commands that need structured results (e.g. listing specializations) rather than a plain
/// status string.
pub fn run_remote_sql_json(cfg: &SshConfig, sql: &str) -> Result<Vec<serde_json::Value>, String> {
    let text = run_sql_impl(cfg, sql, true)?;
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }
    serde_json::from_str(trimmed)
        .map_err(|e| format!("could not parse sqlite3 JSON output: {e}\n{trimmed}"))
}

fn run_sql_impl(cfg: &SshConfig, sql: &str, json: bool) -> Result<String, String> {
    let db_path = Path::new(&cfg.remote_server_dir).join("data/opendaoc.sqlite3.db");
    if is_local(cfg) {
        // Passed as separate argv entries, not a shell string, so no escaping is needed at all.
        let mut cmd = Command::new("sqlite3");
        if json {
            cmd.arg("-json");
        }
        cmd.arg(&db_path).arg(sql);
        return run_command(cmd);
    }
    let remote_cmd = format!(
        "sqlite3 {}'{}' '{}'",
        if json { "-json " } else { "" },
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
        if action == "start" {
            // The server keeps running as a background process after "start" returns - if its
            // stdout/stderr aren't redirected away from us, `.output()` below would block
            // forever waiting for pipes that never close (observed in practice: a "Start
            // server" click left the whole app stuck on *any* subsequent action, because this
            // was a synchronous command handler blocking the thread it ran on). Redirecting
            // before spawning, and not waiting on the child at all, avoids that regardless of
            // whether the start script itself redirects its own output.
            let mut cmd = Command::new(&script);
            cmd.current_dir(&cfg.remote_server_dir)
                .arg(action)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null());
            cmd.spawn()
                .map_err(|e| format!("failed to launch {}: {e}", script.display()))?;
            return Ok(
                "Start requested (not waiting for the server process - check status separately)"
                    .to_string(),
            );
        }
        let mut cmd = Command::new(&script);
        cmd.current_dir(&cfg.remote_server_dir).arg(action);
        return run_command(cmd);
    }
    if action == "start" {
        // Same reasoning as the local branch above, but the hang is even easier to hit over SSH:
        // ssh itself won't return until every duplicated stdio stream from the remote session
        // closes, which a background server process holding them open prevents indefinitely.
        // `nohup ... </dev/null >/dev/null 2>&1 &` severs the backgrounded process from this
        // session's streams entirely, so the remote shell (and therefore ssh) returns almost
        // immediately regardless of how the start script itself handles its own output.
        let remote_cmd = format!(
            "cd '{}' && nohup ./{} start </dev/null >/dev/null 2>&1 &",
            shell_escape(&cfg.remote_server_dir),
            shell_escape(&cfg.start_script_name),
        );
        let mut cmd = Command::new("ssh");
        cmd.args(ssh_base_args(cfg))
            .arg(format!("{}@{}", cfg.user, cfg.host))
            .arg(remote_cmd);
        run_command(cmd)?;
        return Ok(
            "Start requested (not waiting for the server process - check status separately)"
                .to_string(),
        );
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
    fn local_start_action_does_not_wait_for_a_backgrounded_child() {
        let tmp = std::env::temp_dir().join("daoc-class-editor-test-start-script-dir");
        std::fs::remove_dir_all(&tmp).ok();
        std::fs::create_dir_all(&tmp).unwrap();
        let script = tmp.join("daoc-server.sh");
        // Simulates a start script that backgrounds a long-running process without redirecting
        // its own output - the real-world bug this guards against: a "Start server" click left
        // the whole app stuck on every later action, because this command used to synchronously
        // wait on pipes a backgrounded server process held open forever.
        std::fs::write(&script, "#!/bin/sh\n(sleep 30) &\necho started\n").unwrap();
        let mut perms = std::fs::metadata(&script).unwrap().permissions();
        std::os::unix::fs::PermissionsExt::set_mode(&mut perms, 0o755);
        std::fs::set_permissions(&script, perms).unwrap();

        let start = std::time::Instant::now();
        let result = remote_server_control(&cfg("", tmp.to_str().unwrap()), "start");
        assert!(result.is_ok(), "{result:?}");
        assert!(
            start.elapsed() < std::time::Duration::from_secs(5),
            "start action should return almost immediately, took {:?}",
            start.elapsed()
        );

        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn local_deploy_writes_file_directly_to_disk() {
        let tmp = std::env::temp_dir().join("daoc-class-editor-test-deploy-dir");
        std::fs::remove_dir_all(&tmp).ok();
        let result = deploy_class_file(&cfg("", tmp.to_str().unwrap()), "ClassTest.cs", "// hi");
        assert!(result.is_ok(), "{result:?}");
        let written = std::fs::read_to_string(tmp.join("bin/scripts/ClassTest.cs")).unwrap();
        assert_eq!(written, "// hi");
        std::fs::remove_dir_all(&tmp).ok();
    }
}
