import { Link, useParams } from "react-router";
import { fetchSession } from "../api";
import { formatCost, formatDateTime, formatDuration, formatLocation, formatTokens } from "../format";
import { summaryNote } from "../map";
import { useAsync } from "../useAsync";
import { StatusBadge } from "./Status";

export function SessionDetailPage() {
  const { id = "" } = useParams();
  const result = useAsync(() => fetchSession(id), [id]);
  if (result.state === "loading") return <p className="muted">Loading...</p>;
  if (result.state === "error") return <p className="error">{result.message}</p>;
  const s = result.data;
  const note = summaryNote(s.summaryStatus, s.summary, s.summaryError);
  return (
    <section>
      <p><Link to="/">&larr; Sessions</Link></p>
      <h1>{s.project ?? s.sessionId} <StatusBadge status={s.status} /></h1>
      <p className={note.kind}>{note.text}</p>
      <dl>
        <dt>Session</dt><dd>{s.sessionId}</dd>
        <dt>Account</dt><dd>{s.accountLabel ?? s.accountId ?? "-"} {s.accountEmail ?? ""}</dd>
        <dt>Host</dt><dd>{s.host ?? "-"} ({s.os ?? "?"}), local {s.localIp ?? "-"}, public {s.publicIp ?? "-"}</dd>
        <dt>Location</dt><dd>{formatLocation(s.city, s.country)} {s.asOrg ?? ""}</dd>
        <dt>Directory</dt><dd>{s.cwd ?? "-"} {s.gitBranch ? `(${s.gitBranch})` : ""}</dd>
        <dt>Started</dt><dd>{formatDateTime(s.startedAt)} ({s.startSource ?? "-"})</dd>
        <dt>Ended</dt><dd>{formatDateTime(s.endedAt)} ({s.endReason ?? "-"})</dd>
        <dt>Last event</dt><dd>{formatDateTime(s.lastEventAt)}</dd>
        <dt>Requests / errors</dt><dd>{s.apiRequests} / {s.apiErrors}</dd>
        <dt>Prompts</dt><dd>{s.prompts}</dd>
        <dt>Tokens</dt><dd>in {formatTokens(s.inputTokens)}, out {formatTokens(s.outputTokens)}, cache read {formatTokens(s.cacheReadTokens)}, cache write {formatTokens(s.cacheCreationTokens)}</dd>
        <dt>Cost</dt><dd>{formatCost(s.costUsdMicros)}</dd>
      </dl>
      <h2>Tools</h2>
      <table>
        <thead><tr><th>Tool</th><th>Calls</th><th>Failures</th><th>Time</th></tr></thead>
        <tbody>
          {s.tools.map((t) => (
            <tr key={t.toolName}><td>{t.toolName}</td><td className="num">{t.calls}</td><td className="num">{t.failures}</td><td className="num">{formatDuration(t.durationMs)}</td></tr>
          ))}
        </tbody>
      </table>
      <h2>Models</h2>
      <table>
        <thead><tr><th>Model</th><th>Requests</th><th>Cost</th></tr></thead>
        <tbody>
          {s.models.map((m) => (
            <tr key={m.model}><td>{m.model}</td><td className="num">{m.requests}</td><td className="num">{formatCost(m.costUsdMicros)}</td></tr>
          ))}
        </tbody>
      </table>
    </section>
  );
}
