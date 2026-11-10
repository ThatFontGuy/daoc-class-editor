import { Fragment, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  ClassSpecSummary,
  KnownOptions,
  OwnedSpellInLine,
  SpellLineGroup,
  SpellTypeFieldStat,
  SshConfig,
  StyleInfo,
  emptySshConfig,
} from "./types";
import { StatusState, emptyStatus, okStatus, errorStatus, StatusMessage } from "./Status";
import { useProject } from "./ProjectContext";

// A Spell row's real columns vary by server fork and aren't worth rendering all of - these are
// the fields someone tuning a cloned spell is actually likely to want to change. Any column not
// listed here still exists and is still copied faithfully by the clone, it's just not offered in
// this quick-edit form (the "Run SQL" box in Server connection can always reach it directly).
// Order here is the actual display order (Name first, then the longer text fields), not just a
// membership check - see PRIMARY_SPELL_FIELDS below for which of these get their own full-width
// row above the compact grid of the rest.
//
// "Type" (not "SpellType" - a real, found bug this list used to have, silently hiding the single
// most mechanically important field on every spell) is the actual dispatch key: the server parses
// it straight into the `eSpellType` enum and looks up whichever `[SpellHandler(eSpellType.X)]`-
// tagged class implements that category (e.g. "SpeedDecrease" -> SpeedDecreaseSpellHandler.cs) to
// actually apply the spell's effect - everything else here (Value, Duration, Range, ...) is just
// a parameter fed into whatever handler Type selects. ClientEffect/Icon, by contrast, are purely
// cosmetic - they only pick what the client shows, never anything about what the spell does.
//
// "Pulse" (not editable here until a real, reported gap: Frequency alone, with no way to set
// Pulse itself, left no way at all to actually turn a spell into a maintained/channeled one at
// all - confirmed live against SpeedEnhancementSpellHandler.cs's own source: Frequency is never
// even read unless Pulse != 0, so a one-shot spell's own Duration already fully controls how
// long it lasts in seconds on its own, with nothing else needed) flips a spell between a plain
// one-shot effect (Pulse = 0, Duration alone = how many seconds it lasts once cast) and a
// maintained/channeled one (Pulse != 0, re-applying - and re-charging PulsePower - every
// Frequency tenths-of-a-second for as long as it's kept running, e.g. a Minstrel song: dropping
// immediately on interrupt/movement/combat). PulsePower is the power cost charged per pulse -
// meaningless for a one-shot spell, since Pulse = 0 means it never pulses at all.
//
// "SpellGroup" (not editable here until a real, reported bug: "+ Add Next Spell"/"+ Add Spell
// Line" both need it, and so does fixing one by hand after the fact - this is the real signal
// the game client uses to treat several spells as "ranks of the same ability" for delve/tooltip
// display, confirmed live: several genuinely unrelated abilities ending up sharing one `SpellGroup`
// value, after a line's own existing spell was cloned as a starting template, left the client
// showing one ability's own description for all of them, despite each one's own real mechanics -
// which don't read `SpellGroup` at all - performing correctly throughout).
const EDITABLE_SPELL_FIELD_HINTS = [
  "Name",
  "Type",
  "Description",
  "Message1",
  "Message2",
  "Damage",
  "DamageType",
  "Value",
  "Range",
  "Radius",
  "Duration",
  "Pulse",
  "Frequency",
  "PulsePower",
  "Power",
  "CastTime",
  "RecastDelay",
  "Icon",
  "ClientEffect",
  "Target",
  "SpellGroup",
];

// These get their own full-width row, in this order, above the compact grid every other editable
// field is packed into - the ones actually likely to hold a longer string, plus (Name, Type) the
// two fields worth always seeing first regardless of their own length.
const PRIMARY_SPELL_FIELDS = ["Name", "Type", "Description", "Message1", "Message2"];

// Same idea as EDITABLE_SPELL_FIELD_HINTS, but for a Style (combat skill) row - the fields
// someone tuning a melee style is actually likely to want to change. SpecLevelRequirement is
// deliberately left out here - it has its own quick-edit in the Level column instead. Order here
// is the actual display order too.
const EDITABLE_STYLE_FIELD_HINTS = [
  "Name",
  "Icon",
  "EnduranceCost",
  "BonusToHit",
  "BonusToDefense",
  "GrowthOffset",
  "GrowthRate",
  "WeaponTypeRequirement",
  "AttackResultRequirement",
  "OpeningRequirementType",
  "OpeningRequirementValue",
  "StealthRequirement",
  "ArmorHitLocation",
  "RandomProc",
  "TwoHandAnimation",
];

const PRIMARY_STYLE_FIELDS = ["Name"];

/** Splits an already-ordered list of field keys into its own "primary" fields (Name, and for
 * spells the longer text fields) - each shown on its own full-width row - and everything else,
 * packed into a compact multi-column grid below them. Keeps the common rendering shape (one
 * editable-fields object) from having to be duplicated between the spell and style editors. */
function splitPrimaryFields(keys: string[], primary: string[]): { primaryKeys: string[]; restKeys: string[] } {
  return {
    primaryKeys: primary.filter((k) => keys.includes(k)),
    restKeys: keys.filter((k) => !primary.includes(k)),
  };
}

/** One spec's combat styles, fetched and edited only once its own row is actually expanded -
 * cloning/granting this class its own copy of a style on the spot the moment it's actually saved
 * (see `save_style_fields`/`update_style_level` on the Rust side), whether the spec is one of
 * this tool's own clones or a real stock class's spec that was never cloned through this tool at
 * all (e.g. Friar's own real "Staff"). */
