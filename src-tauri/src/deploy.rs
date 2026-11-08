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

pub(crate) fn ssh_base_scp_args(cfg: &SshConfig) -> Vec<String> {
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

/// True if the server's own management script reports it's currently running - confirmed
/// directly against `daoc-server.sh`'s own `status` case (`echo "running (pid $pid)"` or
/// `echo "stopped"`, with no distinct exit code either way - this command always exits 0
/// regardless, so the leading word of its own output is the only thing that actually
/// distinguishes the two).
pub fn is_server_running(cfg: &SshConfig) -> Result<bool, String> {
    let output = remote_server_control(cfg, "status")?;
    Ok(output.trim_start().starts_with("running"))
}

/// This tool's own single, disposable "last known good" checkpoint of the live database -
/// always this exact filename, right next to the real one, replaced every time `backup_database`
/// runs (never kept as a history of its own) - deliberately distinct from the various one-off,
/// hand-made `.bak-<reason>-<date>` snapshots this project's own session history already has.
fn backup_path(remote_server_dir: &str) -> PathBuf {
    let db = Path::new(remote_server_dir).join("data/opendaoc.sqlite3.db");
    let mut path = db.into_os_string();
    path.push(".class-editor-backup");
    PathBuf::from(path)
}

/// A write is only ever considered safe within this long after the last backup - half an hour,
/// per the owner's own stated rule for this whole scheme: long enough to make one round of
/// changes and test them in-game, short enough that an old backup is never mistaken for a
/// current "known good" checkpoint.
const FRESH_BACKUP_MAX_AGE_SECS: i64 = 30 * 60;

#[derive(Debug, Clone, serde::Serialize)]
pub struct BackupStatus {
    pub exists: bool,
    /// Seconds since the backup file was last written - `None` iff `exists` is false. Read
    /// straight from the file's own mtime, so there's no separate tracking state that could ever
    /// drift out of sync with what's actually on disk.
    pub age_seconds: Option<i64>,
    pub is_fresh: bool,
}

/// How long ago (if ever) `backup_database` last actually ran, read straight from the backup
/// file's own mtime - the single source of truth both the write-safety gate and the UI's own
/// "last backup: Xm ago" indicator rely on.
pub fn backup_status(cfg: &SshConfig) -> Result<BackupStatus, String> {
    let path = backup_path(&cfg.remote_server_dir);
    let age_seconds = if is_local(cfg) {
        match std::fs::metadata(&path).and_then(|m| m.modified()) {
            Ok(modified) => Some(
                std::time::SystemTime::now()
                    .duration_since(modified)
                    .map(|d| d.as_secs() as i64)
                    .unwrap_or(0),
            ),
            Err(_) => None,
        }
    } else {
        // The age itself is computed entirely on the remote host (both "now" and the file's own
        // mtime, in the same `date`/`stat` call) rather than comparing a remote mtime against
        // this machine's own clock - a real, confirmed case this matters for: the actual server
        // this project runs against has a clock about an hour off from the machine running this
        // app, which would otherwise make a backup taken seconds ago look nearly an hour stale
        // (or the reverse) purely from clock skew, nothing to do with how old it actually is.
        // Linux `stat -c %Y` first, falling back to BSD/macOS `stat -f %m`.
        let remote_cmd = format!(
            "mtime=$(stat -c %Y '{path}' 2>/dev/null || stat -f %m '{path}' 2>/dev/null) && echo $(($(date +%s) - mtime))",
            path = shell_escape(&path.to_string_lossy())
        );
        let mut cmd = Command::new("ssh");
        cmd.args(ssh_base_args(cfg))
            .arg(format!("{}@{}", cfg.user, cfg.host))
            .arg(remote_cmd);
        match run_command(cmd) {
            Ok(out) => {
                let trimmed = out.trim();
                if trimmed.is_empty() {
                    None
                } else {
                    let age: i64 = trimmed
                        .parse()
                        .map_err(|_| format!("could not parse backup age: {trimmed}"))?;
                    Some(age.max(0))
                }
            }
            Err(_) => None,
        }
    };
    Ok(backup_status_from_age(age_seconds))
}

/// Pure threshold logic, split out from `backup_status` so the exists/fresh boundary can be unit
/// tested directly rather than by actually back-dating a real file's mtime.
fn backup_status_from_age(age_seconds: Option<i64>) -> BackupStatus {
    BackupStatus {
        exists: age_seconds.is_some(),
        age_seconds,
        is_fresh: age_seconds.is_some_and(|age| age <= FRESH_BACKUP_MAX_AGE_SECS),
    }
}

fn format_age(seconds: i64) -> String {
    if seconds < 60 {
        format!("{seconds}s")
    } else {
        format!("{}m{}s", seconds / 60, seconds % 60)
    }
}

/// The single choke point every command that can actually change the live database or deploy a
/// file must call first - refuses outright if the server is still running (writing underneath
/// its own in-memory state risks corruption or a silent overwrite the moment it next saves), or
/// if there's no sufficiently recent backup to fall back to if this particular edit turns out to
/// be a mistake. Never bypassed by a different code path - this is the owner's own stated
/// intent for this whole scheme: stop the server, back up, make changes, test, revert if
/// something broke.
pub fn ensure_safe_to_write(cfg: &SshConfig) -> Result<(), String> {
    if is_server_running(cfg)? {
        return Err(
            "the server is still running - stop it first (Server connection tab), then make \
             changes, so nothing is written underneath its own in-memory state"
                .to_string(),
        );
    }
    let status = backup_status(cfg)?;
    match status.age_seconds {
        None => Err(
            "no backup found - click Backup Database first (Server connection tab) before \
             making any changes"
                .to_string(),
        ),
        Some(age) if !status.is_fresh => Err(format!(
            "the last backup is {} old - click Backup Database again (Server connection tab) \
             before making any changes (a backup older than {} minutes is no longer considered \
             safe to rely on)",
            format_age(age),
            FRESH_BACKUP_MAX_AGE_SECS / 60
        )),
        Some(_) => Ok(()),
    }
}

/// Replaces this tool's own single managed backup with a fresh snapshot of the live database -
/// always via sqlite3's own `.backup` command (safe even against a WAL-mode database with writes
/// not yet checkpointed), never a plain file copy. Refuses outright if the server is still
/// running - the same invariant every other write goes through, extended to backups too, so "I
/// have a fresh backup" always genuinely means a clean snapshot of a database nothing was
/// actively writing to, not just "a file that happens to be recent."
pub fn backup_database(cfg: &SshConfig) -> Result<String, String> {
    if is_server_running(cfg)? {
        return Err(
            "the server is still running - stop it first (Server connection tab), then back up"
                .to_string(),
        );
    }
    let backup = backup_path(&cfg.remote_server_dir);
    run_remote_sql(cfg, &format!(".backup {}", backup.to_string_lossy()))?;
    Ok(format!("backed up to {}", backup.display()))
}

/// Restores this tool's own single managed backup over the live database - via sqlite3's own
/// `.restore` command, the exact inverse of `.backup` above. Refuses outright if the server is
/// still running (restoring underneath its own open file would be at least as unsafe as writing
/// to it directly), or if there's no backup to restore from at all. Deliberately does *not*
/// require the backup to still be "fresh" the way a write does - reverting to whatever backup
/// exists, however old, is exactly the point of this button, even if testing ran long past the
/// usual half-hour window.
pub fn revert_database(cfg: &SshConfig) -> Result<String, String> {
    if is_server_running(cfg)? {
        return Err(
            "the server is still running - stop it first (Server connection tab), then revert"
                .to_string(),
        );
    }
    let status = backup_status(cfg)?;
    if !status.exists {
        return Err("no backup found to revert to - nothing to do".to_string());
    }
    let backup = backup_path(&cfg.remote_server_dir);
    run_remote_sql(cfg, &format!(".restore {}", backup.to_string_lossy()))?;
    Ok("reverted the live database to the last backup".to_string())
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

    /// A fresh temp "server folder" whose `daoc-server.sh status` reports either
    /// `"running (pid 1)"` or `"stopped"` depending on `running`, plus a real, seeded
    /// `data/opendaoc.sqlite3.db` - everything `is_server_running`/`backup_database`/
    /// `revert_database`/`ensure_safe_to_write` need to be exercised for real, locally.
    fn setup_backup_test_server(name: &str, running: bool) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("daoc-class-editor-backup-test-{name}"));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(dir.join("data")).unwrap();
        let script = dir.join("daoc-server.sh");
        let body = if running {
            "#!/bin/sh\necho \"running (pid 1)\"\n"
        } else {
            "#!/bin/sh\necho stopped\n"
        };
        std::fs::write(&script, body).unwrap();
        let mut perms = std::fs::metadata(&script).unwrap().permissions();
        std::os::unix::fs::PermissionsExt::set_mode(&mut perms, 0o755);
        std::fs::set_permissions(&script, perms).unwrap();

        let db = dir.join("data/opendaoc.sqlite3.db");
        let status = Command::new("sqlite3")
            .arg(&db)
            .arg("CREATE TABLE T (V TEXT); INSERT INTO T VALUES ('original');")
            .status()
            .unwrap();
        assert!(status.success());
        dir
    }

    #[test]
    fn is_server_running_reads_the_scripts_own_status_output() {
        let running_dir = setup_backup_test_server("is-running-true", true);
        assert!(is_server_running(&cfg("", running_dir.to_str().unwrap())).unwrap());
        std::fs::remove_dir_all(&running_dir).ok();

        let stopped_dir = setup_backup_test_server("is-running-false", false);
        assert!(!is_server_running(&cfg("", stopped_dir.to_str().unwrap())).unwrap());
        std::fs::remove_dir_all(&stopped_dir).ok();
    }

    #[test]
    fn backup_status_is_absent_until_a_backup_is_taken_then_fresh_afterward() {
        let dir = setup_backup_test_server("status-lifecycle", false);
        let c = cfg("", dir.to_str().unwrap());

        let before = backup_status(&c).unwrap();
        assert!(!before.exists);
        assert_eq!(before.age_seconds, None);
        assert!(!before.is_fresh);

        backup_database(&c).unwrap();

        let after = backup_status(&c).unwrap();
        assert!(after.exists);
        assert!(after.age_seconds.unwrap() < 5, "{after:?}");
        assert!(after.is_fresh);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn backup_database_refuses_while_the_server_is_running() {
        let dir = setup_backup_test_server("backup-refuses-running", true);
        let result = backup_database(&cfg("", dir.to_str().unwrap()));
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("still running"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn ensure_safe_to_write_refuses_when_the_server_is_running() {
        let dir = setup_backup_test_server("gate-refuses-running", true);
        let result = ensure_safe_to_write(&cfg("", dir.to_str().unwrap()));
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("still running"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn ensure_safe_to_write_refuses_with_no_backup_then_allows_one_right_after() {
        let dir = setup_backup_test_server("gate-no-backup", false);
        let c = cfg("", dir.to_str().unwrap());

        let no_backup = ensure_safe_to_write(&c);
        assert!(no_backup.is_err());
        assert!(no_backup.unwrap_err().contains("no backup found"));

        backup_database(&c).unwrap();
        assert!(ensure_safe_to_write(&c).is_ok());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn backup_status_from_age_is_fresh_right_at_the_boundary_and_stale_just_past_it() {
        let missing = backup_status_from_age(None);
        assert!(!missing.exists);
        assert!(!missing.is_fresh);

        let fresh = backup_status_from_age(Some(FRESH_BACKUP_MAX_AGE_SECS));
        assert!(fresh.exists);
        assert!(fresh.is_fresh, "exactly the max age must still count as fresh");

        let stale = backup_status_from_age(Some(FRESH_BACKUP_MAX_AGE_SECS + 1));
        assert!(stale.exists);
        assert!(!stale.is_fresh, "one second past the max age must count as stale");
    }

    #[test]
    fn revert_database_restores_the_backup_over_live_changes() {
        let dir = setup_backup_test_server("revert-restores", false);
        let c = cfg("", dir.to_str().unwrap());
        let db = dir.join("data/opendaoc.sqlite3.db");

        backup_database(&c).unwrap();
        // A "live" change made after the backup, that revert should undo.
        let change = Command::new("sqlite3")
            .arg(&db)
            .arg("UPDATE T SET V = 'changed';")
            .status()
            .unwrap();
        assert!(change.success());

        revert_database(&c).unwrap();

        let rows = Command::new("sqlite3")
            .arg(&db)
            .arg("SELECT V FROM T;")
            .output()
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&rows.stdout).trim(), "original");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn revert_database_refuses_with_no_backup_and_while_the_server_is_running() {
        let dir = setup_backup_test_server("revert-refuses", false);
        let c = cfg("", dir.to_str().unwrap());
        let no_backup = revert_database(&c);
        assert!(no_backup.is_err());
        assert!(no_backup.unwrap_err().contains("no backup found"));
        std::fs::remove_dir_all(&dir).ok();

        let running_dir = setup_backup_test_server("revert-refuses-running", true);
        let running_cfg = cfg("", running_dir.to_str().unwrap());
        let result = revert_database(&running_cfg);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("still running"));
        std::fs::remove_dir_all(&running_dir).ok();
    }
}
