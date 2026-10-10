type Props = {
  dirty: boolean;
  // Label of the first tab that blocks Save, if any.
  errorTabLabel: string | null;
  status: { kind: "ok" | "err"; text: string } | null;
  onSave: () => void;
};

// Global Save: enabled only when there are unsaved changes and no tab has errors.
export default function SaveBar({ dirty, errorTabLabel, status, onSave }: Props) {
  return (
    <footer className="app__footer">
      <button
        className="btn btn--primary"
        onClick={onSave}
        disabled={!dirty || errorTabLabel !== null}
      >Save</button>
      {dirty && errorTabLabel === null && <span className="muted">Unsaved changes</span>}
      {errorTabLabel !== null && (
        <span className="status status--err">Fix the {errorTabLabel} tab to save.</span>
      )}
      {status && <span className={`status status--${status.kind}`}>{status.text}</span>}
    </footer>
  );
}
