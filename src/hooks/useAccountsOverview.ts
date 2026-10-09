import { useEffect, useRef, useState } from "react";
import {
  AccountOverview, accountsOverview, loginAccount, logoutAccount, reloginAccount,
} from "../api";
import { AuthAction, POLL_DELAYS_MS } from "../lib/accounts";
import { IN_TAURI } from "./useConfig";

const AUTH_CALLS: Record<AuthAction, (id: string) => Promise<void>> = {
  login: loginAccount,
  logout: logoutAccount,
  relogin: reloginAccount,
};

// Same key the poller compares: a login/logout/relogin changes status or email.
const overviewKey = (o: AccountOverview | undefined) => (o ? `${o.status}|${o.email ?? ""}` : "");

// Login status + usage per saved account, refreshed on mount, Save, focus and
// (polled) after an auth action, since logins finish in an external terminal.
// Lives in the shell so polling survives tab switches.
export function useAccountsOverview() {
  const [overview, setOverview] = useState<AccountOverview[]>([]);
  const [loaded, setLoaded] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [refreshing, setRefreshing] = useState(false);
  const [actionErrs, setActionErrs] = useState<Record<string, string>>({});
  const [busy, setBusy] = useState<Set<string>>(new Set());
  const mounted = useRef(true);
  const overviewRef = useRef<AccountOverview[]>([]);
  const pollTimers = useRef<Map<string, number>>(new Map());

  const refresh = async (): Promise<AccountOverview[] | null> => {
    setRefreshing(true);
    try {
      const rows = await accountsOverview();
      if (!mounted.current) return null;
      overviewRef.current = rows;
      setOverview(rows);
      setLoaded(true);
      setError(null);
      return rows;
    } catch (e) {
      if (mounted.current) setError(`${e}`);
      return null;
    } finally {
      if (mounted.current) setRefreshing(false);
    }
  };

  const stopPolling = (id: string) => {
    const t = pollTimers.current.get(id);
    if (t !== undefined) window.clearTimeout(t);
    pollTimers.current.delete(id);
  };

  // Re-read the overview with growing delays (~2 min) until this account's
  // status/email differs from `prevKey`, then stop.
  const startPolling = (id: string, prevKey: string) => {
    stopPolling(id);
    let step = 0;
    const schedule = () => {
      if (step >= POLL_DELAYS_MS.length) { pollTimers.current.delete(id); return; }
      pollTimers.current.set(id, window.setTimeout(tick, POLL_DELAYS_MS[step++]));
    };
    const tick = async () => {
      const rows = await refresh();
      if (!mounted.current) return;
      if (overviewKey(rows?.find((r) => r.id === id)) !== prevKey && rows !== null) {
        pollTimers.current.delete(id);
        return;
      }
      schedule();
    };
    schedule();
  };

  const runAuthAction = async (id: string, action: AuthAction) => {
    const prevKey = overviewKey(overviewRef.current.find((r) => r.id === id));
    setBusy((b) => new Set(b).add(id));
    setActionErrs((m) => { const { [id]: _drop, ...rest } = m; return rest; });
    try {
      await AUTH_CALLS[action](id);
      startPolling(id, prevKey);
    } catch (e) {
      setActionErrs((m) => ({ ...m, [id]: `${e}` }));
    } finally {
      setBusy((b) => { const n = new Set(b); n.delete(id); return n; });
    }
  };

  useEffect(() => {
    mounted.current = true;
    const timers = pollTimers.current;
    if (IN_TAURI) void refresh();
    return () => {
      mounted.current = false;
      timers.forEach((t) => window.clearTimeout(t));
      timers.clear();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  return { overview, loaded, error, refreshing, actionErrs, busy, refresh, runAuthAction };
}