function CombatSpecEditor({
  currentCfg,
  ensureCfg,
  classId,
  specKey,
}: {
  /** The config to read with right now - the active local design session if one already exists
   * for this class, otherwise the live server - without ever starting a new session on its own
   * (see `ensureCfg`'s own doc comment for why that distinction matters). */
  currentCfg: () => SshConfig;
  /** The config to write with - lazily starts this class's own local design session on its very
   * first call if none exists yet (see `ClassSpecsSection`'s own doc comment for the full
   * rationale), otherwise just returns the same one every other call this session already has.
   * Deliberately separate from `currentCfg` so merely viewing a spec (expanding "Edit" with no
   * actual change made) never silently starts a session nobody asked for - only an actual write
   * does. */
  ensureCfg: () => Promise<SshConfig>;
  classId: number;
  specKey: string;
}) {
  const [styles, setStyles] = useState<StyleInfo[] | null>(null);
  const [editingStyleId, setEditingStyleId] = useState<number | null>(null);
  const [draft, setDraft] = useState<Record<string, string>>({});
  const [levelDrafts, setLevelDrafts] = useState<Record<number, string>>({});
  const [status, setStatus] = useState<StatusState>(emptyStatus);
  const [busy, setBusy] = useState(false);
  const busyRef = useRef(false);

  useEffect(() => {
    load();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [specKey]);

  async function load() {
    if (busyRef.current) return;
    busyRef.current = true;
    setBusy(true);
    setEditingStyleId(null);
    setLevelDrafts({});
    try {
      const rows = await invoke<StyleInfo[]>("list_styles_for_spec", {
        cfg: currentCfg(),
        classId,
        specKey,
      });
      setStyles(rows);
      setStatus(rows.length === 0 ? okStatus("No combat styles found for this spec.") : emptyStatus);
    } catch (e) {
      setStatus(errorStatus(`Could not load styles: ${e}`));
      setStyles(null);
    } finally {
      busyRef.current = false;
      setBusy(false);
    }
  }

  function startEditing(s: StyleInfo) {
    setEditingStyleId(s.style_id);
    const initial: Record<string, string> = {};
    for (const [key, value] of Object.entries(s.fields)) {
      initial[key] = value === null || value === undefined ? "" : String(value);
    }
    setDraft(initial);
  }

  async function save(s: StyleInfo) {
    if (editingStyleId == null || busyRef.current) return;
    const fields: Record<string, string | number> = {};
    for (const [key, value] of Object.entries(draft)) {
      if (key === "StyleID") continue;
      const originalValue = s.fields[key];
      const originalAsString = originalValue === null || originalValue === undefined ? "" : String(originalValue);
      if (value === originalAsString) continue;
      const wasNumber = typeof originalValue === "number";
      fields[key] = wasNumber && value.trim() !== "" && !Number.isNaN(Number(value)) ? Number(value) : value;
    }
    // See the matching comment in SpellSpecEditor's own save() - this editor's Save button
    // covers a pending level change too while it's open, so there's no separate, easy-to-miss
    // second Save button right next to the Level input to remember to click too.
    const currentLevel = String(s.fields.SpecLevelRequirement ?? 0);
    const levelDraftValue = levelDrafts[s.style_id];
    const levelChanged = levelDraftValue !== undefined && levelDraftValue !== currentLevel;
    const newLevel = levelChanged ? Number(levelDraftValue) : null;
    if (levelChanged && (levelDraftValue.trim() === "" || Number.isNaN(newLevel))) {
      setStatus(errorStatus("Level must be a number."));
      return;
    }
    if (Object.keys(fields).length === 0 && !levelChanged) {
      setStatus(okStatus("Nothing changed."));
      setEditingStyleId(null);
      return;
    }
    busyRef.current = true;
    setBusy(true);
    setStatus(okStatus(s.is_owned ? "Saving..." : "Cloning/granting this style for this class, then saving..."));
    try {
      const cfg = await ensureCfg();
      const messages: string[] = [];
      if (Object.keys(fields).length > 0) {
        const result = await invoke<{ style_id: number; message: string }>("save_style_fields", {
          cfg,
          classId,
          specKey,
          styleId: s.style_id,
          fields,
        });
        messages.push(result.message);
      }
      if (levelChanged) {
        const result = await invoke<string>("update_style_level", {
          cfg,
          classId,
          specKey,
          styleId: s.style_id,
          newLevel,
        });
        messages.push(result);
      }
      setStatus(okStatus(messages.join(" ")));
      setEditingStyleId(null);
      // load() guards itself with the same busyRef to stop overlapping fetches from repeated
      // clicks - but that guard still reads true here (set above, not yet reset), so without
      // this reset load() would see itself as "already busy" and return immediately without
      // re-fetching anything at all. A real, reported bug this caused live: a successful save
      // silently left the UI showing stale pre-save data indefinitely, with no actual way to
      // tell (short of reopening the whole app) that the save had worked at all.
      busyRef.current = false;
      await load();
    } catch (e) {
      setStatus(errorStatus(`Save failed: ${e}`));
    } finally {
      busyRef.current = false;
      setBusy(false);
    }
  }

  async function saveLevel(s: StyleInfo) {
    if (busyRef.current) return;
    const currentLevel = String(s.fields.SpecLevelRequirement ?? 0);
    const draftValue = levelDrafts[s.style_id] ?? currentLevel;
    const newLevel = Number(draftValue);
    if (draftValue.trim() === "" || Number.isNaN(newLevel)) {
      setStatus(errorStatus("Level must be a number."));
      return;
    }
    busyRef.current = true;
    setBusy(true);
    setStatus(okStatus("Saving level..."));
    try {
      const cfg = await ensureCfg();
      const result = await invoke<string>("update_style_level", {
        cfg,
        classId,
        specKey,
        styleId: s.style_id,
        newLevel,
      });
      setStatus(okStatus(result));
      busyRef.current = false; // see the matching comment in save() above
      await load();
    } catch (e) {
      setStatus(errorStatus(`Save failed: ${e}`));
    } finally {
      busyRef.current = false;
      setBusy(false);
    }
  }

  /** Appends one more style to this spec, one level above whatever this class's own highest
   * style currently is (see `add_style_to_spec` on the Rust side for exactly how it's seeded) -
   * the "Edit" button alone only ever let an existing style be tuned, with no way at all to grow
   * a spec past however many styles it started out with. Opens the new style for editing right
   * away, same as a brand-new spec's own seeded placeholder already does. */
  async function addStyle() {
    if (busyRef.current) return;
    busyRef.current = true;
    setBusy(true);
    setStatus(okStatus("Adding a new style..."));
    try {
      const cfg = await ensureCfg();
      const added = await invoke<StyleInfo>("add_style_to_spec", {
        cfg,
        classId,
        specKey,
      });
      setStatus(okStatus("Added - edit it below to make it do what you actually want."));
      busyRef.current = false; // see the matching comment in save() above
      await load();
      startEditing(added);
    } catch (e) {
      setStatus(errorStatus(`Could not add a new style: ${e}`));
    } finally {
      busyRef.current = false;
      setBusy(false);
    }
  }

  return (
    <div>
      {busy && <p className="hint">⏳ Working - please wait...</p>}
      <StatusMessage status={status} />
      <p>
        <button onClick={addStyle} disabled={busy}>
          + Add style
        </button>
      </p>
      {styles && styles.length > 0 && (
        <table className="spec-table">
          <thead>
            <tr>
              <th>Level</th>
              <th>Name</th>
              <th></th>
            </tr>
          </thead>
          <tbody>
            {styles.map((s) => {
              const isEditingField = editingStyleId === s.style_id;
              const currentLevel = String(s.fields.SpecLevelRequirement ?? 0);
              const levelDraft = levelDrafts[s.style_id] ?? currentLevel;
              const levelChanged = levelDraft !== currentLevel;
              const editableKeys = EDITABLE_STYLE_FIELD_HINTS.filter((k) => k in s.fields);
              const fallbackKeys = Object.keys(s.fields).filter(
                (k) => k !== "StyleID" && k !== "SpecLevelRequirement",
              );
              const { primaryKeys, restKeys } = splitPrimaryFields(
                editableKeys.length > 0 ? editableKeys : fallbackKeys,
                PRIMARY_STYLE_FIELDS,
              );
              return (
                <Fragment key={s.style_id}>
                  <tr>
                    <td>
                      <input
                        type="number"
                        value={levelDraft}
                        onChange={(e) => setLevelDrafts((prev) => ({ ...prev, [s.style_id]: e.target.value }))}
                        style={{ width: "4em" }}
                        disabled={busy}
                      />{" "}
                      {levelChanged && !isEditingField && (
                        <button onClick={() => saveLevel(s)} disabled={busy}>
                          Save
                        </button>
                      )}
                    </td>
                    <td>
                      {String(s.fields.Name ?? "")}
                      {!s.is_owned && <span className="hint"> (shared - saving clones/grants it for this class)</span>}
                    </td>
                    <td>
                      {isEditingField ? (
                        <>
                          <button onClick={() => save(s)} disabled={busy}>
                            Save
                          </button>{" "}
                          <button onClick={() => setEditingStyleId(null)} disabled={busy}>
                            Cancel
                          </button>
                        </>
                      ) : (
                        <button onClick={() => startEditing(s)} disabled={busy}>
                          Edit
                        </button>
                      )}
                    </td>
                  </tr>
                  {isEditingField && (
                    <tr>
                      <td colSpan={3}>
                        {primaryKeys.map((key) => (
                          <label className="field" key={key}>
                            <span>{key}</span>
                            <input
                              value={draft[key] ?? ""}
                              onChange={(e) => setDraft((prev) => ({ ...prev, [key]: e.target.value }))}
                            />
                          </label>
                        ))}
                        <div className="compact-field-grid">
                          {restKeys.map((key) => (
                            <label className="field" key={key}>
                              <span>{key}</span>
                              <input
                                value={draft[key] ?? ""}
                                onChange={(e) => setDraft((prev) => ({ ...prev, [key]: e.target.value }))}
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

/** One spec's own spell lines, fetched and edited only once its own row is actually expanded -
 * mirrors `CombatSpecEditor` but for spells (cast spells, grouped by spell line in level order).
 * Save clones a spell independently for this class on the spot, if it isn't one already. */
function SpellSpecEditor({
  currentCfg,
  ensureCfg,
  classId,
  specKey,
  onSpecRenamed,
}: {
  currentCfg: () => SshConfig;
  ensureCfg: () => Promise<SshConfig>;
  classId: number;
  specKey: string;
  /** Called after a spell-line rename that also renamed its owning spec (see rename_spell_line's
   * own doc comment for when that happens) - lets the parent's own "Spells" summary list (a
   * separate fetch, this component's own internal load() never touches it) refresh too, so a
   * spec's own just-renamed display Name shows correctly there without having to collapse and
   * re-expand it first. */
  onSpecRenamed: () => void;
}) {
  const [groups, setGroups] = useState<SpellLineGroup[] | null>(null);
  const [editingSpellId, setEditingSpellId] = useState<number | null>(null);
  const [draft, setDraft] = useState<Record<string, string>>({});
  const [levelDrafts, setLevelDrafts] = useState<Record<string, string>>({});
  // Keyed by line_key - the spell line's own display Name, edited independently of any spell
  // within it. See renameLine's own doc comment for the real gap this fixes.
  const [lineNameDrafts, setLineNameDrafts] = useState<Record<string, string>>({});
  const [status, setStatus] = useState<StatusState>(emptyStatus);
  const [busy, setBusy] = useState(false);
  const [spellTypes, setSpellTypes] = useState<string[]>([]);
  // What's typical for whatever Type the spell currently being edited has - refreshed every time
  // Type changes, so the rest of the form can suggest (and, the first time Type is actually
  // picked after this field was first offered, auto-fill) values consistent with the rest of the
  // game instead of whatever a brand-new spell happened to be seeded from.
  const [typeStats, setTypeStats] = useState<Record<string, SpellTypeFieldStat>>({});
  const busyRef = useRef(false);

  useEffect(() => {
    invoke<KnownOptions>("known_options").then((opts) =>
      setSpellTypes([...opts.spell_types].sort((a, b) => a.localeCompare(b))),
    );
  }, []);

  useEffect(() => {
    load();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [classId, specKey]);

  async function load() {
    if (busyRef.current) return;
    busyRef.current = true;
    setBusy(true);
    setEditingSpellId(null);
    setLevelDrafts({});
    setLineNameDrafts({});
    try {
      const rows = await invoke<SpellLineGroup[]>("list_spell_lines_for_spec", {
        cfg: currentCfg(),
        classId,
        specKey,
      });
      setGroups(rows);
      const totalSpells = rows.reduce((sum, g) => sum + g.spells.length, 0);
      setStatus(totalSpells === 0 ? okStatus("No spells found on this spec's own spell line(s).") : emptyStatus);
    } catch (e) {
      setStatus(errorStatus(`Could not load spell lines: ${e}`));
      setGroups(null);
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
    // Populated fresh for whatever Type this spell already has, purely so its own stat-worthy
    // fields have a <datalist> of established values to pick from right away - unlike
    // onTypeChange below, opening the editor never overwrites anything already on the spell.
    setTypeStats({});
    void (async () => {
      try {
        setTypeStats(await invoke<Record<string, SpellTypeFieldStat>>("spell_type_field_stats", {
          cfg: currentCfg(),
          spellType: initial.Type ?? "",
        }));
      } catch {
        setTypeStats({});
      }
    })();
  }

  /** Re-picking a spell's own Type (including re-selecting the Type it already has, e.g. right
   * after it's seeded from an unrelated template spell - see specs.rs's own
   * `create_new_spell_spec` doc comment) re-seeds every stat-worthy field (Damage, Value, Range,
   * Duration, ...) with the *lowest* real value in use for it on a real spell of that Type - the
   * owner's own explicit preference: "default to the lower end values... and let the user ramp
   * them up as they add to it", not an average or most-common value that might already be a
   * mid-to-late-rank number. The three flavor-text fields (Description/Message1/Message2) are
   * cleared instead of suggested anything - they're spell-specific prose with no "typical value"
   * at all, and otherwise read as leftover text for whatever unrelated spell they were copied
   * from (the exact problem reported live). Name is never touched - this never has an opinion
   * about it either way. */
  async function onTypeChange(newType: string) {
    setDraft((prev) => ({ ...prev, Type: newType }));
    try {
      const stats = await invoke<Record<string, SpellTypeFieldStat>>("spell_type_field_stats", {
        cfg: currentCfg(),
        spellType: newType,
      });
      setTypeStats(stats);
      setDraft((prev) => {
        const next = { ...prev };
        for (const [field, stat] of Object.entries(stats)) next[field] = stat.default;
        next.Description = "";
        next.Message1 = "";
        next.Message2 = "";
        return next;
      });
    } catch {
      setTypeStats({});
    }
  }

  async function save(item: OwnedSpellInLine) {
    if (editingSpellId == null || busyRef.current) return;
    const fields: Record<string, string | number> = {};
    for (const [key, value] of Object.entries(draft)) {
      if (key === "SpellID") continue;
      const originalValue = item.fields[key];
      const originalAsString = originalValue === null || originalValue === undefined ? "" : String(originalValue);
      if (value === originalAsString) continue;
      const wasNumber = typeof originalValue === "number";
      fields[key] = wasNumber && value.trim() !== "" && !Number.isNaN(Number(value)) ? Number(value) : value;
    }
    // The level field editor's own Save button covers *everything* about this row at once while
    // it's open, including a pending level change - there's no separate, easy-to-miss second
    // Save button right next to the Level input to remember to click too (a real, reported
    // point of confusion: a brand-new spell's row opens pre-expanded for editing, so a level
    // change and a field change are now often made in the very same sitting). The Level column's
    // own standalone Save button (see renderDraftField's own table below) is hidden for exactly
    // this row for exactly this reason while this editor is open.
    const levelDraftValue = levelDrafts[item.line_xspell_id];
    const levelChanged = levelDraftValue !== undefined && levelDraftValue !== String(item.level);
    const newLevel = levelChanged ? Number(levelDraftValue) : null;
    if (levelChanged && (levelDraftValue.trim() === "" || Number.isNaN(newLevel))) {
      setStatus(errorStatus("Level must be a number."));
      return;
    }
    if (Object.keys(fields).length === 0 && !levelChanged) {
      setStatus(okStatus("Nothing changed."));
      setEditingSpellId(null);
      return;
    }
    busyRef.current = true;
    setBusy(true);
    setStatus(okStatus(item.is_owned ? "Saving..." : "Cloning this spell for this class, then saving..."));
    try {
      const cfg = await ensureCfg();
      const messages: string[] = [];
      if (Object.keys(fields).length > 0) {
        const result = await invoke<{ spell_id: number; message: string }>("save_line_spell_fields", {
          cfg,
          classId,
          lineXspellId: item.line_xspell_id,
          fields,
        });
        messages.push(result.message);
      }
      if (levelChanged) {
        const result = await invoke<string>("update_line_spell_level", {
          cfg,
          classId,
          lineXspellId: item.line_xspell_id,
          newLevel,
        });
        messages.push(result);
      }
      setStatus(okStatus(messages.join(" ")));
      setEditingSpellId(null);
      // load() guards itself with the same busyRef to stop overlapping fetches from repeated
      // clicks - but that guard still reads true here (set above, not yet reset), so without
      // this reset load() would see itself as "already busy" and return immediately without
      // re-fetching anything at all. A real, reported bug this caused live: a successful save
      // silently left the UI showing stale pre-save data indefinitely, with no actual way to
      // tell (short of reopening the whole app) that the save had worked at all.
      busyRef.current = false;
      await load();
    } catch (e) {
      setStatus(errorStatus(`Save failed: ${e}`));
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
      setStatus(errorStatus("Level must be a number."));
      return;
    }
    busyRef.current = true;
    setBusy(true);
    setStatus(okStatus("Saving level..."));
    try {
      const cfg = await ensureCfg();
      const result = await invoke<string>("update_line_spell_level", {
        cfg,
        classId,
        lineXspellId: item.line_xspell_id,
        newLevel,
      });
      setStatus(okStatus(result));
      busyRef.current = false; // see the matching comment in save() above
      await load();
    } catch (e) {
      setStatus(errorStatus(`Save failed: ${e}`));
    } finally {
      busyRef.current = false;
      setBusy(false);
    }
  }

  /** Starts a brand-new series within this line - a genuinely different ability from anything
   * else already in it, not another rank of one that's already there (see `addNextSpell`'s own
   * doc comment for growing an *existing* series instead). Gets its own fresh SpellGroup (see
   * `add_new_spell_series_to_line` on the Rust side for exactly why that matters) - this is the
   * "+ Add Spell Line" button, always visible above a line's own spell table. */
  async function addSpellSeries(group: SpellLineGroup) {
    if (busyRef.current) return;
    busyRef.current = true;
    setBusy(true);
    setStatus(okStatus("Adding a new spell series..."));
    try {
      const cfg = await ensureCfg();
      const added = await invoke<OwnedSpellInLine>("add_new_spell_series_to_line", {
        cfg,
        classId,
        specKey: group.spec_key,
        lineKey: group.line_key,
      });
      setStatus(okStatus("Added - edit it below to make it do what you actually want."));
      busyRef.current = false; // see the matching comment in save() above
      await load();
      startEditing(added);
    } catch (e) {
      setStatus(errorStatus(`Could not add a new spell series: ${e}`));
    } finally {
      busyRef.current = false;
      setBusy(false);
    }
  }

  /** Adds the next rank to the *same* series `item` already belongs to - the "+ Add Next Spell"
   * button right next to each individual spell's own Edit button, as opposed to `addSpellSeries`'s
   * own "start a whole new one." Inherits `item`'s own SpellGroup (see `add_next_spell_in_series`
   * on the Rust side for exactly why that matters - the real, reported bug this whole pair of
   * buttons exists to let the user avoid by construction, instead of having to notice and fix
   * SpellGroup by hand afterward). */
  async function addNextSpell(item: OwnedSpellInLine) {
    if (busyRef.current) return;
    busyRef.current = true;
    setBusy(true);
    setStatus(okStatus("Adding the next spell in this series..."));
    try {
      const cfg = await ensureCfg();
      const added = await invoke<OwnedSpellInLine>("add_next_spell_in_series", {
        cfg,
        classId,
        sourceLineXspellId: item.line_xspell_id,
      });
      setStatus(okStatus("Added - edit it below to make it do what you actually want."));
      busyRef.current = false; // see the matching comment in save() above
      await load();
      startEditing(added);
    } catch (e) {
      setStatus(errorStatus(`Could not add the next spell: ${e}`));
    } finally {
      busyRef.current = false;
      setBusy(false);
    }
  }

  /** Deletes one spell outright - the "Delete" button, right between each individual spell's own
   * Edit and "+ Add Next Spell" buttons. A real, reported gap: there was no way at all to remove
   * a spell once added, only to keep editing its fields in place forever. Always confirmed first
   * (same as every other destructive action in this tool), since there's no undo.
   *
   * Only ever enabled for an already-owned spell (`item.is_owned`, disabled otherwise below) -
   * the owner's own explicit preference: deleting should never be possible on a class's real,
   * original/stock content at all, only on something that's already this class's own independent
   * clone. `delete_spell_from_line` refuses the same way server-side regardless of this check. */
  async function deleteSpell(item: OwnedSpellInLine) {
    if (busyRef.current || !item.is_owned) return;
    const name = String(item.fields.Name ?? `spell ${item.spell_id}`);
    if (!confirm(`Delete "${name}" (level ${item.level}) entirely? This cannot be undone.`)) return;
    busyRef.current = true;
    setBusy(true);
    setStatus(okStatus("Deleting..."));
    try {
      const cfg = await ensureCfg();
      await invoke("delete_spell_from_line", {
        cfg,
        classId,
        lineXspellId: item.line_xspell_id,
      });
      setStatus(okStatus(`Deleted "${name}".`));
      busyRef.current = false; // see the matching comment in save() above
      await load();
    } catch (e) {
      setStatus(errorStatus(`Delete failed: ${e}`));
    } finally {
      busyRef.current = false;
      setBusy(false);
    }
  }

  /** Renames a spell line's own display Name - a real, reported gap: there was no way at all to
   * rename a line once created (short of recreating the whole thing from scratch), most visible
   * for the common case of a spec with only one spell line, where the line's own name wasn't
   * shown anywhere in this editor at all, only its owning spec's. */
  async function renameLine(group: SpellLineGroup) {
    if (busyRef.current) return;
    const draftValue = lineNameDrafts[group.line_key];
    if (draftValue === undefined || draftValue.trim() === group.line_name.trim()) return;
    busyRef.current = true;
    setBusy(true);
    setStatus(okStatus("Renaming..."));
    try {
      const cfg = await ensureCfg();
      const result = await invoke<string>("rename_spell_line", {
        cfg,
        classId,
        specKey: group.spec_key,
        lineKey: group.line_key,
        newName: draftValue,
      });
      setStatus(okStatus(result));
      busyRef.current = false; // see the matching comment in save() above
      await load();
      onSpecRenamed();
    } catch (e) {
      setStatus(errorStatus(`Rename failed: ${e}`));
    } finally {
      busyRef.current = false;
      setBusy(false);
    }
  }

  const totalSpells = groups?.reduce((sum, g) => sum + g.spells.length, 0) ?? 0;

  /** "Type" gets a dropdown of every real eSpellType value instead of a free-text field - it's
   * the single field that actually decides which handler (and therefore which real gameplay
   * effect) a spell uses, so picking it from a known-good list instead of typing it by hand
   * means there's no risk of a typo silently producing a spell the server can't even dispatch. */
  function renderDraftField(key: string) {
    if (key === "Type") {
      const current = draft[key] ?? "";
      return (
        <select value={current} onChange={(e) => void onTypeChange(e.target.value)}>
          {current !== "" && !spellTypes.includes(current) && <option value={current}>{current} (unrecognized)</option>}
          {spellTypes.map((t) => (
            <option key={t} value={t}>
              {t}
            </option>
          ))}
        </select>
      );
    }
    // A stat-worthy field (see specs.rs's own SPELL_TYPE_STAT_FIELDS) gets a <datalist> of every
    // real value already in use by some spell of the currently-selected Type - a one-click way to
    // stay consistent with the rest of the game (e.g. a speed buff's own Value), while direct
    // entry into the same input still works for a deliberately novel value.
    const stat = typeStats[key];
    const listId = stat ? `spell-field-options-${key}` : undefined;
    return (
      <>
        <input
          value={draft[key] ?? ""}
          list={listId}
          onChange={(e) => setDraft((prev) => ({ ...prev, [key]: e.target.value }))}
        />
        {stat && (
          <datalist id={listId}>
            {stat.options.map((o) => (
              <option key={o} value={o} />
            ))}
          </datalist>
        )}
      </>
    );
  }

  return (
    <div>
      {busy && <p className="hint">⏳ Working - please wait...</p>}
      <StatusMessage status={status} />
      {groups &&
        totalSpells > 0 &&
        groups
          .filter((group) => group.spells.length > 0)
          .map((group) => {
            const editableKeys = EDITABLE_SPELL_FIELD_HINTS.filter((k) => k in (group.spells[0]?.fields ?? {}));
            const fallbackKeys = Object.keys(group.spells[0]?.fields ?? {}).filter((k) => k !== "SpellID");
            const { primaryKeys, restKeys } = splitPrimaryFields(
              editableKeys.length > 0 ? editableKeys : fallbackKeys,
              PRIMARY_SPELL_FIELDS,
            );
            return (
              <div key={group.line_key}>
                <label className="field" style={{ maxWidth: "20em" }}>
                  <span>
                    Line name
                    {group.is_base_line && (
                      <span
                        title="Granted to every class with this spec regardless of spec points spent - scales with character level, not spec level."
                        style={{
                          marginLeft: "0.5em",
                          fontSize: "0.75em",
                          fontWeight: "normal",
                          color: "var(--muted, #888)",
                        }}
                      >
                        (base line - no spec investment needed)
                      </span>
                    )}
                  </span>
                  <input
                    value={lineNameDrafts[group.line_key] ?? group.line_name}
                    onChange={(e) => setLineNameDrafts((prev) => ({ ...prev, [group.line_key]: e.target.value }))}
                    disabled={busy}
                  />
                </label>
                {lineNameDrafts[group.line_key] !== undefined &&
                  lineNameDrafts[group.line_key].trim() !== group.line_name.trim() && (
                    <button onClick={() => renameLine(group)} disabled={busy}>
                      Save
                    </button>
                  )}
                <p>
                  <button onClick={() => addSpellSeries(group)} disabled={busy}>
                    + Add Spell Line
                  </button>
                </p>
                <table className="spec-table">
                  <thead>
                    <tr>
                      <th>Level</th>
                      <th title="The real signal the game client uses to treat several spells as ranks of the same ability for delve/tooltip display - see SpellGroup's own entry in the field editor below for the full explanation.">
                        SpellGroup
                      </th>
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
                                  setLevelDrafts((prev) => ({ ...prev, [item.line_xspell_id]: e.target.value }))
                                }
                                style={{ width: "4em" }}
                                disabled={busy}
                              />{" "}
                              {levelChanged && !isEditingField && (
                                <button onClick={() => saveLevel(item)} disabled={busy}>
                                  Save
                                </button>
                              )}
                            </td>
                            <td>{String(item.fields.SpellGroup ?? "")}</td>
                            <td>{item.spell_id}</td>
                            <td>
                              {String(item.fields.Name ?? "")}
                              {!item.is_owned && <span className="hint"> (shared - saving clones it for this class)</span>}
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
                                <>
                                  <button onClick={() => startEditing(item)} disabled={busy}>
                                    Edit
                                  </button>{" "}
                                  <button
                                    onClick={() => deleteSpell(item)}
                                    disabled={busy || !item.is_owned}
                                    className="danger"
                                    title={item.is_owned ? undefined : "Edit it first to clone it for this class, then it can be deleted"}
                                  >
                                    Delete
                                  </button>{" "}
                                  <button onClick={() => addNextSpell(item)} disabled={busy}>
                                    + Add Next Spell
                                  </button>
                                </>
                              )}
                            </td>
                          </tr>
                          {isEditingField && (
                            <tr>
                              <td colSpan={5}>
                                {primaryKeys.map((key) => (
                                  <label className="field" key={key}>
                                    <span>{key}</span>
                                    {renderDraftField(key)}
                                  </label>
                                ))}
                                <div className="compact-field-grid">
                                  {restKeys.map((key) => (
                                    <label className="field" key={key}>
                                      <span>{key}</span>
                                      {renderDraftField(key)}
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
              </div>
            );
          })}
    </div>
  );
}

/** One row in either summary list (combat specs or spells) - just enough to decide what to show
 * and, on request, expand into the real editor below it. */
function SpecSummaryRow({
  spec,
  expanded,
  onToggle,
  onDelete,
  busy,
  children,
}: {
  spec: ClassSpecSummary;
  expanded: boolean;
  onToggle: () => void;
  onDelete: () => void;
  busy: boolean;
  children: React.ReactNode;
}) {
  return (
    <Fragment>
      <tr>
        <td>{spec.name}</td>
        <td>{spec.level_acquired}</td>
        <td>{spec.is_owned ? "Independent" : "Shared"}</td>
        <td>
          <button onClick={onToggle} disabled={busy}>
            {expanded ? "Hide" : "Edit"}
          </button>{" "}
          <button
            onClick={onDelete}
            disabled={busy || !spec.is_owned}
            className="danger"
            title={spec.is_owned ? undefined : "This is still a shared/stock spec - clone it (edit any field) before it can be deleted"}
          >
            Delete
          </button>
        </td>
      </tr>
      {expanded && (
        <tr>
          <td colSpan={4}>{children}</td>
        </tr>
      )}
    </Fragment>
  );
}

/** A collapsed "Create" button that expands into a small inline Name + Level form - used for both
 * "New Combat Spec" and "New Spell Line" at the top of the section, each seeding one immediately
 * editable placeholder (see `create_new_combat_spec`/`create_new_spell_spec` on the Rust side for
 * what "placeholder" means here and why neither hands the user a literal blank form). */
function NewSpecForm({
  label,
  busy,
  onCreate,
}: {
  label: string;
  busy: boolean;
  onCreate: (name: string, levelAcquired: number) => Promise<void>;
}) {
  const [open, setOpen] = useState(false);
  const [name, setName] = useState("");
  const [level, setLevel] = useState(1);

  if (!open) {
    return (
      <button onClick={() => setOpen(true)} disabled={busy}>
        {label}
      </button>
    );
  }

  return (
    <div className="compact-field-grid">
      <label className="field">
        <span>Name</span>
        <input
          value={name}
          onChange={(e) => setName(e.target.value)}
          placeholder="e.g. Earthshatter"
          autoFocus
        />
      </label>
      <label className="field">
        <span>Level acquired</span>
        <input type="number" value={level} onChange={(e) => setLevel(Number(e.target.value))} />
      </label>
      <div>
        <button
          disabled={busy || !name.trim()}
          onClick={async () => {
            await onCreate(name.trim(), level);
            setName("");
            setLevel(1);
            setOpen(false);
          }}
        >
          Create
        </button>
        <button onClick={() => setOpen(false)} disabled={busy}>
          Cancel
        </button>
      </div>
    </div>
  );
}

/** Embeddable "this class's own specs" section - used by ClassBrowserPanel's class-detail view.
 * Shows two plain summary lists (names, level, and whether each spec is already independent or
 * still shared) - combat specs first, then spells - so the user sees just what's there before
 * picking what to actually expand and edit, instead of every spec's full data dumped on screen
 * at once. A spec with both combat styles and spell lines (rare) shows up in both lists.
 *
 * Owns this class's own local design session (see `design_session.rs`'s own module doc comment
 * for the full rationale, and `ClonePanel.tsx` for the original version of this same pattern):
 * the very first actual edit anywhere below this component - a field save, a level change, Add
 * spell/style, or creating a brand-new spec - lazily starts a disposable local snapshot and every
 * read/write from then on (for *this* class only) goes through it instead of the live server, so
 * designing never needs the live server stopped at all. "Migrate to live" (gated by the usual
 * `ensure_safe_to_write` check, same as every other real live write) reconciles everything
 * designed back onto live in one step once the server actually is stopped - see
 * `class_migration.rs`'s own module doc comment for exactly how. Switching to a different class
 * (`ClassBrowserPanel` remounts this component with a fresh `key` per class) discards any
 * still-open session for the class being left, via this component's own unmount cleanup -
 * there's deliberately no "resume this class's session later" persistence, unlike the Clone
 * Wizard's own project-scoped one, since a half-finished class design is expected to be either
 * migrated or abandoned within one sitting, not picked back up after restarting the app. */
export default function ClassSpecsSection({
  sshConfig,
  classId,
}: {
  sshConfig: SshConfig;
  classId: number;
}) {
  const [summaries, setSummaries] = useState<ClassSpecSummary[] | null>(null);
  const [status, setStatus] = useState<StatusState>(emptyStatus);
  const [busy, setBusy] = useState(false);
  const [expanded, setExpanded] = useState<Set<string>>(new Set());
  const busyRef = useRef(false);

  // This class's own local design session, read/written through the always-mounted project
  // context (ProjectContext.tsx's own classDesignSessions doc comment has the full rationale) -
  // *not* kept as this component's own local state. A real, reported bug this fixes: this
  // component (and the whole rest of the Classes tab around it) unmounts completely the moment
  // the user switches to a *different top-level tab* (App.tsx only conditionally renders it at
  // all), which is exactly what checking the Server connection tab's own Write safety status, or
  // clicking Backup Database, before migrating a finished design requires doing - local-only
  // state here would have been wiped by that totally ordinary action alone, with no way at all to
  // get back to it. Reading the project's own current value directly (not into a `useState`) so
  // switching classes or tabs and back always reflects whatever is actually still active right
  // now, never a stale snapshot from whenever this component instance first mounted.
  const [project, setProject] = useProject();
  const sessionDir = project.classDesignSessions[classId] ?? "";
  const sessionDirRef = useRef(sessionDir);
  sessionDirRef.current = sessionDir;
  const [migrateStatus, setMigrateStatus] = useState<StatusState>(emptyStatus);
  const [migrating, setMigrating] = useState(false);

  function sessionCfg(dir: string): SshConfig {
    // start_script_name must stay blank - this is the exact signal ensure_safe_to_write's own
    // design-session exception (see deploy.rs's own doc comment) relies on to recognize a
    // disposable local scratch copy and skip the real server-running/backup checks entirely
    // (there is nothing to start/stop in a bare snapshot directory). Spreading emptySshConfig
    // instead of hardcoding this was a real, reported bug: emptySshConfig's own start_script_name
    // defaults to "daoc-server.sh" (a sensible default for a *blank form*, not a design session),
    // so every call here after the very first (which got the real session object straight back
    // from start_design_session) silently reconstructed the wrong config - triggering the real
    // ensure_safe_to_write checks against a directory with no such script in it at all, and
    // failing outright with a raw "No such file or directory".
    return { ...emptySshConfig, remote_server_dir: dir, start_script_name: "" };
  }
  /** Read with right now, without ever starting a new session on its own - see
   * `CombatSpecEditor`'s own `currentCfg` prop doc comment for why that distinction matters. */
  function currentCfg(): SshConfig {
    return sessionDirRef.current ? sessionCfg(sessionDirRef.current) : sshConfig;
  }
  async function ensureCfg(): Promise<SshConfig> {
    if (sessionDirRef.current) return sessionCfg(sessionDirRef.current);
    const session = await invoke<SshConfig>("start_design_session", { cfg: sshConfig });
    sessionDirRef.current = session.remote_server_dir;
    setProject((p) => ({
      ...p,
      classDesignSessions: { ...p.classDesignSessions, [classId]: session.remote_server_dir },
    }));
    return session;
  }
  /** Explicit only - never called automatically on unmount (see this component's own session
   * fields' doc comment for exactly why that was a real, reported bug: an earlier version of
   * this discarded a session the instant its own editor component unmounted at all, which
   * includes every totally ordinary reason that can happen, not just "switched to a different
   * class"). Called only from the "Discard design" button itself, and from migrateToLive once
   * everything in it has actually been migrated. */
  async function discardDesignSession() {
    const dir = sessionDirRef.current;
    if (!dir) return;
    sessionDirRef.current = "";
    setProject((p) => {
      const next = { ...p.classDesignSessions };
      delete next[classId];
      return { ...p, classDesignSessions: next };
    });
    try {
      await invoke("discard_design_session", { sessionCfg: sessionCfg(dir) });
    } catch {
      // Best-effort cleanup only - an orphaned session directory left on disk is harmless (it's
      // never reused once classDesignSessions no longer references it), not worth surfacing as
      // an error over.
    }
  }

  useEffect(() => {
    load();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [classId]);

  async function load() {
    if (busyRef.current) return;
    busyRef.current = true;
    setBusy(true);
    setStatus(okStatus("Loading this class's specs..."));
    try {
      const rows = await invoke<ClassSpecSummary[]>("list_class_spec_summaries", {
        cfg: currentCfg(),
        classId,
      });
      setSummaries(rows);
      setStatus(rows.length === 0 ? okStatus("This class has no specs assigned.") : emptyStatus);
    } catch (e) {
      setStatus(errorStatus(`Could not load specs: ${e}`));
      setSummaries(null);
    } finally {
      busyRef.current = false;
      setBusy(false);
    }
  }

  function toggle(key: string) {
    setExpanded((prev) => {
      const next = new Set(prev);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    });
  }

  /** Creates a brand-new spec (combat or spell, per `command`) and immediately expands it so its
   * one seeded placeholder (a Style or Spell) is right there to edit - there is no separate "now
   * go find what you just made" step. */
  async function createSpec(command: "create_new_combat_spec" | "create_new_spell_spec", name: string, levelAcquired: number) {
    setBusy(true);
    setStatus(okStatus(`Creating "${name}"...`));
    try {
      const cfg = await ensureCfg();
      const summary = await invoke<ClassSpecSummary>(command, {
        cfg,
        classId,
        name,
        levelAcquired,
      });
      await load();
      setExpanded((prev) => new Set(prev).add(summary.key_name));
      setStatus(okStatus(`Created "${summary.name}" - edit its placeholder below to make it do what you actually want.`));
    } catch (e) {
      setStatus(errorStatus(`Could not create "${name}": ${e}`));
    } finally {
      setBusy(false);
    }
  }

  /** Deletes a spec outright - the "Delete" button next to each spec's own Edit/Hide toggle in
   * the "Combat specs"/"Spells" summary lists. A real, reported gap: there was no way to remove a
   * spec from the Classes tab at all, short of going through the Clone Wizard's own whole-list
   * resubmission flow instead. Always confirmed first, since there's no undo.
   *
   * Only ever enabled for an already-owned/independent spec (`spec.is_owned`, disabled otherwise
   * below) - the owner's own explicit preference: deleting should never be possible on a class's
   * real, original/stock content at all, only on something that's already this class's own
   * independent clone. A still-shared spec can still be dropped from this class via the Clone
   * Wizard's own spec list instead. `delete_class_spec` refuses the same way server-side
   * regardless of this check. */
  async function deleteSpec(spec: ClassSpecSummary) {
    if (!spec.is_owned) return;
    if (
      !confirm(
        `Delete "${spec.name}" from this class entirely?\n\n` +
          "This removes its whole spec/spell line/spell/style data - it's this class's own independent copy, nothing else uses it." +
          "\n\nAny existing character already using this class would have its points in this spec reset (a free respec is granted). This cannot be undone.",
      )
    )
      return;
    setBusy(true);
    setStatus(okStatus(`Deleting "${spec.name}"...`));
    try {
      const cfg = await ensureCfg();
      const result = await invoke<string>("delete_class_spec", {
        cfg,
        classId,
        specKey: spec.key_name,
      });
      setStatus(okStatus(result));
      setExpanded((prev) => {
        const next = new Set(prev);
        next.delete(spec.key_name);
        return next;
      });
      await load();
    } catch (e) {
      setStatus(errorStatus(`Delete failed: ${e}`));
    } finally {
      setBusy(false);
    }
  }

  // A spec with styles is always a combat spec, full stop, even if it also happens to carry an
  // incidental SpellLine of its own (a real, reproduced case: "Shields" has both 157 Style rows
  // *and* a "Warrior Shields (Live)" spell line, presumably internal bookkeeping for some
  // block/bash-related effect) - from a player's perspective that's still obviously a combat
  // skill, not something to also list as a "spell." Combat classification takes priority; a spec
  // only shows as a Spell if it has no styles at all.
  const combatSpecs = (summaries ?? []).filter((s) => s.has_styles);
  const spellSpecs = (summaries ?? []).filter((s) => s.has_spell_lines && !s.has_styles);

  /** Reconciles everything designed in this class's own local session back onto the real live
   * server (`class_migration::migrate_class_design_to_live`) - gated by the usual
   * `ensure_safe_to_write` check (server stopped + a sufficiently fresh backup) at the Tauri
   * command boundary, same as any other live write. Discards the now fully-migrated session
   * afterward and reloads from live, so the summary list immediately reflects the real live
   * state rather than the (now-stale) local one. */
  async function migrateToLive() {
    const dir = sessionDirRef.current;
    if (!dir) return;
    setMigrating(true);
    setMigrateStatus(okStatus("Migrating this class's design to live..."));
    try {
      const result = await invoke<string>("migrate_class_design_to_live", {
        sessionCfg: sessionCfg(dir),
        liveCfg: sshConfig,
        classId,
      });
      await discardDesignSession();
      setMigrateStatus(okStatus(result));
      await load();
    } catch (e) {
      setMigrateStatus(errorStatus(`Migrate failed: ${e}`));
    } finally {
      setMigrating(false);
    }
  }

  return (
    <div className="panel">
      {busy && <p className="hint">⏳ Working - please wait...</p>}
      <StatusMessage status={status} />

      {sessionDir && (
        <div className="panel" style={{ background: "var(--panel-alt, #2a2a2a)", marginBottom: "1rem" }}>
          <p>
            <strong>🧪 Designing locally</strong> - every edit below is going to a disposable local
            copy, not the live server. Nothing here is visible to players until you migrate it.
          </p>
          <StatusMessage status={migrateStatus} />
          <button onClick={migrateToLive} disabled={migrating}>
            Migrate to live
          </button>{" "}
          <button
            onClick={async () => {
              await discardDesignSession();
              setMigrateStatus(emptyStatus);
              await load();
            }}
            disabled={migrating}
          >
            Discard design
          </button>
          <p className="hint">
            <strong>Migrate to live</strong> requires the server stopped and a fresh backup (the
            same write-safety check every other live change already goes through) - everything
            you've designed is reconciled onto the real database in one step, then this local
            session is cleaned up automatically.
          </p>
        </div>
      )}

      <div className="new-spec-actions">
        <h3>Create from scratch</h3>
        <p className="hint">
          Adds a brand-new spec to this class - not cloned from anything else - seeded with one
          placeholder (a style or spell) ready to rename and tune below. Works for any class,
          standard or cloned, the same as editing an existing spec below already does.
        </p>
        <NewSpecForm label="New Combat Spec" busy={busy} onCreate={(name, lvl) => createSpec("create_new_combat_spec", name, lvl)} />{" "}
        <NewSpecForm label="New Spell Line" busy={busy} onCreate={(name, lvl) => createSpec("create_new_spell_spec", name, lvl)} />
      </div>

      {combatSpecs.length > 0 && (
        <div>
          <h3>Combat specs</h3>
          <p className="hint">
            Specs that grant combat styles (melee skills). Click <strong>Edit</strong> on one to
            see and edit just its own styles - "Shared" means editing it will clone/grant this
            class its own copy on Save, without touching any other class's.
          </p>
          <table className="spec-table">
            <thead>
              <tr>
                <th>Name</th>
                <th>Level</th>
                <th>Status</th>
                <th></th>
              </tr>
            </thead>
            <tbody>
              {combatSpecs.map((s) => (
                <SpecSummaryRow
                  key={`combat-${s.key_name}`}
                  spec={s}
                  expanded={expanded.has(s.key_name)}
                  onToggle={() => toggle(s.key_name)}
                  onDelete={() => deleteSpec(s)}
                  busy={busy}
                >
                  <CombatSpecEditor currentCfg={currentCfg} ensureCfg={ensureCfg} classId={classId} specKey={s.key_name} />
                </SpecSummaryRow>
              ))}
            </tbody>
          </table>
        </div>
      )}

      {spellSpecs.length > 0 && (
        <div>
          <h3>Spells</h3>
          <p className="hint">
            Specs that grant spells. Click <strong>Edit</strong> on one to see and edit just its
            own spell line(s) - "Shared" means editing it will clone this class its own
            independent copy on Save, without touching any other class's.
          </p>
          <table className="spec-table">
            <thead>
              <tr>
                <th>Name</th>
                <th>Level</th>
                <th>Status</th>
                <th></th>
              </tr>
            </thead>
            <tbody>
              {spellSpecs.map((s) => (
                <SpecSummaryRow
                  key={`spell-${s.key_name}`}
                  spec={s}
                  expanded={expanded.has(s.key_name)}
                  onToggle={() => toggle(s.key_name)}
                  onDelete={() => deleteSpec(s)}
                  busy={busy}
                >
                  <SpellSpecEditor currentCfg={currentCfg} ensureCfg={ensureCfg} classId={classId} specKey={s.key_name} onSpecRenamed={load} />
                </SpecSummaryRow>
              ))}
            </tbody>
          </table>
        </div>
      )}

      {summaries && combatSpecs.length === 0 && spellSpecs.length === 0 && (
        <p className="hint">This class has no combat specs or spells to show.</p>
      )}
    </div>
  );
}
