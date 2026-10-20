import { useState } from "react";
import ClonePanel from "./ClonePanel";
import DeploySettingsPanel from "./DeploySettingsPanel";
import "./App.css";

type Tab = "clone" | "settings";

export default function App() {
  const [tab, setTab] = useState<Tab>("clone");

  return (
    <div className="app">
      <header>
        <h1>DAoC Class Editor</h1>
        <nav>
          <button className={tab === "clone" ? "active" : ""} onClick={() => setTab("clone")}>
            Clone a class
          </button>
          <button
            className={tab === "settings" ? "active" : ""}
            onClick={() => setTab("settings")}
          >
            Server connection
          </button>
        </nav>
      </header>
      <main>{tab === "clone" ? <ClonePanel /> : <DeploySettingsPanel />}</main>
    </div>
  );
}
