import { useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  ClassInfo,
  ClassSpecRow,
  CloneRequest,
  KnownOptions,
  SpecAssignmentPayload,
  SpecializationRow,
  SshConfig,
  emptySshConfig,
} from "./types";
import { Overrides, SpecAssignmentUI, useProject } from "./ProjectContext";

export default function ClonePanel() {
  const [project, setProject] = useProject();
  const [classes, setClasses] = useState<ClassInfo[]>([]);
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

  const [allSpecs, setAllSpecs] = useState<SpecializationRow[]>([]);
  const [sourceSpecs, setSourceSpecs] = useState<ClassSpecRow[] | null>(null);
  const [specLog, setSpecLog] = useState("");

  useEffect(() => {
    invoke<ClassInfo[]>("list_classes").then(setClasses);
    invoke<KnownOptions>("known_options").then(setOptions);
    invoke<SshConfig>("load_ssh_config").then(setSshConfig);
    if (project.newId === 0) {
      invoke<number>("suggest_next_id").then((id) => setProject((p) => ({ ...p, newId: id })));
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const source = useMemo(
    () => classes.find((c) => c.class_ident === project.sourceClassIdent),
    [classes, project.sourceClassIdent],
  );

  const byRealm = useMemo(() => {
    const groups: Record<string, ClassInfo[]> = {};
    for (const c of classes) {
      (groups[c.realm] ??= []).push(c);
    }
    return groups;
  }, [classes]);

  function onPickSource(id: string) {
    setProject((p) => ({
      ...p,
      sourceClassIdent: id,
      generatedCode: "",
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
      base_class_name: source.base_class_name,
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
    setProject((p) => ({ ...p, generatedCode: generated, filename: `${project.newIdent}.cs` }));
  }

  async function runAction(label: string, action: () => Promise<string>) {
    setBusy(true);
    setLog((l) => `${l}\n> ${label}...`);
    try {
      const result = await action();
      setLog((l) => `${l}\n${result}`);
    } catch (e) {
      setLog((l) => `${l}\nERROR: ${e}`);
    } finally {
      setBusy(false);
    }
  }

  async function loadSourceSpecs() {
    if (!source) return;
    setSpecLog("loading...");
    try {
      const [all, mine] = await Promise.all([
        invoke<SpecializationRow[]>("list_specializations", { cfg: sshConfig }),
        invoke<ClassSpecRow[]>("list_class_specs", { cfg: sshConfig, classId: source.id }),
      ]);
      setAllSpecs(all);
      setSourceSpecs(mine);
      // Pre-fill assignments from the source class's own specs, defaulting every one to "reuse"
      // (same mechanics, shared with the source class) unless already customized.
      if (project.specAssignments.length === 0) {
        setProject((p) => ({
          ...p,
          specAssignments: mine.map((m) => ({
            source_key: m.spec_key_name,
            source_name: m.spec_name,
            mode: "reuse",
            new_key: "",
            new_name: "",
            level_acquired: m.level_acquired,
            applied: false,
          })),
        }));
      }
      setSpecLog(`found ${mine.length} spec(s) on ${source.name}`);
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
          mode: "reuse",
          new_key: "",
          new_name: "",
          level_acquired: 0,
          applied: false,
        },
      ],
    }));
  }

  async function applySpecsToServer() {
    if (!source) return;
    setBusy(true);
    setSpecLog("applying...");
    try {
      const payload: SpecAssignmentPayload[] = project.specAssignments.map((a) => ({
        source_key: a.source_key,
        mode: a.mode,
        new_key: a.mode === "clone" ? a.new_key : null,
        new_name: a.mode === "clone" ? a.new_name : null,
        level_acquired: a.level_acquired,
      }));
      const result = await invoke<string>("apply_class_specs", {
        cfg: sshConfig,
        newClassId: project.newId,
        sourceClassId: source.id,
        assignments: payload,
      });
      setSpecLog(result || "done");
      setProject((p) => ({
        ...p,
        specAssignments: p.specAssignments.map((a) => ({ ...a, applied: true })),
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
      <select value={project.sourceClassIdent} onChange={(e) => onPickSource(e.target.value)}>
        <option value="">-- choose a source class --</option>
        {Object.entries(byRealm).map(([realm, list]) => (
          <optgroup label={realm} key={realm}>
            {list.map((c) => (
              <option value={c.class_ident} key={c.class_ident}>
                {c.name} (base: {c.base_class_name})
              </option>
            ))}
          </optgroup>
        ))}
      </select>

      {source && (
        <>
          <div className="source-summary">
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
          <label className="field">
            <span>Display name (shown at character creation)</span>
            <input
              value={project.newName}
              onChange={(e) => onNameChange(e.target.value)}
              placeholder="e.g. Grovekeeper"
            />
          </label>
          <label className="field">
            <span>C# class name (auto-derived, editable)</span>
            <input
              value={project.newIdent}
              onChange={(e) => setProject((p) => ({ ...p, newIdent: e.target.value }))}
            />
          </label>
          <label className="field">
            <span>Class ID (must be unused - next free one is suggested)</span>
            <input
              type="number"
              value={project.newId}
              onChange={(e) => setProject((p) => ({ ...p, newId: Number(e.target.value) }))}
            />
          </label>

          <h2>3. Spec lines</h2>
          <p className="hint">
            Specializations are assigned per class by raw numeric ID in the server's database,
            with no inheritance from the class you're cloning - without this step your new class
            would have no specs, no combat styles, and no weapon/armor proficiency at all, even
            though the generated .cs file correctly subclasses {source.name}. "Reuse" shares the
            exact same spec (and any edits to it) with every other class that already has it;
            "Clone" makes an independent copy under a new name - its own spell lines/spells and
            styles - that you can rename and, later, edit separately.
          </p>
          {sourceSpecs === null ? (
            <button onClick={loadSourceSpecs} disabled={busy}>
              Load {source.name}'s current specs
            </button>
          ) : (
            <>
              <table className="spec-table">
                <thead>
                  <tr>
                    <th>Source spec</th>
                    <th>Mode</th>
                    <th>New name (if cloning)</th>
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
                        <select
                          value={a.mode}
                          onChange={(e) =>
                            updateSpecAssignment(i, { mode: e.target.value as "reuse" | "clone" })
                          }
                        >
                          <option value="reuse">Reuse</option>
                          <option value="clone">Clone &amp; rename</option>
                        </select>
                      </td>
                      <td>
                        <input
                          disabled={a.mode !== "clone"}
                          value={a.new_name}
                          placeholder={a.mode === "clone" ? "e.g. Earthshatter" : "-"}
                          onChange={(e) => {
                            const new_name = e.target.value;
                            const new_key = new_name
                              .toLowerCase()
                              .replace(/[^a-z0-9]+/g, "_")
                              .replace(/^_+|_+$/g, "");
                            updateSpecAssignment(i, { new_name, new_key });
                          }}
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
          <div className="override-grid">
            <OverrideRow
              label="Profession (flavor text)"
              checked={project.overrides.profession}
              onToggle={() => toggle("profession")}
            >
              <input
                defaultValue={source.profession ?? ""}
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
                defaultValue={source.primary_stat}
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
                defaultValue={source.secondary_stat}
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
                defaultValue={source.tertiary_stat}
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
                defaultValue={source.mana_stat}
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
                defaultValue={source.ws_base ?? 0}
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
                defaultValue={source.base_hp ?? 0}
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
                defaultValue={source.specialization_multiplier ?? 10}
                onChange={(e) => setValue("specialization_multiplier", e.target.value)}
              />
            </OverrideRow>

            <OverrideRow
              label="Class type"
              checked={project.overrides.class_type}
              onToggle={() => toggle("class_type")}
            >
              <select
                defaultValue={source.class_type ?? ""}
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
                defaultValue={String(source.is_focus_caster ?? false)}
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
                  .map((r) => (
                    <label key={r}>
                      <input
                        type="checkbox"
                        defaultChecked={source.effective_eligible_races.includes(r)}
                        onChange={(e) => {
                          const current = (project.values.eligible_races as string[]) ?? [
                            ...source.effective_eligible_races,
                          ];
                          const next = e.target.checked
                            ? [...new Set([...current, r])]
                            : current.filter((x) => x !== r);
                          setValue("eligible_races", next);
                        }}
                      />
                      {r}
                    </label>
                  ))}
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
              <label className="field">
                <span>Filename in the server's scripts/ folder</span>
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
                    runAction(`Deploy ${project.filename}`, () =>
                      invoke<string>("deploy_class", {
                        cfg: sshConfig,
                        filename: project.filename,
                        content: project.generatedCode,
                      }),
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
                For anything this tool doesn't have its own form for yet - e.g. a trainer NPC
                spawn row, starter equipment, or a ServerProperty flag like the one that enabled
                Sluaghbinder. Server should be stopped first for any write.
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

              {log && <pre className="status">{log.trim()}</pre>}
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
