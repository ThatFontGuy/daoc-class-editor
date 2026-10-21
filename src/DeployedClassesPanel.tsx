import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { DeployedClass, SshConfig, emptySshConfig } from "./types";

export default function DeployedClassesPanel() {
  const [sshConfig, setSshConfig] = useState<SshConfig>(emptySshConfig);
  const [classes, setClasses] = useState<DeployedClass[] | null>(null);
  const [status, setStatus] = useState("");
  const [busy, setBusy] = useState(false);
  const [cleanupByFile, setCleanupByFile] = useState<Record<string, boolean>>({});

  useEffect(() => {
    invoke<SshConfig>("load_ssh_config").then(setSshConfig);
  }, []);

  async function refresh() {
    setBusy(true);
    setStatus("Loading...");
    try {
      const result = await invoke<DeployedClass[]>("list_deployed_classes", { cfg: sshConfig });
      setClasses(result);
      setStatus(result.length === 0 ? "No deployed custom classes found in scripts/." : "");
    } catch (e) {
      setStatus(`Could not list deployed classes: ${e}`);
      setClasses(null);
    } finally {
      setBusy(false);
    }
  }

  async function remove(c: DeployedClass) {
    const cleanup = cleanupByFile[c.filename] ?? true;
    const warning =
      `Delete "${c.name}" (id ${c.id}, ${c.filename})?\n\n` +
      "This removes the class file from scripts/ and its spec assignments" +
      (cleanup ? ", plus every spec line/spell line/style this tool cloned for it." : ".") +
      "\n\nAny existing character already using this class would be left in a broken state - " +
      "only delete classes with no characters on them, and stop the server first.";
    if (!confirm(warning)) return;

    setBusy(true);
    setStatus(`Deleting ${c.name}...`);
    try {
      const result = await invoke<string>("delete_deployed_class", {
        cfg: sshConfig,
        filename: c.filename,
        classId: c.id,
        cleanupClonedSpecs: cleanup,
      });
      setStatus(result);
      await refresh();
    } catch (e) {
      setStatus(`Delete failed: ${e}`);
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="panel">
      <h2>Deployed classes</h2>
      <p className="hint">
        Lists custom classes currently in your server's <code>scripts/</code> folder (anything
        generated and deployed with this tool). Useful for cleaning up a clone that didn't work
        out, or just to see what's installed.
      </p>
      <p className="hint">
        <strong>Stop the server before deleting</strong> - the same database-lock caution as
        deploying applies here. Deleting a class still in use by an existing character will leave
        that character broken; only delete classes nothing is using yet.
      </p>
      <div className="actions">
        <button onClick={refresh} disabled={busy}>
          Refresh list
        </button>
      </div>
      {status && <pre className="status">{status}</pre>}

      {classes && classes.length > 0 && (
        <table className="spec-table">
          <thead>
            <tr>
              <th>ID</th>
              <th>Name</th>
              <th>Based on</th>
              <th>File</th>
              <th>Also remove cloned spec lines</th>
              <th></th>
            </tr>
          </thead>
          <tbody>
            {classes.map((c) => (
              <tr key={c.filename}>
                <td>{c.id}</td>
                <td>{c.name}</td>
                <td>{c.base_class_name}</td>
                <td>
                  <code>{c.filename}</code>
                </td>
                <td>
                  <input
                    type="checkbox"
                    checked={cleanupByFile[c.filename] ?? true}
                    onChange={(e) =>
                      setCleanupByFile((prev) => ({ ...prev, [c.filename]: e.target.checked }))
                    }
                  />
                </td>
                <td>
                  <button onClick={() => remove(c)} disabled={busy} className="danger">
                    Delete
                  </button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </div>
  );
}
