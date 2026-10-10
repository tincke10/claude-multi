import { Config, Project } from "../api";
import OrcaPanel from "../components/OrcaPanel";
import { appendImported } from "../lib/orca";

type Props = {
  config: Config;
  applyPersisted: (fn: (c: Config) => Config) => void;
};

export default function OrcaTab({ config, applyPersisted }: Props) {
  return (
    <OrcaPanel
      accounts={config.accounts}
      // The backend already persisted these, so they must not mark the form dirty.
      onImported={(added: Project[]) =>
        applyPersisted((c) => ({ ...c, projects: appendImported(c.projects, added) }))}
    />
  );
}
