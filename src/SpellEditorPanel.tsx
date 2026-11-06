import { Fragment, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { ClassInfo, ClassListResult, OwnedSpellRow, SshConfig, emptySshConfig } from "./types";

// A Spell row's real columns vary by server fork and aren't worth rendering all of - these are
// the fields someone tuning a cloned spell is actually likely to want to change. Any column not
// listed here still exists and is still copied faithfully by the clone, it's just not offered in
// this quick-edit form (the "Run SQL" box in Server connection can always reach it directly).
const EDITABLE_FIELD_HINTS = [
  "Name",
  "Description",
  "Damage",
  "DamageType",
  "Value",
  "Range",
  "Radius",
  "Duration",
  "Frequency",
  "Power",
  "CastTime",
  "RecastDelay",
  "Icon",
  "ClientEffect",
  "Target",
  "SpellType",
  "Message1",
  "Message2",
];

interface MigrateProgress {
  processed: number;
  total: number;
}

export default function SpellEditorPanel() {
  const [sshConfig, setSshConfig] = useState<SshConfig>(emptySshConfig);
  const [deployedClasses, setDeployedClasses] = useState<ClassInfo[]>([]);
  const [selectedClassId, setSelectedClassId] = useState<number | null>(null);
  const [spells, setSpells] = useState<OwnedSpellRow[] | null>(null);
  const [editingSpellId, setEditingSpellId] = useState<number | null>(null);
  const [draft, setDraft] = useState<Record<string, string>>({});
  const [status, setStatus] = useState("");
  const [busy, setBusy] = useState(false);
  const [migrateProgress, setMigrateProgress] = useState<MigrateProgress | null>(null);
  // A real, reported problem this guards against: a slow operation (migrating dozens of spells,
  // each its own round-trip) with no obvious visual feedback invited a double-click on the same
  // button, racing two overlapping calls against each other. `busy` (React state) is what
  // visually disables the button, but a very fast second click can still slip in before a
  // re-render actually paints that - a plain ref, checked and set synchronously the instant a
  // handler starts, closes that gap completely regardless of render timing.
  const busyRef = useRef(false);

  useEffect(() => {
    invoke<SshConfig>("load_ssh_config").then(setSshConfig);
  }, []);

  useEffect(() => {
    const unlisten = listen<MigrateProgress>("migrate-progress", (event) => {
      setMigrateProgress(event.payload);
    });
    return () => {
      unlisten.then((f) => f());
    };
  }, []);

  async function refreshClasses() {
    if (busyRef.current) return;
    busyRef.current = true;
    setBusy(true);
    setStatus("Loading deployed classes...");
    try {
      const result = await invoke<ClassListResult>("list_classes_with_deployed", {
        cfg: sshConfig,
      });
      setDeployedClasses(result.classes.filter((c) => c.is_custom));
      setStatus(result.deployed_error ?? "");
    } catch (e) {
      setStatus(`Could not list deployed classes: ${e}`);
    } finally {
      busyRef.current = false;
      setBusy(false);
    }
  }

  async function loadSpells(classId: number) {
    if (busyRef.current) return;
    busyRef.current = true;
    setSelectedClassId(classId);
    setEditingSpellId(null);
    setBusy(true);
    setStatus("Loading this class's own spells...");
    try {
      const rows = await invoke<OwnedSpellRow[]>("list_owned_spells", {
        cfg: sshConfig,
        classId,
      });
      setSpells(rows);
      setStatus(
        rows.length === 0
          ? "This class has no independently-owned spells yet - use Migrate below if it was " +
              "cloned before spell editing existed, or redeploy/edit its specs to create some."
          : "",
      );
    } catch (e) {
      setStatus(`Could not load spells: ${e}`);
      setSpells(null);
    } finally {
      busyRef.current = false;
      setBusy(false);
    }
  }

  async function migrate() {
    if (selectedClassId == null || busyRef.current) return;
    busyRef.current = true;
    setBusy(true);
    setMigrateProgress({ processed: 0, total: 0 });
    setStatus("Migrating this class's spells to independent copies - please wait, this can take a while...");
    try {
      const result = await invoke<string>("migrate_class_to_owned_spells", {
        cfg: sshConfig,
        classId: selectedClassId,
      });
      setStatus(result);
      await loadSpellsInner(selectedClassId);
    } catch (e) {
      setStatus(`Migration failed: ${e}`);
    } finally {
      setMigrateProgress(null);
      busyRef.current = false;
      setBusy(false);
    }
  }

  // `migrate` already holds the busy guard for its own whole operation (including the refresh
  // that follows) - this is the same body as `loadSpells` minus its own (would-be re-entrant)
  // guard/busy toggling.
  async function loadSpellsInner(classId: number) {
    setSelectedClassId(classId);
    setEditingSpellId(null);
    try {
      const rows = await invoke<OwnedSpellRow[]>("list_owned_spells", { cfg: sshConfig, classId });
      setSpells(rows);
    } catch (e) {
      setStatus(`Could not load spells: ${e}`);
      setSpells(null);
    }
  }

  function startEditing(spell: OwnedSpellRow) {
    const spellId = Number(spell.SpellID);
    setEditingSpellId(spellId);
    const initial: Record<string, string> = {};
    for (const [key, value] of Object.entries(spell)) {
      initial[key] = value === null || value === undefined ? "" : String(value);
    }
    setDraft(initial);
  }

  async function save(original: OwnedSpellRow) {
    if (editingSpellId == null || busyRef.current) return;
    // Only send fields that actually changed, and coerce back to a number when the original
    // value was one - the backend stores whatever JSON type is sent verbatim.
    const fields: Record<string, string | number> = {};
    for (const [key, value] of Object.entries(draft)) {
      if (key === "SpellID") continue;
      const originalValue = original[key];
      const originalAsString = originalValue === null || originalValue === undefined ? "" : String(originalValue);
      if (value === originalAsString) continue;
      const wasNumber = typeof originalValue === "number";
      fields[key] = wasNumber && value.trim() !== "" && !Number.isNaN(Number(value)) ? Number(value) : value;
    }
    if (Object.keys(fields).length === 0) {
      setStatus("Nothing changed.");
      setEditingSpellId(null);
      return;
    }
    busyRef.current = true;
    setBusy(true);
    setStatus("Saving...");
    try {
      const result = await invoke<string>("update_owned_spell_fields", {
        cfg: sshConfig,
        spellId: editingSpellId,
        fields,
      });
      setStatus(result);
      setEditingSpellId(null);
      if (selectedClassId != null) await loadSpellsInner(selectedClassId);
    } catch (e) {
      setStatus(`Save failed: ${e}`);
    } finally {
      busyRef.current = false;
      setBusy(false);
    }
  }

  const editableKeys = spells
    ? Object.keys(spells[0] ?? {}).filter((k) => EDITABLE_FIELD_HINTS.includes(k))
    : [];
  const fallbackKeys = spells ? Object.keys(spells[0] ?? {}).filter((k) => k !== "SpellID") : [];

  return (
    <div className="panel">
      <h2>Edit spells</h2>
      <p className="hint">
        Only spells this tool has <strong>independently cloned</strong> for a class are editable
        here - never a spell still shared with the class it was cloned from (or any other class).
        A class cloned before this feature existed needs a one-time <strong>Migrate</strong> first
        to convert its spells to independent copies.
      </p>
      <p className="hint">
        <strong>Stop the server before editing</strong> - same database-lock caution as deploying
        or editing a class.
      </p>
      <div className="actions">
        <button onClick={refreshClasses} disabled={busy}>
          Refresh deployed classes
        </button>
        <select
          value={selectedClassId ?? ""}
          onChange={(e) => e.target.value && loadSpells(Number(e.target.value))}
          disabled={busy || deployedClasses.length === 0}
        >
          <option value="">Pick a deployed class...</option>
          {deployedClasses.map((c) => (
            <option key={c.id} value={c.id}>
              {c.name} (id {c.id})
            </option>
          ))}
        </select>
        {selectedClassId != null && (
          <button onClick={migrate} disabled={busy}>
            {busy && migrateProgress ? "Migrating..." : "Migrate to independent spells"}
          </button>
        )}
      </div>
      {migrateProgress && (
        <div className="progress-indicator">
          <progress value={migrateProgress.processed} max={Math.max(migrateProgress.total, 1)} />
          <span>
            {migrateProgress.total === 0
              ? "Checking what needs migrating..."
              : `${migrateProgress.processed} / ${migrateProgress.total} spell(s)/proc(s) migrated - please wait, do not click again`}
          </span>
        </div>
      )}
      {busy && !migrateProgress && <p className="hint">⏳ Working - please wait...</p>}
      {status && <pre className="status">{status}</pre>}

      {spells && spells.length > 0 && (
        <table className="spec-table">
          <thead>
            <tr>
              <th>SpellID</th>
              <th>Name</th>
              <th></th>
            </tr>
          </thead>
          <tbody>
            {spells.map((spell) => {
              const spellId = Number(spell.SpellID);
              const isEditing = editingSpellId === spellId;
              return (
                <Fragment key={spellId}>
                  <tr>
                    <td>{spellId}</td>
                    <td>{String(spell.Name ?? "")}</td>
                    <td>
                      {isEditing ? (
                        <>
                          <button onClick={() => save(spell)} disabled={busy}>
                            Save
                          </button>{" "}
                          <button onClick={() => setEditingSpellId(null)} disabled={busy}>
                            Cancel
                          </button>
                        </>
                      ) : (
                        <button onClick={() => startEditing(spell)} disabled={busy}>
                          Edit
                        </button>
                      )}
                    </td>
                  </tr>
                  {isEditing && (
                    <tr>
                      <td colSpan={3}>
                        <div className="override-grid">
                          {(editableKeys.length > 0 ? editableKeys : fallbackKeys).map((key) => (
                            <label className="field" key={key}>
                              <span>{key}</span>
                              <input
                                value={draft[key] ?? ""}
                                onChange={(e) =>
                                  setDraft((prev) => ({ ...prev, [key]: e.target.value }))
                                }
                              />
                            </label>
                          ))}
                        </div>
                      </td>
                    </tr>
                  )}
                </Fragment>
              );
            })}
          </tbody>
        </table>
      )}
    </div>
  );
}
