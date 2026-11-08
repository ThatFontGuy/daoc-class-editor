//! A disposable, local-only working copy of the live database a user can design a new (or
//! re-edit an already-deployed) class against freely - assigning specs, picking spell lines,
//! previewing generated code - all without the live server needing to be stopped at all.
//!
//! The real problem this solves: before this existed, `apply_class_specs` (assigning specs to a
//! class being built) wrote directly to the *live* database, so it was gated by the exact same
//! `ensure_safe_to_write` check as every other live write - requiring the server to be stopped
//! for the *entire* design process (which can run long: browsing specs, trying different spell-
//! line selections, previewing names), not just the brief moment the class actually goes live.
//! The owner's own framing of the fix: "the safeguard check on whether the server is running
//! should be conducted when I first attempt to interact with the server... not before."
//!
//! `apply_class_specs` is fully declarative (it always fully specifies the desired end state via
//! `assignments`/`previous_keys`, not an incremental delta - confirmed by its own existing
//! idempotency guarantee), so no action-log/replay machinery is needed at all: a design session
//! is purely a scratch copy to iterate against safely, and `ClonePanel.tsx`'s own deploy step
//! simply re-invokes the exact same `apply_class_specs` call a second time against the *live*
//! config once the user is ready - freshly re-reading the source class's current specs and
//! re-resolving every new ID against the live database's actual current state, same as any other
//! deploy. A later source-class edit in the interim is naturally picked up, not frozen at
//! whatever the snapshot looked like when the session started.
//!
//! Taking the snapshot itself is always safe, live server running or not: sqlite3's own
//! `.backup` command is explicitly designed to run safely against a database another process
//! has open and is actively writing to (confirmed already in `deploy::backup_database`'s own doc
//! comment - "safe even against a WAL-mode database with writes not yet checkpointed"). It's only
//! *writing back* to the live file that's unsafe while the server holds it open - which is why
//! actually deploying still goes through `ensure_safe_to_write` exactly as before, just now only
//! at that one final moment instead of for the whole design process.

use crate::deploy::{SshConfig, is_local, run_command, run_remote_sql, ssh_base_args, ssh_base_scp_args};
use std::path::{Path, PathBuf};
use std::process::Command;
use uuid::Uuid;

/// Every design session's own local working copy lives under here, one subdirectory per
/// session (named by a fresh UUID) - never the user's own config/data, so wiping this whole tree
/// is always safe if something is ever left behind by a session that was never cleanly ended.
fn sessions_root() -> PathBuf {
    let base = dirs::config_dir().unwrap_or_else(std::env::temp_dir);
    base.join("daoc-class-editor").join("design-sessions")
}

/// Starts a brand-new local design session - see this module's own doc comment for the full
/// rationale. Returns an `SshConfig` pointing at the fresh local copy (blank `host`, so every
/// existing `specs.rs`/`classdata.rs` function already works against it completely unchanged,
/// via `deploy::is_local`'s existing local/SSH dispatch).
pub fn start_design_session(cfg: &SshConfig) -> Result<SshConfig, String> {
    let session_dir = sessions_root().join(Uuid::new_v4().to_string());
    let local_data_dir = session_dir.join("data");
    std::fs::create_dir_all(&local_data_dir)
        .map_err(|e| format!("could not create {}: {e}", local_data_dir.display()))?;
    let local_db = local_data_dir.join("opendaoc.sqlite3.db");

    let snapshot_result = if is_local(cfg) {
        let live_db = Path::new(&cfg.remote_server_dir).join("data/opendaoc.sqlite3.db");
        let mut backup_cmd = Command::new("sqlite3");
        backup_cmd
            .arg(&live_db)
            .arg(format!(".backup '{}'", local_db.to_string_lossy()));
        run_command(backup_cmd).map(|_| ())
    } else {
        snapshot_remote_db(cfg, &local_db)
    };

    if let Err(e) = snapshot_result {
        let _ = std::fs::remove_dir_all(&session_dir);
        return Err(format!("could not snapshot the database for a new design session: {e}"));
    }

    Ok(SshConfig {
        host: String::new(),
        user: String::new(),
        port: 22,
        identity_file: String::new(),
        remote_server_dir: session_dir.to_string_lossy().to_string(),
        start_script_name: String::new(),
    })
}

