import { Fragment, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import {
  ClassInfo,
  ClassListResult,
  OwnedSpellInLine,
  SpellLineGroup,
  SshConfig,
  emptySshConfig,
} from "./types";

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
  const [spellLines, setSpellLines] = useState<SpellLineGroup[] | null>(null);
  const [editingSpellId, setEditingSpellId] = useState<number | null>(null);
  const [draft, setDraft] = useState<Record<string, string>>({});
  // Keyed by line_xspell_id - only populated once a level's own input is actually touched, so an
  // untouched row always reflects the real current level straight from `spellLines`.
  const [levelDrafts, setLevelDrafts] = useState<Record<string, string>>({});
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

  async function loadSpellLines(classId: number) {
    if (busyRef.current) return;
    busyRef.current = true;
    setBusy(true);
    setStatus("Loading this class's own spell lines...");
    try {
      await loadSpellLinesInner(classId);
    } finally {
      busyRef.current = false;
      setBusy(false);
    }
  }

  // Shared body for both the class picker and the post-migrate/post-save refresh - callers that
  // already hold the busy guard for their own whole operation (migrate, save) call this directly
  // instead of `loadSpellLines`, which would otherwise treat itself as a re-entrant call and
  // no-op.
  async function loadSpellLinesInner(classId: number) {
    setSelectedClassId(classId);
    setEditingSpellId(null);
    setLevelDrafts({});
    try {
      const groups = await invoke<SpellLineGroup[]>("list_owned_spell_lines", {
        cfg: sshConfig,
        classId,
      });
      setSpellLines(groups);
      const totalSpells = groups.reduce((sum, g) => sum + g.spells.length, 0);
      setStatus(
        totalSpells === 0
          ? "No spell lines found yet for this class's own specs - use Migrate below if it was " +
              "cloned before spell editing existed, or redeploy/edit its specs to create some."
          : "",
      );
    } catch (e) {
      setStatus(`Could not load spell lines: ${e}`);
      setSpellLines(null);
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
      await loadSpellLinesInner(selectedClassId);
    } catch (e) {
      setStatus(`Migration failed: ${e}`);
    } finally {
      setMigrateProgress(null);
      busyRef.current = false;
      setBusy(false);
    }
  }

  function startEditing(item: OwnedSpellInLine) {
    setEditingSpellId(item.spell_id);
    const initial: Record<string, string> = {};
    for (const [key, value] of Object.entries(item.fields)) {
      initial[key] = value === null || value === undefined ? "" : String(value);
    }
    setDraft(initial);
  }

  async function save(item: OwnedSpellInLine) {
    if (editingSpellId == null || busyRef.current) return;
    // Only send fields that actually changed, and coerce back to a number when the original
    // value was one - the backend stores whatever JSON type is sent verbatim.
    const fields: Record<string, string | number> = {};
    for (const [key, value] of Object.entries(draft)) {
      if (key === "SpellID") continue;
      const originalValue = item.fields[key];
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
        spellId: item.spell_id,
        fields,
      });
      setStatus(result);
      setEditingSpellId(null);
      if (selectedClassId != null) await loadSpellLinesInner(selectedClassId);
    } catch (e) {
      setStatus(`Save failed: ${e}`);
    } finally {
      busyRef.current = false;
      setBusy(false);
    }
  }

  async function saveLevel(item: OwnedSpellInLine) {
    if (selectedClassId == null || busyRef.current) return;
    const draftValue = levelDrafts[item.line_xspell_id];
    const newLevel = Number(draftValue);
    if (draftValue === undefined || draftValue.trim() === "" || Number.isNaN(newLevel)) {
      setStatus("Level must be a number.");
      return;
    }
    busyRef.current = true;
    setBusy(true);
    setStatus("Saving level...");
    try {
      const result = await invoke<string>("update_line_spell_level", {
        cfg: sshConfig,
        classId: selectedClassId,
        lineXspellId: item.line_xspell_id,
        newLevel,
      });
      setStatus(result);
      await loadSpellLinesInner(selectedClassId);
    } catch (e) {
      setStatus(`Save failed: ${e}`);
    } finally {
      busyRef.current = false;
      setBusy(false);
    }
  }

  const totalSpells = spellLines?.reduce((sum, g) => sum + g.spells.length, 0) ?? 0;

  return (
    <div className="panel">
      <h2>Edit spells</h2>
      <p className="hint">
        Grouped by this class's own spell lines (e.g. Drengr's three real lines), in level order,
        so a spec's actual progression is visible - the level shown is editable too. Field-editing
        a spell's own values (damage, range, duration, ...) only ever touches a spell this tool
        has <strong>independently cloned</strong> for this class - never one still shared with the
        class it was cloned from (or any other class). A class cloned before this feature existed
        needs a one-time <strong>Migrate</strong> first to convert its spells to independent
        copies.
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
          onChange={(e) => e.target.value && loadSpellLines(Number(e.target.value))}
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

      {spellLines && totalSpells > 0 && (
        <>
          {spellLines
            .filter((group) => group.spells.length > 0)
            .map((group) => {
              const editableKeys = Object.keys(group.spells[0]?.fields ?? {}).filter((k) =>
                EDITABLE_FIELD_HINTS.includes(k),
              );
              const fallbackKeys = Object.keys(group.spells[0]?.fields ?? {}).filter(
                (k) => k !== "SpellID",
              );
              return (
                <div key={group.line_key}>
                  <h3>
                    {group.line_name} <span className="hint">({group.spec_key})</span>
                  </h3>
                  <table className="spec-table">
                    <thead>
                      <tr>
                        <th>Level</th>
                        <th>SpellID</th>
                        <th>Name</th>
                        <th></th>
                      </tr>
                    </thead>
                    <tbody>
                      {group.spells.map((item) => {
                        const isEditingField = editingSpellId === item.spell_id;
                        const levelDraft = levelDrafts[item.line_xspell_id] ?? String(item.level);
                        const levelChanged = levelDraft !== String(item.level);
                        return (
                          <Fragment key={item.line_xspell_id}>
                            <tr>
                              <td>
                                <input
                                  type="number"
                                  value={levelDraft}
                                  onChange={(e) =>
                                    setLevelDrafts((prev) => ({
                                      ...prev,
                                      [item.line_xspell_id]: e.target.value,
                                    }))
                                  }
                                  style={{ width: "4em" }}
                                  disabled={busy}
                                />{" "}
                                {levelChanged && (
                                  <button onClick={() => saveLevel(item)} disabled={busy}>
                                    Save
                                  </button>
                                )}
                              </td>
                              <td>{item.spell_id}</td>
                              <td>
                                {String(item.fields.Name ?? "")}
                                {!item.is_owned && (
                                  <span className="hint"> (not yet migrated - read-only)</span>
                                )}
                              </td>
                              <td>
                                {isEditingField ? (
                                  <>
                                    <button onClick={() => save(item)} disabled={busy}>
                                      Save
                                    </button>{" "}
                                    <button onClick={() => setEditingSpellId(null)} disabled={busy}>
                                      Cancel
                                    </button>
                                  </>
                                ) : (
                                  <button
                                    onClick={() => startEditing(item)}
                                    disabled={busy || !item.is_owned}
                                  >
                                    Edit
                                  </button>
                                )}
                              </td>
                            </tr>
                            {isEditingField && (
                              <tr>
                                <td colSpan={4}>
                                  <div className="override-grid">
                                    {(editableKeys.length > 0 ? editableKeys : fallbackKeys).map(
                                      (key) => (
                                        <label className="field" key={key}>
                                          <span>{key}</span>
                                          <input
                                            value={draft[key] ?? ""}
                                            onChange={(e) =>
                                              setDraft((prev) => ({
                                                ...prev,
                                                [key]: e.target.value,
                                              }))
                                            }
                                          />
                                        </label>
                                      ),
                                    )}
                                  </div>
                                </td>
                              </tr>
                            )}
                          </Fragment>
                        );
                      })}
                    </tbody>
                  </table>
                </div>
              );
            })}
        </>
      )}
    </div>
  );
}
