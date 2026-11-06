import { Fragment, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
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

export default function SpellEditorPanel() {
  const [sshConfig, setSshConfig] = useState<SshConfig>(emptySshConfig);
  const [deployedClasses, setDeployedClasses] = useState<ClassInfo[]>([]);
  const [selectedClassId, setSelectedClassId] = useState<number | null>(null);
  const [spells, setSpells] = useState<OwnedSpellRow[] | null>(null);
  const [editingSpellId, setEditingSpellId] = useState<number | null>(null);
  const [draft, setDraft] = useState<Record<string, string>>({});
  const [status, setStatus] = useState("");
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    invoke<SshConfig>("load_ssh_config").then(setSshConfig);
  }, []);

  async function refreshClasses() {
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
      setBusy(false);
    }
  }

  async function loadSpells(classId: number) {
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
      setBusy(false);
    }
  }

  async function migrate() {
    if (selectedClassId == null) return;
    setBusy(true);
    setStatus("Migrating this class's spells to independent copies...");
    try {
      const result = await invoke<string>("migrate_class_to_owned_spells", {
        cfg: sshConfig,
        classId: selectedClassId,
      });
      setStatus(result);
      await loadSpells(selectedClassId);
    } catch (e) {
      setStatus(`Migration failed: ${e}`);
    } finally {
      setBusy(false);
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
    if (editingSpellId == null) return;
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
      if (selectedClassId != null) await loadSpells(selectedClassId);
    } catch (e) {
      setStatus(`Save failed: ${e}`);
    } finally {
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
            Migrate to independent spells
          </button>
        )}
      </div>
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
