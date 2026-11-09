import { useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ClassInfo, ClassListResult, REALM_ORDER, SshConfig, emptySshConfig, leafClassesOf } from "./types";
import { buildEditingProjectState, useProject } from "./ProjectContext";
import ClassSpecsSection from "./ClassSpecsSection";

/** The single, unified "every class in the game" browser - replaces the old separate "Deployed
 * classes" and "Edit spells" tabs. Lists every class (the ~63 vendored reference classes this
 * tool ships with, plus anything already deployed with it), organized by realm, each tagged with
 * its real status - no more needing to already know a native-slot class's numeric ID (e.g.
 * Drengr) just to reach it, since it's vendored and shows up here by name like any other class.
 * Selecting one shows its details plus its own spells, right below, in one place. */
export default function ClassBrowserPanel({ onNavigateToClone }: { onNavigateToClone: () => void }) {
  const [, setProject] = useProject();
  const [sshConfig, setSshConfig] = useState<SshConfig>(emptySshConfig);
  const [classes, setClasses] = useState<ClassInfo[]>([]);
  const [disabledClassIds, setDisabledClassIds] = useState<Set<number>>(new Set());
  const [selectedIdent, setSelectedIdent] = useState<string | null>(null);
  const [status, setStatus] = useState("");
  const [busy, setBusy] = useState(false);
  const [cleanup, setCleanup] = useState(true);
  // Bumped on "Update All Class Data" so the embedded spells section remounts and reloads too,
  // not just the class list itself.
  const [spellsRefreshKey, setSpellsRefreshKey] = useState(0);

  useEffect(() => {
    invoke<SshConfig>("load_ssh_config").then((cfg) => {
      setSshConfig(cfg);
      refresh(cfg);
    });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  async function refresh(cfg: SshConfig) {
    setBusy(true);
    setStatus("Loading classes...");
    try {
      const result = await invoke<ClassListResult>("list_classes_with_deployed", { cfg });
      setClasses(result.classes);
      setStatus(result.deployed_error ? `Could not read deployed classes: ${result.deployed_error}` : "");
    } catch (e) {
      setStatus(`Could not list classes: ${e}`);
    } finally {
      setBusy(false);
    }
    // Best-effort, separate from the class list itself - a server fork without this property (or
    // an unreachable connection) shouldn't block browsing the class list at all, it just means
    // nothing gets marked disabled.
    try {
      const ids = await invoke<number[]>("list_disabled_class_ids", { cfg });
      setDisabledClassIds(new Set(ids));
    } catch {
      setDisabledClassIds(new Set());
    }
  }

  async function updateAllClassData() {
    await refresh(sshConfig);
    setSpellsRefreshKey((k) => k + 1);
  }

  const leafClasses = useMemo(() => leafClassesOf(classes), [classes]);
  const realms = useMemo(() => {
    const present = new Set(leafClasses.map((c) => c.realm));
    return REALM_ORDER.filter((r) => present.has(r)).concat(
      [...present].filter((r) => !REALM_ORDER.includes(r)).sort(),
    );
  }, [leafClasses]);

  const selected = useMemo(
    () => leafClasses.find((c) => c.class_ident === selectedIdent) ?? null,
    [leafClasses, selectedIdent],
  );

  function cloneThisClass() {
    if (!selected) return;
    setProject((p) => ({
      ...p,
      sourceClassIdent: selected.class_ident,
      generatedCode: "",
      editingDeployedClassId: null,
    }));
    onNavigateToClone();
  }

  function editClassFile() {
    if (!selected) return;
    const parent = classes.find((p) => p.class_ident === selected.parent_ident);
    if (!parent) {
      setStatus(
        `Could not find ${selected.parent_ident} (the class ${selected.name} subclasses) in the ` +
          "current class list - try Update All Class Data first.",
      );
      return;
    }
    // Preserves classDesignSessions across this switch - buildEditingProjectState otherwise
    // resets the whole project state wholesale (it spreads emptyProjectState as its own base),
    // which would silently lose track of any Classes-tab design session still active for *any*
    // class the moment "Edit class file" is clicked for this one - the actual directory on disk
    // would still exist, but nothing would know to look for it again (see
    // ProjectContext.tsx's own classDesignSessions doc comment for the related, already-reported
    // bug this same class of mistake caused live).
    setProject((p) => ({ ...buildEditingProjectState(selected, parent), classDesignSessions: p.classDesignSessions }));
    onNavigateToClone();
  }

  async function deleteThisClass() {
    if (!selected) return;
    const filename = selected.filename ?? `${selected.class_ident}.cs`;
    const warning =
      `Delete "${selected.name}" (id ${selected.id}, ${filename})?\n\n` +
      "This removes the class file from bin/scripts/ and its spec assignments" +
      (cleanup ? ", plus every spec line/spell line/style this tool cloned for it." : ".") +
      "\n\nAny existing character already using this class would be left in a broken state - " +
      "only delete classes with no characters on them, and stop the server first.";
    if (!confirm(warning)) return;

    setBusy(true);
    setStatus(`Deleting ${selected.name}...`);
    try {
      const result = await invoke<string>("delete_deployed_class", {
        cfg: sshConfig,
        filename,
        classId: selected.id,
        cleanupClonedSpecs: cleanup,
      });
      setStatus(result);
      setSelectedIdent(null);
      await refresh(sshConfig);
    } catch (e) {
      setStatus(`Delete failed: ${e}`);
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="panel">
      {!selected && (
        <>
          <h2>Classes</h2>
          <p className="hint">
            Every class in the game, organized by realm - the ~63 built-in classes this tool
            ships reference data for, plus anything you've already deployed with it. Pick one to
            see its details and its own spells, clone it as a starting point for a new class, or
            (for a class you deployed yourself) edit its file in place or delete it. A class
            marked <span className="badge-disabled">DISABLED</span> is currently blocked at
            character creation for regular players (the server's own "disabled_classes"/feature-
            flag settings) - a GM account can still create one anyway. A class shown struck
            through with <span className="badge-disabled">→ [name]</span> is a retired native
            client slot that's been superseded by that other, actively-used class entirely -
            never actually creatable regardless of any setting.
          </p>
          <div className="actions">
            <button onClick={updateAllClassData} disabled={busy}>
              Update All Class Data
            </button>
          </div>
          {status && <pre className="status">{status}</pre>}

          {realms.map((realm) => {
            const inRealm = leafClasses
              .filter((c) => c.realm === realm)
              .sort((a, b) => a.name.localeCompare(b.name));
            if (inRealm.length === 0) return null;
            return (
              <div key={realm} className={`realm-${realm.toLowerCase()}`}>
                <h3>{realm}</h3>
                <table className="spec-table">
                  <thead>
                    <tr>
                      <th>ID</th>
                      <th>Name</th>
                      <th>Status</th>
                      <th>Profession</th>
                      <th>Based on</th>
                      <th></th>
                    </tr>
                  </thead>
                  <tbody>
                    {inRealm.map((c) => {
                      const disabled = disabledClassIds.has(c.id);
                      return (
                        <tr key={c.class_ident}>
                          <td>{c.id}</td>
                          <td>
                            {c.superseded_by ? (
                              <>
                                <span style={{ textDecoration: "line-through" }}>{c.name}</span>{" "}
                                <span className="badge-disabled">→ {c.superseded_by}</span>
                              </>
                            ) : (
                              <>
                                {c.name}
                                {disabled && <span className="badge-disabled"> DISABLED</span>}
                              </>
                            )}
                          </td>
                          <td>{c.is_custom ? "Cloned Class" : "Standard Class"}</td>
                          <td>{c.profession ?? "-"}</td>
                          <td>{c.base_class_name}</td>
                          <td>
                            <button onClick={() => setSelectedIdent(c.class_ident)} disabled={busy}>
                              View
                            </button>
                          </td>
                        </tr>
                      );
                    })}
                  </tbody>
                </table>
              </div>
            );
          })}
        </>
      )}

      {selected && (
        <div className={`panel realm-${selected.realm.toLowerCase()}`}>
          <div className="actions" style={{ justifyContent: "space-between" }}>
            <h2 style={{ margin: 0 }}>
              {selected.superseded_by ? (
                <span style={{ textDecoration: "line-through" }}>{selected.name}</span>
              ) : (
                selected.name
              )}{" "}
              <span className="hint">(ID {selected.id})</span>
            </h2>
            <button onClick={() => setSelectedIdent(null)} disabled={busy}>
              Return to Class List
            </button>
          </div>
          <p>
            <strong>{selected.is_custom ? "Cloned Class" : "Standard Class"}</strong> - realm:{" "}
            {selected.realm}, profession: {selected.profession ?? "-"}, based on:{" "}
            {selected.base_class_name} ({selected.parent_ident})
            {selected.superseded_by ? (
              <span className="badge-disabled"> → {selected.superseded_by}</span>
            ) : (
              disabledClassIds.has(selected.id) && <span className="badge-disabled"> DISABLED</span>
            )}
          </p>
          <div className="actions">
            <button onClick={cloneThisClass} disabled={busy}>
              Clone this class
            </button>
            {selected.is_custom && (
              <>
                <button onClick={editClassFile} disabled={busy}>
                  Edit class file
                </button>
                <label
                  className="field"
                  style={{ display: "inline-flex", flexDirection: "row", gap: "0.3rem", alignItems: "center" }}
                >
                  <input
                    type="checkbox"
                    checked={cleanup}
                    onChange={(e) => setCleanup(e.target.checked)}
                  />
                  <span>Also remove cloned spec lines</span>
                </label>
                <button onClick={deleteThisClass} disabled={busy} className="danger">
                  Delete class
                </button>
              </>
            )}
            <button onClick={updateAllClassData} disabled={busy}>
              Update All Class Data
            </button>
          </div>
          <p className="hint">
            <strong>Stop the server before editing or deleting</strong> - the same database-lock
            caution as deploying applies here. Changing or removing a class still in use by an
            existing character will leave that character broken; only touch classes nothing is
            using yet.
          </p>
          {status && <pre className="status">{status}</pre>}
          <ClassSpecsSection
            key={`${selected.id}-${spellsRefreshKey}`}
            sshConfig={sshConfig}
            classId={selected.id}
          />
        </div>
      )}
    </div>
  );
}
