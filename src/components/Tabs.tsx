import { TabId } from "../lib/lastTab";

export type TabDef = { id: TabId; label: string; error: boolean };

type Props = {
  tabs: TabDef[];
  active: TabId;
  onSelect: (id: TabId) => void;
};

// Tab strip with an error dot on tabs whose section blocks Save.
export default function Tabs({ tabs, active, onSelect }: Props) {
  return (
    <div className="tabs" role="tablist">
      {tabs.map((t) => (
        <button
          key={t.id}
          role="tab"
          id={`tab-${t.id}`}
          aria-selected={t.id === active}
          aria-controls="tabpanel"
          className={`tabs__tab ${t.id === active ? "tabs__tab--active" : ""}`}
          onClick={() => onSelect(t.id)}
        >
          {t.label}
          {t.error && <span className="tabs__dot" role="img" aria-label="has errors" />}
        </button>
      ))}
    </div>
  );
}
