import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { save, open } from "@tauri-apps/plugin-dialog";
import ClonePanel from "./ClonePanel";
import DeploySettingsPanel from "./DeploySettingsPanel";
import DeployedClassesPanel from "./DeployedClassesPanel";
import { ProjectProvider, emptyProjectState, useProject } from "./ProjectContext";
import "./App.css";

type Tab = "clone" | "deployed" | "settings";

const FILTERS = [{ name: "DAoC Class Project", extensions: ["daocclass.json", "json"] }];

function Header({ tab, setTab }: { tab: Tab; setTab: (t: Tab) => void }) {
  const [project, setProject] = useProject();
  const [status, setStatus] = useState("");

  async function saveProject() {
    const path = await save({ filters: FILTERS, defaultPath: `${project.newName || "class"}.daocclass.json` });
    if (!path) return;
    try {
      await invoke("save_project_file", { path, json: JSON.stringify(project, null, 2) });
      setStatus(`Saved ${path}`);
    } catch (e) {
      setStatus(`Save failed: ${e}`);
    }
  }

  async function loadProject() {
    const path = await open({ filters: FILTERS, multiple: false });
    if (!path || Array.isArray(path)) return;
    try {
      const json = await invoke<string>("load_project_file", { path });
      const loaded = JSON.parse(json);
      // Merge over the empty default so an older save missing newer fields (e.g. specAssignments
      // added later) doesn't crash the app - it just starts those fields blank.
      setProject({ ...emptyProjectState, ...loaded });
      setStatus(`Loaded ${path}`);
    } catch (e) {
      setStatus(`Load failed: ${e}`);
    }
  }

  function newProject() {
    if (!confirm("Start a new, blank class? Unsaved changes will be lost.")) return;
    setProject(emptyProjectState);
    setStatus("Started a new blank project.");
  }

  return (
    <header>
      <h1>DAoC Class Editor</h1>
      <nav>
        <button className={tab === "clone" ? "active" : ""} onClick={() => setTab("clone")}>
          Clone a class
        </button>
        <button className={tab === "deployed" ? "active" : ""} onClick={() => setTab("deployed")}>
          Deployed classes
        </button>
        <button className={tab === "settings" ? "active" : ""} onClick={() => setTab("settings")}>
          Server connection
        </button>
      </nav>
      <div className="project-actions">
        <button onClick={newProject}>New</button>
        <button onClick={saveProject}>Save</button>
        <button onClick={loadProject}>Load</button>
        {status && <span className="project-status">{status}</span>}
      </div>
    </header>
  );
}

export default function App() {
  const [tab, setTab] = useState<Tab>("clone");

  return (
    <ProjectProvider>
      <div className="app">
        <Header tab={tab} setTab={setTab} />
        <main>
          {tab === "clone" && <ClonePanel />}
          {tab === "deployed" && <DeployedClassesPanel />}
          {tab === "settings" && <DeploySettingsPanel />}
        </main>
      </div>
    </ProjectProvider>
  );
}
