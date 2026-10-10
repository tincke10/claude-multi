import { useEffect, useState } from "react";

export type AsyncState<T> =
  | { state: "loading" }
  | { state: "error"; message: string }
  | { state: "ready"; data: T };

export function useAsync<T>(load: () => Promise<T>, deps: readonly unknown[]): AsyncState<T> {
  const [value, setValue] = useState<AsyncState<T>>({ state: "loading" });
  useEffect(() => {
    let cancelled = false;
    setValue({ state: "loading" });
    load().then(
      (data) => {
        if (!cancelled) setValue({ state: "ready", data });
      },
      (err: unknown) => {
        if (!cancelled) setValue({ state: "error", message: err instanceof Error ? err.message : String(err) });
      },
    );
    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, deps);
  return value;
}
