import { useState } from "react";
import { fetchTotals } from "../api";
import { formatCost, formatTokens } from "../format";
import { totalsGrand } from "../map";
import { filtersToQueryString, EMPTY_FILTERS } from "../query";
import { useAsync } from "../useAsync";

export function TotalsPage() {
  const [from, setFrom] = useState("");
  const [to, setTo] = useState("");
  const qs = filtersToQueryString({ ...EMPTY_FILTERS, from, to });
  const result = useAsync(() => fetchTotals(qs), [qs]);
  return (
    <section>
      <h1>Totals per account</h1>
      <div className="filters">
        <input aria-label="From" type="date" value={from} onChange={(e) => setFrom(e.target.value)} />
        <input aria-label="To" type="date" value={to} onChange={(e) => setTo(e.target.value)} />
      </div>
      {result.state === "loading" && <p className="muted">Loading...</p>}
      {result.state === "error" && <p className="error">{result.message}</p>}
      {result.state === "ready" && (
        <table>
          <thead><tr><th>Account</th><th>Sessions</th><th>Prompts</th><th>Requests</th><th>Tool calls</th><th>Tokens in/out</th><th>Cost</th></tr></thead>
          <tbody>
            {result.data.accounts.map((a) => (
              <tr key={a.accountId ?? "none"}>
                <td>{a.accountId ?? "-"}</td><td className="num">{a.sessions}</td><td className="num">{a.prompts}</td>
                <td className="num">{a.apiRequests}</td><td className="num">{a.toolCalls}</td>
                <td className="num">{formatTokens(a.inputTokens)} / {formatTokens(a.outputTokens)}</td>
                <td className="num">{formatCost(a.costUsdMicros)}</td>
              </tr>
            ))}
          </tbody>
          <tfoot>
            {(() => {
              const g = totalsGrand(result.data.accounts);
              return (
                <tr><th>Total</th><td className="num">{g.sessions}</td><td className="num">{g.prompts}</td><td className="num">{g.apiRequests}</td><td className="num">{g.toolCalls}</td>
                  <td className="num">{formatTokens(g.inputTokens)} / {formatTokens(g.outputTokens)}</td><td className="num">{formatCost(g.costUsdMicros)}</td></tr>
              );
            })()}
          </tfoot>
        </table>
      )}
    </section>
  );
}
