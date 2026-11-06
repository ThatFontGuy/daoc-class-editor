import { Fragment, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { OwnedSpellInLine, SpellLineGroup, SshConfig } from "./types";

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

/** Embeddable "this class's own spells, grouped by spell line" section - used by
 * ClassBrowserPanel's class-detail view. Field-editing a spell clones it independently for this
 * class on the spot the moment it's actually saved (see `save_line_spell_fields` on the Rust
 * side) - there's no separate "Migrate" step to run first; Save always just works. */
export default function ClassSpellsSection({
  sshConfig,
  classId,
}: {
  sshConfig: SshConfig;
  classId: number;
}) {
  const [spellLines, setSpellLines] = useState<SpellLineGroup[] | null>(null);
  const [editingSpellId, setEditingSpellId] = useState<number | null>(null);
  const [draft, setDraft] = useState<Record<string, string>>({});
  // Keyed by line_xspell_id - only populated once a level's own input is actually touched, so an
  // untouched row always reflects the real current level straight from `spellLines`.
  const [levelDrafts, setLevelDrafts] = useState<Record<string, string>>({});
  const [status, setStatus] = useState("");
  const [busy, setBusy] = useState(false);
  // A real, reported problem this guards against: a slow operation with no obvious visual
  // feedback invited a double-click on the same button, racing two overlapping calls against
  // each other. `busy` (React state) is what visually disables the button, but a very fast
  // second click can still slip in before a re-render actually paints that - a plain ref,
  // checked and set synchronously the instant a handler starts, closes that gap completely
  // regardless of render timing.
  const busyRef = useRef(false);

  useEffect(() => {
    load();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [classId]);

  async function load() {
    if (busyRef.current) return;
    busyRef.current = true;
    setBusy(true);
    setStatus("Loading this class's own spell lines...");
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
          ? "This class has no independently-clonable spec lines of its own yet - only specs " +
              "whose key ends in this class's own ID (created via Clone this class's spec-line " +
              "step) show up here."
          : "",
      );
    } catch (e) {
      setStatus(`Could not load spell lines: ${e}`);
      setSpellLines(null);
    } finally {
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
    setStatus(item.is_owned ? "Saving..." : "Cloning this spell for this class, then saving...");
    try {
      const result = await invoke<{ spell_id: number; message: string }>("save_line_spell_fields", {
        cfg: sshConfig,
        classId,
        lineXspellId: item.line_xspell_id,
        fields,
      });
      setStatus(result.message);
      setEditingSpellId(null);
      await load();
    } catch (e) {
      setStatus(`Save failed: ${e}`);
    } finally {
      busyRef.current = false;
      setBusy(false);
    }
  }

  async function saveLevel(item: OwnedSpellInLine) {
    if (busyRef.current) return;
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
        classId,
        lineXspellId: item.line_xspell_id,
        newLevel,
      });
      setStatus(result);
      await load();
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
      <h3>Spells</h3>
      <p className="hint">
        Grouped by this class's own spell lines, in level order, so a spec's actual progression is
        visible - the level shown is editable too. Editing a spell's own field values (damage,
        range, duration, ...) independently clones it for this class the moment you hit{" "}
        <strong>Save</strong>, if it isn't an independent copy yet - it never touches a spell still
        shared with the class it was cloned from (or any other class).
      </p>
      {busy && <p className="hint">⏳ Working - please wait...</p>}
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
                  <h4>
                    {group.line_name} <span className="hint">({group.spec_key})</span>
                  </h4>
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
                                  <span className="hint"> (shared - saving clones it for this class)</span>
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
                                  <button onClick={() => startEditing(item)} disabled={busy}>
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
