import { useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  ClassInfo,
  ClassListResult,
  ClassSpecRow,
  CloneRequest,
  KnownOptions,
  SpecAssignmentPayload,
  SpecializationRow,
  SshConfig,
  emptySshConfig,
} from "./types";
import { Overrides, SpecAssignmentUI, useProject } from "./ProjectContext";
import TrainerPanel from "./TrainerPanel";

export default function ClonePanel() {
  const [project, setProject] = useProject();
  const [classes, setClasses] = useState<ClassInfo[]>([]);
  const [classesLoading, setClassesLoading] = useState(false);
  const [deployedWarning, setDeployedWarning] = useState<string | null>(null);
  const [options, setOptions] = useState<KnownOptions>({
    stats: [],
    races: [],
    class_types: [],
    race_realms: {},
  });
  const [sshConfig, setSshConfig] = useState<SshConfig>(emptySshConfig);
  const [sql, setSql] = useState("");
  const [log, setLog] = useState<string>("");
  const [busy, setBusy] = useState(false);
  const logRef = useRef<HTMLPreElement>(null);

  // The log box has a fixed max-height and scrolls, so without this, each new action's result
  // (e.g. "Check status") lands below the fold of a box full of earlier entries and looks like
  // nothing happened at all - keep it pinned to the latest line whenever it grows.
  useEffect(() => {
    if (logRef.current) {
      logRef.current.scrollTop = logRef.current.scrollHeight;
    }
  }, [log]);

  const [allSpecs, setAllSpecs] = useState<SpecializationRow[]>([]);
  const [sourceSpecs, setSourceSpecs] = useState<ClassSpecRow[] | null>(null);
  const [specLog, setSpecLog] = useState("");

  async function refreshClasses(cfg: SshConfig) {
    setClassesLoading(true);
    try {
      const result = await invoke<ClassListResult>("list_classes_with_deployed", { cfg });
      setClasses(result.classes);
      setDeployedWarning(result.deployed_error);
    } catch (e) {
      setDeployedWarning(`Could not load classes: ${e}`);
    } finally {
      setClassesLoading(false);
    }
  }

  useEffect(() => {
    invoke<KnownOptions>("known_options").then(setOptions);
    invoke<SshConfig>("load_ssh_config").then((cfg) => {
      setSshConfig(cfg);
      refreshClasses(cfg);
      if (project.newId === 0) {
        invoke<number>("suggest_next_id", { cfg }).then((id) =>
          setProject((p) => (p.newId === 0 ? { ...p, newId: id } : p)),
        );
      }
    });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const source = useMemo(
    () => classes.find((c) => c.class_ident === project.sourceClassIdent),
    [classes, project.sourceClassIdent],
  );

  const isEditing = project.editingDeployedClassId !== null;
  // Step 3 reads "what specs does X currently have" from one class, and writes new assignments
  // to another - when editing an already-deployed class in place, both are that same class (its
  // specs are being read and modified directly); when designing a brand-new clone, specs are
  // read from the source (to show what's available to copy) but written to the new class's ID.
  const specsReadFromId = isEditing ? project.editingDeployedClassId! : source?.id;
  const specsWriteToId = isEditing ? project.editingDeployedClassId! : project.newId;
  const specsOwnerName = isEditing ? project.newName : (source?.name ?? "");
  // The picker's "active" highlight must track whatever class is actually being worked on - when
  // editing, that's the deployed class itself (newIdent), not its parent (sourceClassIdent, used
  // instead for the override grid's inherited-defaults lookup). Without this split, editing
  // Nosferatu highlighted Shadowblade as "selected" in the picker, which looked like the tool was
  // showing/editing the wrong class even though the values below it were correct.
  const activeClassIdent = isEditing ? project.newIdent : project.sourceClassIdent;

  // The base archetype files (ClassFighter, ClassAcolyte, ClassViking, ...) parse into ClassInfo
  // records too (classdata.rs needs them for the EligibleRaces inheritance fallback), but they
  // aren't playable classes themselves - only concrete classes that subclass one of them are
  // valid clone sources. "CharacterClassBase" is the literal C# parent every archetype file
  // itself extends, so anything still pointing at it directly is an archetype, not a leaf class.
  const leafClasses = useMemo(
    () => classes.filter((c) => c.parent_ident !== "CharacterClassBase"),
    [classes],
  );

  const REALM_ORDER = ["Albion", "Midgard", "Hibernia"];
  const realms = useMemo(() => {
    const present = new Set(leafClasses.map((c) => c.realm));
    return REALM_ORDER.filter((r) => present.has(r)).concat(
      [...present].filter((r) => !REALM_ORDER.includes(r)).sort(),
    );
  }, [leafClasses]);

  const [pickerRealm, setPickerRealm] = useState<string | null>(null);
  const [pickerBase, setPickerBase] = useState<string | null>(null);

  // Keep the picker's realm/base-class context pointed at whatever is actually selected (e.g.
  // right after loading a saved project), without resetting it just because the user clicks
  // around between realms/base classes while still looking for something else to clone.
  useEffect(() => {
    if (source) {
      setPickerRealm(source.realm);
      setPickerBase(source.base_class_name);
    }
  }, [source]);

  const baseClassesForRealm = useMemo(() => {
    if (!pickerRealm) return [];
    const counts = new Map<string, number>();
    for (const c of leafClasses) {
      if (c.realm === pickerRealm) {
        counts.set(c.base_class_name, (counts.get(c.base_class_name) ?? 0) + 1);
      }
    }
    return [...counts.entries()].sort(([a], [b]) => a.localeCompare(b));
  }, [leafClasses, pickerRealm]);

  const classesForBase = useMemo(() => {
    if (!pickerRealm || !pickerBase) return [];
    return leafClasses
      .filter((c) => c.realm === pickerRealm && c.base_class_name === pickerBase)
      .sort((a, b) => a.name.localeCompare(b.name));
  }, [leafClasses, pickerRealm, pickerBase]);

  function onPickRealm(realm: string) {
    setPickerRealm(realm);
    setPickerBase(null);
  }

  function onPickBase(base: string) {
    setPickerBase(base);
  }

  function onPickSource(id: string) {
    // Clicking the class already being edited (now correctly highlighted as active in the
    // picker) must stay a no-op - otherwise it silently exits edit mode and starts a brand-new
    // "clone from this class" instead, discarding its real overrides (ws_base/base_hp/etc.) in
    // favor of that class's own un-inherited raw fields. This is exactly how editing Nosferatu
    // could make its own correct values vanish just by clicking its (previously unhighlighted)
    // picker button.
    if (isEditing && id === project.newIdent) return;
    setProject((p) => ({
      ...p,
      sourceClassIdent: id,
      generatedCode: "",
      // Picking a fresh class from the browser always means "design a new class," even if a
      // deployed one was being edited a moment ago.
      editingDeployedClassId: null,
    }));
    setSourceSpecs(null);
  }

  async function onNameChange(name: string) {
    const ident = name.trim() ? await invoke<string>("sanitize_class_ident", { name }) : "";
    setProject((p) => ({
      ...p,
      newName: name,
      newIdent: ident ? `Class${ident}` : p.newIdent,
    }));
  }

  function toggle(key: keyof Overrides) {
    setProject((p) => ({ ...p, overrides: { ...p.overrides, [key]: !p.overrides[key] } }));
  }

  function setValue(key: string, v: string | number | boolean | string[]) {
    setProject((p) => ({ ...p, values: { ...p.values, [key]: v } }));
  }

  async function generate() {
    if (!source) return;
    const { overrides, values } = project;
    const req: CloneRequest = {
      source,
      new_name: project.newName,
      new_class_ident: project.newIdent,
      new_id: project.newId,
      profession: overrides.profession ? (values.profession as string) ?? null : null,
      primary_stat: overrides.primary_stat ? (values.primary_stat as string) ?? null : null,
      secondary_stat: overrides.secondary_stat ? (values.secondary_stat as string) ?? null : null,
      tertiary_stat: overrides.tertiary_stat ? (values.tertiary_stat as string) ?? null : null,
      mana_stat: overrides.mana_stat ? (values.mana_stat as string) ?? null : null,
      ws_base: overrides.ws_base ? Number(values.ws_base) : null,
      base_hp: overrides.base_hp ? Number(values.base_hp) : null,
      specialization_multiplier: overrides.specialization_multiplier
        ? Number(values.specialization_multiplier)
        : null,
      class_type: overrides.class_type ? (values.class_type as string) ?? null : null,
      is_focus_caster: overrides.is_focus_caster ? Boolean(values.is_focus_caster) : null,
      eligible_races: overrides.eligible_races ? (values.eligible_races as string[]) ?? [] : null,
    };
    const generated = await invoke<string>("generate_class_code", { req });
    setProject((p) => ({
      ...p,
      generatedCode: generated,
      // Preserve an already-set filename (notably while editing a deployed class in place, or
      // after the user has manually customized it in step 6) - only derive a fresh one the first
      // time there isn't one yet.
      filename: p.filename || `${project.newIdent}.cs`,
    }));
  }

  async function runAction(
    label: string,
    action: () => Promise<string>,
    onSuccess?: () => void,
  ) {
    setBusy(true);
    setLog((l) => `${l}\n> ${label}...`);
    try {
      const result = await action();
      setLog((l) => `${l}\n${result}`);
      onSuccess?.();
    } catch (e) {
      setLog((l) => `${l}\nERROR: ${e}`);
    } finally {
      setBusy(false);
    }
  }

  async function loadSourceSpecs() {
    if (!source || specsReadFromId === undefined) return;
    setSpecLog("loading...");
    try {
      const [all, mine] = await Promise.all([
        invoke<SpecializationRow[]>("list_specializations", { cfg: sshConfig }),
        invoke<ClassSpecRow[]>("list_class_specs", { cfg: sshConfig, classId: specsReadFromId }),
      ]);
      setAllSpecs(all);
      setSourceSpecs(mine);
      // Pre-fill assignments from the source class's own specs, defaulting the new name to the
      // source spec's own name (editable) - every assignment is always a clone now (see
      // specs.rs's module doc comment for why "reuse" was removed).
      if (project.specAssignments.length === 0) {
        setProject((p) => ({
          ...p,
          specAssignments: mine.map((m) => ({
            source_key: m.spec_key_name,
            source_name: m.spec_name,
            new_name: m.spec_name,
            level_acquired: m.level_acquired,
            applied: false,
          })),
          // Only meaningful while editing an already-deployed class in place (reading its own
          // specs back, not a parent's to copy from) - this snapshot is what apply_class_specs
          // diffs against to tell "removed from the list on purpose" apart from "never submitted
          // in the first place", so a spec dropped from the list here and redeployed is actually
          // removed on the server too, not left behind.
          originalAssignedSpecKeys: isEditing ? mine.map((m) => m.spec_key_name) : [],
        }));
      }
      setSpecLog(`found ${mine.length} spec(s) on ${specsOwnerName}`);
    } catch (e) {
      setSpecLog(`error: ${e}`);
    }
  }

  function updateSpecAssignment(index: number, patch: Partial<SpecAssignmentUI>) {
    setProject((p) => ({
      ...p,
      specAssignments: p.specAssignments.map((a, i) =>
        i === index ? { ...a, ...patch, applied: false } : a,
      ),
    }));
  }

  function removeSpecAssignment(index: number) {
    setProject((p) => ({
      ...p,
      specAssignments: p.specAssignments.filter((_, i) => i !== index),
    }));
  }

  function addBlankAssignment() {
    setProject((p) => ({
      ...p,
      specAssignments: [
        ...p.specAssignments,
        {
          source_key: allSpecs[0]?.key_name ?? "",
          source_name: allSpecs[0]?.name ?? "",
          new_name: allSpecs[0]?.name ?? "",
          level_acquired: 0,
          applied: false,
        },
      ],
    }));
  }

  // A reasonable, unique-enough DB key from whatever the user named the clone - doesn't need to
  // be pretty, just stable and safe for a SQL identifier; apply_class_specs always appends the
  // new class's own ID on top of this anyway, so collisions across classes aren't a concern.
  function deriveSpecKey(name: string): string {
    return name.replace(/[^A-Za-z0-9]/g, "").toLowerCase() || "spec";
  }

  async function applySpecsToServer() {
    if (specsReadFromId === undefined) return;
    setBusy(true);
    setSpecLog("applying...");
    try {
      const payload: SpecAssignmentPayload[] = project.specAssignments.map((a) => ({
        source_key: a.source_key,
        new_key: deriveSpecKey(a.new_name),
        new_name: a.new_name,
        level_acquired: a.level_acquired,
      }));
      const result = await invoke<string>("apply_class_specs", {
        cfg: sshConfig,
        newClassId: specsWriteToId,
        assignments: payload,
        previousKeys: project.originalAssignedSpecKeys,
      });
      setSpecLog(result || "done");
      setProject((p) => ({
        ...p,
        specAssignments: p.specAssignments.map((a) => ({ ...a, applied: true })),
        // Refresh the baseline to what's now actually assigned, so a second apply in the same
        // session (without reloading from the server first) diffs against reality instead of
        // the pre-first-apply snapshot - harmless either way (delete_existing_clone is a no-op
        // for a key that's already gone), but keeps the log free of redundant "removed" lines.
        originalAssignedSpecKeys: isEditing
          ? payload.map((p) => `${p.new_key}_${specsWriteToId}`)
          : p.originalAssignedSpecKeys,
      }));
    } catch (e) {
      setSpecLog(`error: ${e}`);
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="panel">
      <h2>1. Pick a class to clone</h2>
      <p className="hint">
        Includes any custom classes you've already deployed to the server (not just the ~63
        stock classes) - clone one of those again to refine it further.
      </p>
      <div className="actions">
        <button onClick={() => refreshClasses(sshConfig)} disabled={classesLoading}>
          {classesLoading ? "Refreshing..." : "Refresh class list"}
        </button>
      </div>
      {deployedWarning && (
        <p className="hint">
          Could not read deployed custom classes from the server ({deployedWarning}) - only the
          stock classes are shown below.
        </p>
      )}
      {isEditing ? (
        <div className="picker-breadcrumb">
          Editing <strong>{project.newName}</strong> (id {project.newId}) in place - its name,
          C# class name, ID, and filename are locked below so redeploying overwrites the same
          file. Use "New" at the top to design a separate class instead, or pick a different
          class below to abandon this edit.
        </div>
      ) : (
        source && (
          <div className="picker-breadcrumb">
            Cloning <strong>{source.name}</strong> ({source.realm}, {source.base_class_name}).
            Browse below to pick a different one, or continue to step 2.
          </div>
        )
      )}

      <div className="picker-step-label">Realm</div>
      <div className="picker-row">
        {realms.map((realm) => (
          <button
            key={realm}
            className={`picker-btn ${pickerRealm === realm ? "active" : ""}`}
            onClick={() => onPickRealm(realm)}
          >
            {realm}
          </button>
        ))}
      </div>

      {pickerRealm && (
        <>
          <div className="picker-step-label">
            Base class (every class below only sees races, specs, and abilities valid for{" "}
            {pickerRealm})
          </div>
          <div className="picker-row">
            {baseClassesForRealm.map(([base, count]) => (
              <button
                key={base}
                className={`picker-btn ${pickerBase === base ? "active" : ""}`}
                onClick={() => onPickBase(base)}
              >
                {base}
                <span className="count">
                  ({count} class{count === 1 ? "" : "es"})
                </span>
              </button>
            ))}
          </div>
        </>
      )}

      {pickerRealm && pickerBase && (
        <>
          <div className="picker-step-label">
            {pickerBase}-descended classes in {pickerRealm}
          </div>
          <div className="picker-row">
            {classesForBase.map((c) => (
              <button
                key={c.class_ident}
                className={`picker-btn ${activeClassIdent === c.class_ident ? "active" : ""}`}
                onClick={() => onPickSource(c.class_ident)}
              >
                {c.name}
                {c.is_custom && <span className="count">(your custom class)</span>}
              </button>
            ))}
          </div>
        </>
      )}

      {source && (
        <>
          <div className="source-summary">
            {isEditing && <em>Parent class ({project.newName} subclasses this): </em>}
            <strong>{source.name}</strong> ({source.class_ident}, ID {source.id}) - realm:{" "}
            {source.realm}, profession: {source.profession ?? "-"}, stats: {source.primary_stat}/
            {source.secondary_stat}/{source.tertiary_stat}, mana: {source.mana_stat ?? "-"}, WS
            base: {source.ws_base}, base HP: {source.base_hp}, type: {source.class_type}, focus
            caster: {String(source.is_focus_caster)}, races:{" "}
            {source.effective_eligible_races.length
              ? source.effective_eligible_races.join(", ")
              : "(none found)"}
          </div>

          <h2>2. Name your new class</h2>
          {isEditing && (
            <p className="hint">
              Locked while editing an existing deployed class - start a new clone (the "New"
              button above) if you want a different name, ID, or file instead.
            </p>
          )}
          <label className="field">
            <span>Display name (shown at character creation)</span>
            <input
              value={project.newName}
              onChange={(e) => onNameChange(e.target.value)}
              placeholder="e.g. Grovekeeper"
              disabled={isEditing}
            />
          </label>
          <label className="field">
            <span>C# class name (auto-derived, editable)</span>
            <input
              value={project.newIdent}
              onChange={(e) => setProject((p) => ({ ...p, newIdent: e.target.value }))}
              disabled={isEditing}
            />
          </label>
          <label className="field">
            <span>Class ID (must be unused - next free one is suggested)</span>
            <input
              type="number"
              value={project.newId}
              onChange={(e) => setProject((p) => ({ ...p, newId: Number(e.target.value) }))}
              disabled={isEditing}
            />
          </label>

          <h2>3. Spec lines</h2>
          <p className="hint">
            {isEditing ? (
              <>Showing {specsOwnerName}'s own currently-assigned specs - add more below.</>
            ) : (
              <>
                Specializations are assigned per class by raw numeric ID in the server's
                database, with no inheritance from the class you're cloning - without this step
                your new class would have no specs, no combat styles, and no weapon/armor
                proficiency at all, even though the generated .cs file correctly subclasses{" "}
                {source.name}.
              </>
            )}{" "}
            Every spec is always cloned - its own independent copy of the Specialization row,
            spell lines, and styles - which you can rename (e.g. "Celtic Dual" could become
            "Nosferatu's Claw") and, later, edit separately. (Sharing a spec verbatim with its
            source class used to be an option here, but a real, reproduced bug retired it: a
            class-locked style is only ever visible to the exact class ID it's locked to, so a
            shared spec whose styles happen to be locked elsewhere would train and show a level
            with no visible styles at all - invisibly. Cloning always repoints any class lock to
            your new class, so this can't happen.)
          </p>
          {sourceSpecs === null ? (
            <button onClick={loadSourceSpecs} disabled={busy}>
              Load {specsOwnerName}'s current specs
            </button>
          ) : (
            <>
              <table className="spec-table">
                <thead>
                  <tr>
                    <th>Source spec</th>
                    <th>New name</th>
                    <th>Level acquired</th>
                    <th></th>
                  </tr>
                </thead>
                <tbody>
                  {project.specAssignments.map((a, i) => (
                    <tr key={i}>
                      <td>
                        <select
                          value={a.source_key}
                          onChange={(e) => {
                            const picked = allSpecs.find((s) => s.key_name === e.target.value);
                            updateSpecAssignment(i, {
                              source_key: e.target.value,
                              source_name: picked?.name ?? e.target.value,
                            });
                          }}
                        >
                          {allSpecs.map((s) => (
                            <option value={s.key_name} key={s.key_name}>
                              {s.name}
                            </option>
                          ))}
                        </select>
                      </td>
                      <td>
                        <input
                          value={a.new_name}
                          placeholder="e.g. Nosferatu's Claw"
                          onChange={(e) => updateSpecAssignment(i, { new_name: e.target.value })}
                        />
                      </td>
                      <td>
                        <input
                          type="number"
                          value={a.level_acquired}
                          onChange={(e) =>
                            updateSpecAssignment(i, { level_acquired: Number(e.target.value) })
                          }
                        />
                      </td>
                      <td>
                        {a.applied ? (
                          <span className="mode-badge" style={{ marginBottom: 0 }}>
                            applied
                          </span>
                        ) : (
                          <button onClick={() => removeSpecAssignment(i)}>Remove</button>
                        )}
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
              <div className="actions">
                <button onClick={addBlankAssignment} disabled={allSpecs.length === 0}>
                  + Add another spec
                </button>
                <button
                  onClick={applySpecsToServer}
                  disabled={busy || project.specAssignments.length === 0}
                >
                  Apply spec assignments to server
                </button>
              </div>
              {specLog && <pre className="status">{specLog}</pre>}
            </>
          )}

          <h2>4. Everything below is inherited from {source.name} unless you check and change it</h2>
          <div className="override-grid" key={`${project.sourceClassIdent}:${project.editingDeployedClassId}`}>
            <OverrideRow
              label="Profession (flavor text)"
              checked={project.overrides.profession}
              onToggle={() => toggle("profession")}
            >
              <input
                defaultValue={(project.values.profession as string) ?? source.profession ?? ""}
                onChange={(e) => setValue("profession", e.target.value)}
              />
            </OverrideRow>

            <OverrideRow
              label="Primary stat"
              checked={project.overrides.primary_stat}
              onToggle={() => toggle("primary_stat")}
            >
              <StatSelect
                options={options.stats}
                defaultValue={(project.values.primary_stat as string) ?? source.primary_stat}
                onChange={(v) => setValue("primary_stat", v)}
              />
            </OverrideRow>

            <OverrideRow
              label="Secondary stat"
              checked={project.overrides.secondary_stat}
              onToggle={() => toggle("secondary_stat")}
            >
              <StatSelect
                options={options.stats}
                defaultValue={(project.values.secondary_stat as string) ?? source.secondary_stat}
                onChange={(v) => setValue("secondary_stat", v)}
              />
            </OverrideRow>

            <OverrideRow
              label="Tertiary stat"
              checked={project.overrides.tertiary_stat}
              onToggle={() => toggle("tertiary_stat")}
            >
              <StatSelect
                options={options.stats}
                defaultValue={(project.values.tertiary_stat as string) ?? source.tertiary_stat}
                onChange={(v) => setValue("tertiary_stat", v)}
              />
            </OverrideRow>

            <OverrideRow
              label="Mana stat"
              checked={project.overrides.mana_stat}
              onToggle={() => toggle("mana_stat")}
            >
              <StatSelect
                options={options.stats}
                defaultValue={(project.values.mana_stat as string) ?? source.mana_stat}
                onChange={(v) => setValue("mana_stat", v)}
              />
            </OverrideRow>

            <OverrideRow
              label="Weapon skill base"
              checked={project.overrides.ws_base}
              onToggle={() => toggle("ws_base")}
            >
              <input
                type="number"
                defaultValue={(project.values.ws_base as number) ?? source.ws_base ?? 0}
                onChange={(e) => setValue("ws_base", e.target.value)}
              />
            </OverrideRow>

            <OverrideRow
              label="Base HP"
              checked={project.overrides.base_hp}
              onToggle={() => toggle("base_hp")}
            >
              <input
                type="number"
                defaultValue={(project.values.base_hp as number) ?? source.base_hp ?? 0}
                onChange={(e) => setValue("base_hp", e.target.value)}
              />
            </OverrideRow>

            <OverrideRow
              label="Specialization multiplier"
              checked={project.overrides.specialization_multiplier}
              onToggle={() => toggle("specialization_multiplier")}
            >
              <input
                type="number"
                defaultValue={
                  (project.values.specialization_multiplier as number) ??
                  source.specialization_multiplier ??
                  10
                }
                onChange={(e) => setValue("specialization_multiplier", e.target.value)}
              />
            </OverrideRow>

            <OverrideRow
              label="Class type"
              checked={project.overrides.class_type}
              onToggle={() => toggle("class_type")}
            >
              <select
                defaultValue={(project.values.class_type as string) ?? source.class_type ?? ""}
                onChange={(e) => setValue("class_type", e.target.value)}
              >
                {options.class_types.map((t) => (
                  <option value={t} key={t}>
                    {t}
                  </option>
                ))}
              </select>
            </OverrideRow>

            <OverrideRow
              label="Focus caster"
              checked={project.overrides.is_focus_caster}
              onToggle={() => toggle("is_focus_caster")}
            >
              <select
                defaultValue={String(
                  (project.values.is_focus_caster as boolean | undefined) ??
                    source.is_focus_caster ??
                    false,
                )}
                onChange={(e) => setValue("is_focus_caster", e.target.value === "true")}
              >
                <option value="true">true</option>
                <option value="false">false</option>
              </select>
            </OverrideRow>

            <OverrideRow
              label="Eligible races"
              checked={project.overrides.eligible_races}
              onToggle={() => toggle("eligible_races")}
            >
              <div className="race-checkboxes">
                {options.races
                  .filter((r) => options.race_realms[r] === source.realm)
                  .map((r) => {
                    const current = (project.values.eligible_races as string[]) ?? [
                      ...source.effective_eligible_races,
                    ];
                    return (
                      <label key={r}>
                        <input
                          type="checkbox"
                          defaultChecked={current.includes(r)}
                          onChange={(e) => {
                            const next = e.target.checked
                              ? [...new Set([...current, r])]
                              : current.filter((x) => x !== r);
                            setValue("eligible_races", next);
                          }}
                        />
                        {r}
                      </label>
                    );
                  })}
              </div>
            </OverrideRow>
          </div>

          <button onClick={generate} disabled={!project.newName || !project.newIdent}>
            Generate code
          </button>

          {project.generatedCode && (
            <>
              <h2>5. Preview</h2>
              <textarea
                className="code-preview"
                readOnly
                value={project.generatedCode}
                rows={20}
              />

              <h2>6. Deploy</h2>
              {isEditing && (
                <p className="hint">
                  Deploying will overwrite the existing <code>{project.filename}</code> in place -
                  this is the redeploy step for the edit you just made.
                </p>
              )}
              <label className="field">
                <span>Filename in the server's bin/scripts/ folder</span>
                <input
                  value={project.filename}
                  onChange={(e) => setProject((p) => ({ ...p, filename: e.target.value }))}
                />
              </label>
              <div className="actions">
                <button
                  disabled={busy}
                  onClick={() =>
                    runAction("Check server status", () =>
                      invoke<string>("server_status", { cfg: sshConfig }),
                    )
                  }
                >
                  Check status
                </button>
                <button
                  disabled={busy}
                  onClick={() =>
                    runAction("Stop server", () =>
                      invoke<string>("server_control", { cfg: sshConfig, action: "stop" }),
                    )
                  }
                >
                  Stop server
                </button>
                <button
                  disabled={busy}
                  onClick={() =>
                    runAction(
                      `Deploy ${project.filename}`,
                      () =>
                        invoke<string>("deploy_class", {
                          cfg: sshConfig,
                          filename: project.filename,
                          content: project.generatedCode,
                        }),
                      () => refreshClasses(sshConfig),
                    )
                  }
                >
                  Deploy class file
                </button>
                <button
                  disabled={busy}
                  onClick={() =>
                    runAction("Start server", () =>
                      invoke<string>("server_control", { cfg: sshConfig, action: "start" }),
                    )
                  }
                >
                  Start server
                </button>
              </div>

              <h3>Advanced: run SQL on the server (optional)</h3>
              <p className="hint">
                For anything this tool doesn't have its own form for yet - e.g. starter
                equipment, a starting location, or a ServerProperty flag like the one that
                enabled Sluaghbinder (a trainer NPC has its own step below). Server should be
                stopped first for any write.
              </p>
              <textarea
                className="sql-box"
                value={sql}
                onChange={(e) => setSql(e.target.value)}
                placeholder='UPDATE ServerProperty SET Value="True" WHERE Key="...";'
                rows={3}
              />
              <button
                disabled={busy || !sql.trim()}
                onClick={() =>
                  runAction("Run SQL", () => invoke<string>("run_remote_sql", { cfg: sshConfig, sql }))
                }
              >
                Run SQL
              </button>

              {log && (
                <pre className="status" ref={logRef}>
                  {log.trim()}
                </pre>
              )}

              <TrainerPanel
                classId={project.newId}
                className={project.newName}
                source={source}
                classes={classes}
                sshConfig={sshConfig}
              />
            </>
          )}
        </>
      )}
    </div>
  );
}

function OverrideRow(props: {
  label: string;
  checked: boolean;
  onToggle: () => void;
  children: React.ReactNode;
}) {
  return (
    <div className="override-row">
      <label className="override-toggle">
        <input type="checkbox" checked={props.checked} onChange={props.onToggle} />
        {props.label}
      </label>
      <div className={props.checked ? "override-value" : "override-value disabled"}>
        {props.children}
      </div>
    </div>
  );
}

function StatSelect(props: {
  options: string[];
  defaultValue: string | null;
  onChange: (v: string) => void;
}) {
  return (
    <select defaultValue={props.defaultValue ?? ""} onChange={(e) => props.onChange(e.target.value)}>
      {props.options.map((s) => (
        <option value={s} key={s}>
          {s}
        </option>
      ))}
    </select>
  );
}
