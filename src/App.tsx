import { useEffect, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { orcaHealth } from "./api";
import Tabs, { TabDef } from "./components/Tabs";
import SaveBar from "./components/SaveBar";
import { IN_TAURI, useConfig } from "./hooks/useConfig";
import { useAccountsOverview } from "./hooks/useAccountsOverview";
import { useLastTab } from "./hooks/useLastTab";
import { TabId } from "./lib/lastTab";
import { orcaPanelVisible } from "./lib/orca";
import { firstErrorTab, tabErrors } from "./lib/tabErrors";
import AccountsTab from "./tabs/AccountsTab";
import ProjectsTab from "./tabs/ProjectsTab";
import TrackingTab from "./tabs/TrackingTab";
import OrcaTab from "./tabs/OrcaTab";
import SettingsTab from "./tabs/SettingsTab";

const TAB_LABELS: Record<TabId, string> = {
  accounts: "Accounts",
  projects: "Projects",
  tracking: "Tracking",
  orca: "Orca",
  settings: "Settings",
};

export default function App() {
  const ov = useAccountsOverview();
  const cfgState = useConfig(() => { void ov.refresh(); });
  const [storedTab, setTab] = useLastTab();
  // The Orca tab only exists when the backend supports it and Orca is installed.
  const [orcaVisible, setOrcaVisible] = useState(false);

  useEffect(() => {
    if (!IN_TAURI) return;
    let alive = true;
    orcaHealth()
      .then((h) => { if (alive) setOrcaVisible(orcaPanelVisible(h)); })
      .catch(() => { /* the panel is optional; leave the tab hidden */ });
    return () => { alive = false; };
  }, []);

  // The window is hidden (not destroyed) on close, so this component persists.
  // Clear any stale status message and re-read the overview on focus.
  const { clearStatus } = cfgState;
  const { refresh } = ov;
  useEffect(() => {
    if (!IN_TAURI) return;
    let alive = true;
    let unlisten: (() => void) | undefined;
    getCurrentWindow()
      .onFocusChanged(({ payload: focused }) => { if (focused) { clearStatus(); void refresh(); } })
      .then((u) => {
        // Unmounted before the subscription resolved: release it right away.
        if (alive) unlisten = u; else u();
      })
      .catch(() => { /* no window events outside Tauri; nothing to clean up */ });
    return () => { alive = false; unlisten?.(); };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const { config, loadError } = cfgState;

  if (loadError) {
    return (
      <div className="app">
        <header className="app__header">
          <h1>claude-multi</h1>
          <p className="app__subtitle">Preferences</p>
        </header>
        <p className="banner banner--err">{loadError}</p>
      </div>
    );
  }
  if (!config) return <div className="app"><p className="muted">Loading…</p></div>;

  const errors = tabErrors(config);
  const errorTab = firstErrorTab(errors);
  const tabs: TabDef[] = (Object.keys(TAB_LABELS) as TabId[])
    .filter((id) => id !== "orca" || orcaVisible)
    .map((id) => ({ id, label: TAB_LABELS[id], error: errors[id] !== null }));
  // A remembered Orca tab falls back to Accounts when Orca isn't available.
  const active: TabId = tabs.some((t) => t.id === storedTab) ? storedTab : "accounts";

  return (
    <div className="app">
      <header className="app__header">
        <h1>claude-multi</h1>
        <p className="app__subtitle">Preferences</p>
      </header>

      <Tabs tabs={tabs} active={active} onSelect={setTab} />

      <div id="tabpanel" role="tabpanel" aria-labelledby={`tab-${active}`}>
        {active === "accounts" && (
          <AccountsTab
            config={config}
            snapshot={cfgState.snapshot}
            patch={cfgState.patch}
            accountsOverview={ov}
          />
        )}
        {active === "projects" && (
          <ProjectsTab config={config} patch={cfgState.patch} applyPersisted={cfgState.applyPersisted} />
        )}
        {active === "tracking" && (
          <TrackingTab config={config} patch={cfgState.patch} saveWarnings={cfgState.saveWarnings} />
        )}
        {active === "orca" && <OrcaTab config={config} applyPersisted={cfgState.applyPersisted} />}
        {active === "settings" && (
          <SettingsTab config={config} terminals={cfgState.terminals} patch={cfgState.patch} />
        )}
      </div>

      <SaveBar
        dirty={cfgState.dirty}
        errorTabLabel={errorTab === null ? null : TAB_LABELS[errorTab]}
        status={cfgState.status}
        onSave={() => void cfgState.save()}
      />
    </div>
  );
}
