import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ClassInfo, ClassListResult, SshConfig, emptySshConfig } from "./types";
import { buildEditingProjectState, useProject } from "./ProjectContext";

export default function DeployedClassesPanel({ onEditDeployed }: { onEditDeployed: () => void }) {
  const [, setProject] = useProject();
  const [sshConfig, setSshConfig] = useState<SshConfig>(emptySshConfig);
  // The full merged list (vendored + deployed) is kept around purely so "Edit" can look up
  // whatever a deployed class's own parent_ident points at - that parent is usually a vendored
  // class, but could itself be another deployed one in a multi-generation clone chain.
  const [allClasses, setAllClasses] = useState<ClassInfo[]>([]);
  const [deployedClasses, setDeployedClasses] = useState<ClassInfo[] | null>(null);
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
      const result = await invoke<ClassListResult>("list_classes_with_deployed", {
        cfg: sshConfig,
      });
      setAllClasses(result.classes);
      const deployed = result.classes.filter((c) => c.is_custom);
      setDeployedClasses(deployed);
      setStatus(
        result.deployed_error
          ? `Could not read deployed classes: ${result.deployed_error}`
          : deployed.length === 0
            ? "No deployed custom classes found in scripts/."
            : "",
      );
    } catch (e) {
      setStatus(`Could not list deployed classes: ${e}`);
      setDeployedClasses(null);
    } finally {
      setBusy(false);
    }
  }

  function edit(c: ClassInfo) {
    const parent = allClasses.find((p) => p.class_ident === c.parent_ident);
    if (!parent) {
      setStatus(
        `Could not find ${c.parent_ident} (the class ${c.name} subclasses) in the current class list - try Refresh list first.`,
      );
      return;
    }
    setProject(buildEditingProjectState(c, parent));
    onEditDeployed();
  }

  async function remove(c: ClassInfo) {
    const filename = c.filename ?? `${c.class_ident}.cs`;
    const cleanup = cleanupByFile[filename] ?? true;
    const warning =
      `Delete "${c.name}" (id ${c.id}, ${filename})?\n\n` +
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
        filename,
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
        generated and deployed with this tool). Edit one to tweak its stats/races/etc and
        redeploy in place, or delete one to clean up a clone that didn't work out.
      </p>
      <p className="hint">
        <strong>Stop the server before editing or deleting</strong> - the same database-lock
        caution as deploying applies here. Changing or removing a class still in use by an
        existing character will leave that character broken; only touch classes nothing is using
        yet.
      </p>
      <div className="actions">
        <button onClick={refresh} disabled={busy}>
          Refresh list
        </button>
      </div>
      {status && <pre className="status">{status}</pre>}

      {deployedClasses && deployedClasses.length > 0 && (
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
            {deployedClasses.map((c) => {
              const filename = c.filename ?? `${c.class_ident}.cs`;
              return (
                <tr key={filename}>
                  <td>{c.id}</td>
                  <td>{c.name}</td>
                  <td>{c.base_class_name}</td>
                  <td>
                    <code>{filename}</code>
                  </td>
                  <td>
                    <input
                      type="checkbox"
                      checked={cleanupByFile[filename] ?? true}
                      onChange={(e) =>
                        setCleanupByFile((prev) => ({ ...prev, [filename]: e.target.checked }))
                      }
                    />
                  </td>
                  <td>
                    <button onClick={() => edit(c)} disabled={busy}>
                      Edit
                    </button>{" "}
                    <button onClick={() => remove(c)} disabled={busy} className="danger">
                      Delete
                    </button>
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      )}
    </div>
  );
}
