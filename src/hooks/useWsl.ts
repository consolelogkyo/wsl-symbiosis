import { useCallback, useEffect, useRef, useState } from "react";
import {
  Distro,
  WslInfo,
  errorMessage,
  getWslInfo,
  listDistros,
  shutdownWsl,
  startDistro,
} from "../lib/wsl";

const POLL_INTERVAL_MS = 5000;

export type PendingAction = "start" | "shutdown" | null;

/** `preferred`: default distro from the settings (undefined while they load). */
export function useWsl(preferred?: string | null) {
  const [distros, setDistros] = useState<Distro[]>([]);
  const [info, setInfo] = useState<WslInfo | null>(null);
  const [selectedName, setSelectedName] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [pending, setPending] = useState<PendingAction>(null);
  const [error, setError] = useState<string | null>(null);
  const inFlight = useRef(false);
  const hasInfo = useRef(false);

  const refresh = useCallback(async () => {
    if (inFlight.current) return;
    inFlight.current = true;
    // The WSL service can be slow to answer at boot; keep asking until it does.
    if (!hasInfo.current) {
      getWslInfo()
        .then((i) => {
          hasInfo.current = i.wslVersion !== null;
          setInfo(i);
        })
        .catch(() => {});
    }
    try {
      const list = await listDistros();
      setDistros(list);
      // Keep the current selection if it still exists, otherwise fall back to the default distro.
      setSelectedName((current) =>
        current && list.some((d) => d.name === current)
          ? current
          : (list.find((d) => d.isDefault) ?? list[0])?.name ?? null,
      );
      setError(null);
    } catch (err) {
      setError(errorMessage(err));
    } finally {
      inFlight.current = false;
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    refresh();
    const id = window.setInterval(refresh, POLL_INTERVAL_MS);
    return () => window.clearInterval(id);
  }, [refresh]);

  // Apply the configured default distro once, as soon as both settings and list are in.
  const preferredApplied = useRef(false);
  useEffect(() => {
    if (preferredApplied.current || preferred === undefined || distros.length === 0) return;
    preferredApplied.current = true;
    if (preferred && distros.some((d) => d.name === preferred)) setSelectedName(preferred);
  }, [preferred, distros]);

  const runAction = useCallback(
    async (action: Exclude<PendingAction, null>, task: () => Promise<void>) => {
      setPending(action);
      try {
        await task();
        setError(null);
      } catch (err) {
        setError(errorMessage(err));
      } finally {
        setPending(null);
        await refresh();
      }
    },
    [refresh],
  );

  const selected = distros.find((d) => d.name === selectedName) ?? null;

  return {
    distros,
    info,
    selected,
    select: setSelectedName,
    loading,
    pending,
    error,
    dismissError: () => setError(null),
    refresh,
    start: () => selected && runAction("start", () => startDistro(selected.name)),
    shutdown: () => runAction("shutdown", shutdownWsl),
  };
}
