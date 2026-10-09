import type { SessionStatus } from "@cm/core";
import { statusLabel } from "../format";

export function StatusBadge({ status }: { status: SessionStatus }) {
  return <span className={`badge ${status}`}>{statusLabel(status)}</span>;
}
