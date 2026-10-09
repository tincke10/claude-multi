import { useCallback, useState } from "react";
import { TabId, loadTab, saveTab } from "../lib/lastTab";

// Active tab, remembered across opens of the Preferences window.
export function useLastTab(): [TabId, (tab: TabId) => void] {
  const [tab, setTabState] = useState<TabId>(() => loadTab());
  const setTab = useCallback((next: TabId) => {
    setTabState(next);
    saveTab(next);
  }, []);
  return [tab, setTab];
}
