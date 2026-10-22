import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ClassInfo, SshConfig, TrainerNpcCandidate, TrainerRequest } from "./types";
import { useProject } from "./ProjectContext";

export default function TrainerPanel({
  classId,
  className,
  source,
  classes,
  sshConfig,
}: {
  classId: number;
  className: string;
  source: ClassInfo;
  classes: ClassInfo[];
  sshConfig: SshConfig;
}) {
  const [project, setProject] = useProject();
  const trainer = project.trainer;
  const [archetype, setArchetype] = useState<ClassInfo | null>(null);
  const [archetypeError, setArchetypeError] = useState("");
  const [busy, setBusy] = useState(false);
  const [log, setLog] = useState("");
  const [candidates, setCandidates] = useState<TrainerNpcCandidate[] | null>(null);

  function setTrainer(patch: Partial<typeof trainer>) {
    setProject((p) => ({ ...p, trainer: { ...p.trainer, ...patch } }));
  }

  async function enable() {
    setArchetypeError("");
    try {
      // Nothing beyond the game client's built-in class list is directly selectable at
      // creation - the player creates this archetype instead, and the trainer promotes them.
      // Resolving from the *source* (the class actually cloned, e.g. Shadowblade), not classId
      // itself, since a brand-new class that hasn't been deployed yet has no entry of its own
      // in `classes` to walk up from - but it would resolve to the exact same archetype anyway.
      const resolved = await invoke<ClassInfo | null>("root_archetype", {
        classes,
        classIdent: source.class_ident,
      });
      if (!resolved) {
        setArchetypeError(
          `Could not resolve an archetype for ${source.name} - try Refresh class list on step 1 first.`,
        );
        return;
      }
      setArchetype(resolved);
      setTrainer({
        enabled: true,
        guildName: trainer.guildName || `${className} Trainer`,
        trainerClassIdent: trainer.trainerClassIdent || `${className.replace(/[^A-Za-z0-9]/g, "")}Trainer`,
        npcName: trainer.npcName || `${className} Trainer`,
        referenceSearch: trainer.referenceSearch || `${source.name} Trainer`,
      });
    } catch (e) {
      setArchetypeError(`${e}`);
    }
  }

  async function generate() {
    if (!archetype) return;
    const req: TrainerRequest = {
      class_id: classId,
      class_name: className,
      trainer_class_ident: trainer.trainerClassIdent,
      guild_name: trainer.guildName,
      realm: source.realm,
      archetype_id: archetype.id,
      archetype_name: archetype.name,
      min_level: trainer.minLevel,
    };
    const code = await invoke<string>("generate_trainer_code", { req });
    setTrainer({ generatedCode: code, filename: trainer.filename || `${trainer.trainerClassIdent}.cs` });
  }

  async function deployTrainerFile() {
    setBusy(true);
    setLog("Deploying trainer file...");
    try {
      const result = await invoke<string>("deploy_class", {
        cfg: sshConfig,
        filename: trainer.filename,
        content: trainer.generatedCode,
      });
      setLog(result);
    } catch (e) {
      setLog(`ERROR: ${e}`);
    } finally {
      setBusy(false);
    }
  }

  async function searchNpcs() {
    setBusy(true);
    setLog("Searching...");
    try {
      const found = await invoke<TrainerNpcCandidate[]>("find_trainer_npcs", {
        cfg: sshConfig,
        search: trainer.referenceSearch,
      });
      setCandidates(found);
      setLog(found.length === 0 ? "No matching trainer NPCs found - try a different search." : "");
    } catch (e) {
      setLog(`ERROR: ${e}`);
      setCandidates(null);
    } finally {
      setBusy(false);
    }
  }

  async function spawn() {
    if (!trainer.selectedReferenceMobId) return;
    setBusy(true);
    setLog("Spawning...");
    try {
      const result = await invoke<string>("spawn_trainer_npc", {
        cfg: sshConfig,
        referenceMobId: trainer.selectedReferenceMobId,
        npcName: trainer.npcName,
        guildName: trainer.guildName,
        classType: `DOL.GS.Trainer.${trainer.trainerClassIdent}`,
        offsetX: trainer.offsetX,
        offsetY: trainer.offsetY,
      });
      setLog(result);
    } catch (e) {
      setLog(`ERROR: ${e}`);
    } finally {
      setBusy(false);
    }
  }

  return (
    <>
      <h2>7. Trainer (optional)</h2>
      <p className="hint">
        {className} isn't in the game client's built-in class list, so it can't be picked at
        character creation - classic DAoC's workaround (how every post-launch class, including
        Sluaghbinder, actually works) is a trainer that promotes an existing character of the
        right archetype into it at some level.
      </p>
      {!trainer.enabled ? (
        <button onClick={enable} disabled={busy}>
          Set up a trainer for {className}
        </button>
      ) : (
        <>
          {archetypeError && <p className="hint">{archetypeError}</p>}
          {archetype && (
            <p className="hint">
              Players create a <strong>{archetype.name}</strong> ({archetype.realm}) and this
              trainer offers to promote them into {className} once they reach the level below.
            </p>
          )}

          <label className="field">
            <span>Minimum level to promote at</span>
            <input
              type="number"
              value={trainer.minLevel}
              onChange={(e) => setTrainer({ minLevel: Number(e.target.value) })}
            />
          </label>
          <label className="field">
            <span>Trainer's NPCGuildScript name (must be unique on the server)</span>
            <input
              value={trainer.guildName}
              onChange={(e) => setTrainer({ guildName: e.target.value })}
            />
          </label>
          <label className="field">
            <span>Trainer's C# class name</span>
            <input
              value={trainer.trainerClassIdent}
              onChange={(e) => setTrainer({ trainerClassIdent: e.target.value })}
            />
          </label>

          <div className="actions">
            <button onClick={generate} disabled={busy || !archetype}>
              Generate trainer code
            </button>
          </div>

          {trainer.generatedCode && (
            <>
              <textarea className="code-preview" readOnly value={trainer.generatedCode} rows={16} />
              <label className="field">
                <span>Filename in the server's bin/scripts/ folder</span>
                <input
                  value={trainer.filename}
                  onChange={(e) => setTrainer({ filename: e.target.value })}
                />
              </label>
              <div className="actions">
                <button onClick={deployTrainerFile} disabled={busy}>
                  Deploy trainer file
                </button>
              </div>
            </>
          )}

          <h3>Spawn the trainer NPC</h3>
          <p className="hint">
            Finds an existing trainer NPC to copy - full stats/model/position, offset so it
            doesn't spawn on top of it. Search for the trainer of the class you cloned (e.g. "
            {source.name} Trainer") and pick whichever location you want this one next to (many
            classes have several, including one in the realm's capital city).
          </p>
          <label className="field">
            <span>Search existing trainer NPCs (matches the Guild field)</span>
            <input
              value={trainer.referenceSearch}
              onChange={(e) => setTrainer({ referenceSearch: e.target.value })}
            />
          </label>
          <div className="actions">
            <button onClick={searchNpcs} disabled={busy}>
              Search
            </button>
          </div>

          {candidates && candidates.length > 0 && (
            <table className="spec-table">
              <thead>
                <tr>
                  <th></th>
                  <th>Name</th>
                  <th>Guild</th>
                  <th>Region</th>
                  <th>X / Y</th>
                  <th>Level</th>
                </tr>
              </thead>
              <tbody>
                {candidates.map((c) => (
                  <tr key={c.mob_id}>
                    <td>
                      <input
                        type="radio"
                        name="trainer-reference"
                        checked={trainer.selectedReferenceMobId === c.mob_id}
                        onChange={() => setTrainer({ selectedReferenceMobId: c.mob_id })}
                      />
                    </td>
                    <td>{c.name}</td>
                    <td>{c.guild}</td>
                    <td>{c.region}</td>
                    <td>
                      {c.x} / {c.y}
                    </td>
                    <td>{c.level}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}

          {trainer.selectedReferenceMobId && (
            <>
              <label className="field">
                <span>New NPC's name</span>
                <input value={trainer.npcName} onChange={(e) => setTrainer({ npcName: e.target.value })} />
              </label>
              <label className="field">
                <span>X offset from the reference NPC</span>
                <input
                  type="number"
                  value={trainer.offsetX}
                  onChange={(e) => setTrainer({ offsetX: Number(e.target.value) })}
                />
              </label>
              <label className="field">
                <span>Y offset from the reference NPC</span>
                <input
                  type="number"
                  value={trainer.offsetY}
                  onChange={(e) => setTrainer({ offsetY: Number(e.target.value) })}
                />
              </label>
              <div className="actions">
                <button onClick={spawn} disabled={busy}>
                  Spawn trainer NPC
                </button>
              </div>
            </>
          )}

          {log && <pre className="status">{log}</pre>}
        </>
      )}
    </>
  );
}