/// Remote case: snapshot into a remote temp file first (sqlite3's own `.backup`, run over SSH -
/// the same mechanism `backup_database` already uses for the live server's own managed backup),
/// then `scp` it down, then clean up the remote temp copy either way - nothing from a design
/// session is ever left behind on the server.
fn snapshot_remote_db(cfg: &SshConfig, local_db: &Path) -> Result<(), String> {
    let remote_tmp = format!("/tmp/daoc-class-editor-design-{}.db", Uuid::new_v4());
    let backup_result = run_remote_sql(cfg, &format!(".backup {remote_tmp}"));
    let scp_result = backup_result.and_then(|_| {
        let mut cmd = Command::new("scp");
        cmd.args(ssh_base_scp_args(cfg));
        cmd.arg(format!("{}@{}:{}", cfg.user, cfg.host, remote_tmp));
        cmd.arg(local_db);
        run_command(cmd)
    });

    let mut cleanup = Command::new("ssh");
    cleanup
        .args(ssh_base_args(cfg))
        .arg(format!("{}@{}", cfg.user, cfg.host))
        .arg(format!("rm -f '{remote_tmp}'"));
    let _ = run_command(cleanup);

    scp_result.map(|_| ())
}

/// Deletes a design session's own local working copy entirely - called once its changes have
/// been deployed to the live server, or if the user discards the design outright. A no-op (not
/// an error) if the directory is already gone, so calling this twice, or on a session that was
/// never actually started, is always safe. Refuses anything that isn't really one of this
/// module's own session directories (under `sessions_root()`) - a defensive check against ever
/// deleting the wrong thing, even though every caller is expected to pass back exactly the
/// `SshConfig` `start_design_session` returned.
pub fn discard_design_session(session_cfg: &SshConfig) -> Result<(), String> {
    if !is_local(session_cfg) {
        return Err("not a local design session - refusing to delete".to_string());
    }
    let dir = Path::new(&session_cfg.remote_server_dir);
    if !dir.exists() {
        // Already gone (or never existed) - a safe no-op, matching this function's own
        // "calling it twice is always safe" guarantee. Checked before canonicalize(), which
        // requires the path to actually exist and would otherwise misreport a session that was
        // already cleanly discarded as "not a design session directory" instead.
        return Ok(());
    }
    let root = sessions_root();
    match dir.canonicalize() {
        Ok(canonical) if canonical.starts_with(&root) => {}
        _ => {
            return Err(format!(
                "'{}' is not a design session directory under {} - refusing to delete",
                session_cfg.remote_server_dir,
                root.display()
            ));
        }
    }
    if dir.exists() {
        std::fs::remove_dir_all(dir).map_err(|e| format!("could not remove {}: {e}", dir.display()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command as StdCommand;

    fn local_cfg(dir: &str) -> SshConfig {
        SshConfig {
            host: String::new(),
            user: String::new(),
            port: 22,
            identity_file: String::new(),
            remote_server_dir: dir.to_string(),
            start_script_name: "daoc-server.sh".to_string(),
        }
    }

    /// A real local "server folder" with a seeded `data/opendaoc.sqlite3.db` - everything
    /// `start_design_session`'s own local-mode snapshot path needs to run for real.
    fn setup_live_server(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("daoc-class-editor-design-session-test-{name}"));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(dir.join("data")).unwrap();
        let db = dir.join("data/opendaoc.sqlite3.db");
        let status = StdCommand::new("sqlite3")
            .arg(&db)
            .arg("CREATE TABLE Specialization (KeyName TEXT, Name TEXT); INSERT INTO Specialization VALUES ('smite','Smite');")
            .status()
            .expect("sqlite3 must be on PATH for this test");
        assert!(status.success());
        dir
    }

    #[test]
    fn start_design_session_snapshots_the_live_database_into_a_local_copy() {
        let live_dir = setup_live_server("snapshot-basic");
        let session_cfg = start_design_session(&local_cfg(live_dir.to_str().unwrap())).unwrap();

        assert!(is_local(&session_cfg), "a design session is always local-mode");
        let session_db = Path::new(&session_cfg.remote_server_dir).join("data/opendaoc.sqlite3.db");
        assert!(session_db.is_file(), "{}", session_db.display());

        let rows = crate::deploy::run_remote_sql_json(
            &session_cfg,
            "SELECT Name FROM Specialization WHERE KeyName='smite';",
        )
        .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["Name"], "Smite");

        discard_design_session(&session_cfg).unwrap();
        std::fs::remove_dir_all(&live_dir).ok();
    }

    #[test]
    fn start_design_session_is_safe_to_call_while_the_server_is_marked_running() {
        // The entire point: taking a snapshot via sqlite3's own .backup never requires the
        // server to be stopped first, unlike every real write this tool makes - confirmed by
        // simply never checking `is_server_running` at all here.
        let live_dir = setup_live_server("snapshot-while-running");
        let script = live_dir.join("daoc-server.sh");
        std::fs::write(&script, "#!/bin/sh\necho \"running (pid 1)\"\n").unwrap();
        let mut perms = std::fs::metadata(&script).unwrap().permissions();
        std::os::unix::fs::PermissionsExt::set_mode(&mut perms, 0o755);
        std::fs::set_permissions(&script, perms).unwrap();

        let result = start_design_session(&local_cfg(live_dir.to_str().unwrap()));
        assert!(result.is_ok(), "{result:?}");

        discard_design_session(&result.unwrap()).unwrap();
        std::fs::remove_dir_all(&live_dir).ok();
    }

    #[test]
    fn writes_to_a_design_sessions_copy_never_touch_the_live_database() {
        let live_dir = setup_live_server("writes-stay-local");
        let live_cfg = local_cfg(live_dir.to_str().unwrap());
        let session_cfg = start_design_session(&live_cfg).unwrap();

        crate::deploy::run_remote_sql(
            &session_cfg,
            "INSERT INTO Specialization VALUES ('new_spec','Brand New Spec');",
        )
        .unwrap();

        let live_rows = crate::deploy::run_remote_sql_json(
            &live_cfg,
            "SELECT COUNT(*) AS c FROM Specialization;",
        )
        .unwrap();
        assert_eq!(live_rows[0]["c"], 1, "the live database must be completely untouched");

        let session_rows = crate::deploy::run_remote_sql_json(
            &session_cfg,
            "SELECT COUNT(*) AS c FROM Specialization;",
        )
        .unwrap();
        assert_eq!(session_rows[0]["c"], 2, "the session's own copy has the new row");

        discard_design_session(&session_cfg).unwrap();
        std::fs::remove_dir_all(&live_dir).ok();
    }

    #[test]
    fn discard_design_session_removes_the_local_copy() {
        let live_dir = setup_live_server("discard");
        let session_cfg = start_design_session(&local_cfg(live_dir.to_str().unwrap())).unwrap();
        let session_dir = session_cfg.remote_server_dir.clone();
        assert!(Path::new(&session_dir).is_dir());

        discard_design_session(&session_cfg).unwrap();
        assert!(!Path::new(&session_dir).exists());

        // Calling it again (already gone) must be a safe no-op, not an error.
        assert!(discard_design_session(&session_cfg).is_ok());

        std::fs::remove_dir_all(&live_dir).ok();
    }

    #[test]
    fn discard_design_session_refuses_a_directory_outside_its_own_sessions_root() {
        let outside = std::env::temp_dir().join("daoc-class-editor-not-a-design-session");
        std::fs::create_dir_all(&outside).unwrap();
        let result = discard_design_session(&local_cfg(outside.to_str().unwrap()));
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("refusing to delete"));
        std::fs::remove_dir_all(&outside).ok();
    }
}
