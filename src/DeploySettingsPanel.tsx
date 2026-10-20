import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { SshConfig, emptySshConfig } from "./types";

export default function DeploySettingsPanel() {
  const [config, setConfig] = useState<SshConfig>(emptySshConfig);
  const [status, setStatus] = useState<string>("");
  const [busy, setBusy] = useState(false);

  const isLocal = config.host.trim() === "";

  useEffect(() => {
    invoke<SshConfig>("load_ssh_config").then(setConfig);
  }, []);

  async function save() {
    setBusy(true);
    try {
      await invoke("save_ssh_config", { cfg: config });
      setStatus("Saved.");
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
        Used to deploy generated class files to your server's <code>scripts/</code> folder, run
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
    </div>
  );
}
