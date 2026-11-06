import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { SshConfig, WriteSafetyStatus, emptySshConfig } from "./types";

function formatAge(seconds: number): string {
  if (seconds < 60) return `${seconds}s`;
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes}m`;
  const hours = Math.floor(minutes / 60);
  return `${hours}h ${minutes % 60}m`;
}

export default function DeploySettingsPanel() {
  const [config, setConfig] = useState<SshConfig>(emptySshConfig);
  const [status, setStatus] = useState<string>("");
  const [busy, setBusy] = useState(false);
  const [writeSafety, setWriteSafety] = useState<WriteSafetyStatus | null>(null);
  const [safetyError, setSafetyError] = useState("");

  const isLocal = config.host.trim() === "";

  useEffect(() => {
    invoke<SshConfig>("load_ssh_config").then((cfg) => {
      setConfig(cfg);
      refreshWriteSafety(cfg);
    });
    // Keeps the "last backup: Xm ago" display current without the user having to manually
    // refresh - this is the one status the whole write-safety scheme depends on being trusted,
    // so it should never look stale itself.
    const interval = setInterval(() => {
      setConfig((current) => {
        refreshWriteSafety(current);
        return current;
      });
    }, 60_000);
    return () => clearInterval(interval);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  async function refreshWriteSafety(cfg: SshConfig) {
    try {
      const result = await invoke<WriteSafetyStatus>("write_safety_status", { cfg });
      setWriteSafety(result);
      setSafetyError("");
    } catch (e) {
      setWriteSafety(null);
      setSafetyError(`Could not check server/backup status: ${e}`);
    }
  }

  async function save() {
    setBusy(true);
    try {
      await invoke("save_ssh_config", { cfg: config });
      setStatus("Saved.");
      await refreshWriteSafety(config);
    } catch (e) {
      setStatus(`Error saving: ${e}`);
    } finally {
      setBusy(false);
    }
  }

  async function test() {
    setBusy(true);
    setStatus(isLocal ? "Checking local folder..." : "Connecting...");
    try {
      const result = await invoke<string>("test_ssh_connection", { cfg: config });
      setStatus(`OK:\n${result}`);
    } catch (e) {
      setStatus(`Failed: ${e}`);
    } finally {
      setBusy(false);
    }
  }

  async function backupDatabase() {
    setBusy(true);
    setStatus("Backing up...");
    try {
      const result = await invoke<string>("backup_database", { cfg: config });
      setStatus(result);
      await refreshWriteSafety(config);
    } catch (e) {
      setStatus(`Backup failed: ${e}`);
    } finally {
      setBusy(false);
    }
  }

  async function revertDatabase() {
    if (
      !confirm(
        "Revert the live database to the last backup?\n\n" +
          "This throws away every change made since that backup was taken - there is no undo " +
          "for this. Make sure the server is stopped first.",
      )
    ) {
      return;
    }
    setBusy(true);
    setStatus("Reverting...");
    try {
      const result = await invoke<string>("revert_database", { cfg: config });
      setStatus(result);
      await refreshWriteSafety(config);
    } catch (e) {
      setStatus(`Revert failed: ${e}`);
    } finally {
      setBusy(false);
    }
  }

  function field(
    label: string,
    key: keyof SshConfig,
    placeholder: string,
    type: string = "text",
    disabled: boolean = false,
  ) {
    return (
      <label className="field">
        <span>{label}</span>
        <input
          type={type}
          placeholder={placeholder}
          value={config[key] as string | number}
          disabled={disabled}
          onChange={(e) =>
            setConfig({
              ...config,
              [key]: type === "number" ? Number(e.target.value) : e.target.value,
            })
          }
        />
      </label>
    );
  }

  return (
    <div className="panel">
      <h2>Server connection</h2>
      <p className="hint">
        Used to deploy generated class files to your server's <code>bin/scripts/</code> folder, run
        the small SQL statements this tool needs, and restart the server.
      </p>
      <p className="hint">
        <strong>If the server runs on this same computer, leave Host blank</strong> - the folder
        field below is then used directly as a local path, no SSH involved at all. Fill in Host
        if the server runs elsewhere; this reuses your existing SSH keys/agent, nothing is sent
        anywhere except directly to your server.
      </p>
      <div className="mode-badge">{isLocal ? "Mode: local (no SSH)" : "Mode: SSH"}</div>

      {field(
        "Host (leave blank for local mode)",
        "host",
        "blank = local, or e.g. 192.168.1.69 / your DDNS address",
      )}
      {field("SSH user", "user", "your-username", "text", isLocal)}
      {field("SSH port", "port", "22", "number", isLocal)}
      {field("Identity file (optional)", "identity_file", "~/.ssh/id_ed25519", "text", isLocal)}
      {field(
        isLocal ? "Server folder (local path on this machine)" : "Server folder on the remote machine",
        "remote_server_dir",
        "/path/to/your/daoc-server",
      )}
      {field(
        "Start/stop script filename",
        "start_script_name",
        "daoc-server.sh (or whatever you renamed it to)",
      )}
      <div className="actions">
        <button onClick={save} disabled={busy}>
          Save
        </button>
        <button onClick={test} disabled={busy}>
          {isLocal ? "Check local folder" : "Test connection"}
        </button>
      </div>
      {status && <pre className="status">{status}</pre>}

      <h3>Write safety</h3>
      <p className="hint">
        Every change this tool can make to your live server (deploying a class, editing specs/
        spells/styles, running SQL, ...) is refused unless the server is stopped <em>and</em> a
        backup was taken within the last 30 minutes - stop the server, back up, make your
        changes, test them, and use <strong>Revert Database</strong> if something broke.{" "}
        <strong>Backup Database</strong> always replaces the one backup this tool keeps (it isn't
        a history) - any longer-term backups are still up to you.
      </p>
      {safetyError && <pre className="status">{safetyError}</pre>}
      {writeSafety && (
        <p>
          Server:{" "}
          {writeSafety.server_running ? (
            <span className="badge-disabled">RUNNING - stop it to make changes</span>
          ) : (
            <strong>stopped</strong>
          )}
          {" · "}
          Backup:{" "}
          {!writeSafety.backup.exists ? (
            <span className="badge-disabled">none yet - back up before making changes</span>
          ) : writeSafety.backup.is_fresh ? (
            <strong>{formatAge(writeSafety.backup.age_seconds ?? 0)} ago</strong>
          ) : (
            <span className="badge-disabled">
              {formatAge(writeSafety.backup.age_seconds ?? 0)} ago - too old, back up again
            </span>
          )}
        </p>
      )}
      <div className="actions">
        <button onClick={backupDatabase} disabled={busy}>
          Backup Database
        </button>
        <button
          onClick={revertDatabase}
          disabled={busy || !writeSafety?.backup.exists}
          className="danger"
        >
          Revert Database
        </button>
      </div>
    </div>
  );
}
