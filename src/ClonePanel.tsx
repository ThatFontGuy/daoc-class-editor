import { useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ClassInfo, CloneRequest, KnownOptions, SshConfig, emptySshConfig } from "./types";

type Overrides = {
  profession: boolean;
  primary_stat: boolean;
  secondary_stat: boolean;
  tertiary_stat: boolean;
  mana_stat: boolean;
  ws_base: boolean;
  base_hp: boolean;
  specialization_multiplier: boolean;
  class_type: boolean;
  is_focus_caster: boolean;
  eligible_races: boolean;
};

const noOverrides: Overrides = {
  profession: false,
  primary_stat: false,
  secondary_stat: false,
  tertiary_stat: false,
  mana_stat: false,
  ws_base: false,
  base_hp: false,
  specialization_multiplier: false,
  class_type: false,
  is_focus_caster: false,
  eligible_races: false,
};

export default function ClonePanel() {
  const [classes, setClasses] = useState<ClassInfo[]>([]);
  const [options, setOptions] = useState<KnownOptions>({
    stats: [],
    races: [],
    class_types: [],
    race_realms: {},
  });
  const [sourceId, setSourceId] = useState<string>("");
  const [newName, setNewName] = useState("");
  const [newIdent, setNewIdent] = useState("");
  const [newId, setNewId] = useState<number>(0);
  const [overrides, setOverrides] = useState<Overrides>(noOverrides);
  const [values, setValues] = useState<Record<string, string | number | boolean | string[]>>({});
  const [code, setCode] = useState<string>("");
  const [filename, setFilename] = useState("");
  const [log, setLog] = useState<string>("");
  const [sshConfig, setSshConfig] = useState<SshConfig>(emptySshConfig);
  const [sql, setSql] = useState("");
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    invoke<ClassInfo[]>("list_classes").then(setClasses);
    invoke<KnownOptions>("known_options").then(setOptions);
    invoke<number>("suggest_next_id").then(setNewId);
    invoke<SshConfig>("load_ssh_config").then(setSshConfig);
  }, []);

  const source = useMemo(
    () => classes.find((c) => c.class_ident === sourceId),
    [classes, sourceId],
  );

  const byRealm = useMemo(() => {
    const groups: Record<string, ClassInfo[]> = {};
    for (const c of classes) {
      (groups[c.realm] ??= []).push(c);
    }
    return groups;
  }, [classes]);

  async function onPickSource(id: string) {
    setSourceId(id);
    setOverrides(noOverrides);
    setValues({});
    setCode("");
  }

  async function onNameChange(name: string) {
    setNewName(name);
    if (name.trim()) {
      const ident = await invoke<string>("sanitize_class_ident", { name });
      setNewIdent(`Class${ident}`);
    }
  }

  function toggle(key: keyof Overrides) {
    setOverrides({ ...overrides, [key]: !overrides[key] });
  }

  function setValue(key: string, v: string | number | boolean | string[]) {
    setValues({ ...values, [key]: v });
  }

  async function generate() {
    if (!source) return;
    const req: CloneRequest = {
      source,
      new_name: newName,
      new_class_ident: newIdent,
      new_id: newId,
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
    setCode(generated);
    setFilename(`${newIdent}.cs`);
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

  return (
    <div className="panel">
      <h2>1. Pick a class to clone</h2>
      <select value={sourceId} onChange={(e) => onPickSource(e.target.value)}>
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
            <strong>{source.name}</strong> ({source.class_ident}) - realm: {source.realm},
            profession: {source.profession ?? "-"}, stats: {source.primary_stat}/
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
              value={newName}
              onChange={(e) => onNameChange(e.target.value)}
              placeholder="e.g. Grovekeeper"
            />
          </label>
          <label className="field">
            <span>C# class name (auto-derived, editable)</span>
            <input value={newIdent} onChange={(e) => setNewIdent(e.target.value)} />
          </label>
          <label className="field">
            <span>Class ID (must be unused - next free one is suggested)</span>
            <input
              type="number"
              value={newId}
              onChange={(e) => setNewId(Number(e.target.value))}
            />
          </label>

          <h2>3. Everything below is inherited from {source.name} unless you check and change it</h2>
          <div className="override-grid">
            <OverrideRow
              label="Profession (flavor text)"
              checked={overrides.profession}
              onToggle={() => toggle("profession")}
            >
              <input
                defaultValue={source.profession ?? ""}
                onChange={(e) => setValue("profession", e.target.value)}
              />
            </OverrideRow>

            <OverrideRow
              label="Primary stat"
              checked={overrides.primary_stat}
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
              checked={overrides.secondary_stat}
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
              checked={overrides.tertiary_stat}
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
              checked={overrides.mana_stat}
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
              checked={overrides.ws_base}
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
              checked={overrides.base_hp}
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
              checked={overrides.specialization_multiplier}
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
              checked={overrides.class_type}
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
              checked={overrides.is_focus_caster}
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
              checked={overrides.eligible_races}
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
                          const current = (values.eligible_races as string[]) ?? [
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

          <button onClick={generate} disabled={!newName || !newIdent}>
            Generate code
          </button>

          {code && (
            <>
              <h2>4. Preview</h2>
              <textarea className="code-preview" readOnly value={code} rows={20} />

              <h2>5. Deploy</h2>
              <label className="field">
                <span>Filename in the server's scripts/ folder</span>
                <input value={filename} onChange={(e) => setFilename(e.target.value)} />
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
                    runAction(`Deploy ${filename}`, () =>
                      invoke<string>("deploy_class", {
                        cfg: sshConfig,
                        filename,
                        content: code,
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
