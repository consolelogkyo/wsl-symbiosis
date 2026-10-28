import { useCallback, useEffect, useRef, useState } from "react";
import { EnvironmentScan, ListeningPort, scanEnvironment } from "../lib/packages";
import { errorMessage } from "../lib/wsl";

/** Last scan per distro: reopening the tab shows it instantly while a fresh scan runs. */
const cache = new Map<string, EnvironmentScan>();

export function useEnvironment(distro: string | null, running: boolean) {
  const [scan, setScan] = useState<EnvironmentScan | null>(distro ? cache.get(distro) ?? null : null);
  const [scanning, setScanning] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const current = useRef(distro);
  current.current = distro;

  const refresh = useCallback(async () => {
    const target = current.current;
    if (!target) return;
    setScanning(true);
    try {
      const result = await scanEnvironment(target);
      cache.set(target, result);
      if (current.current === target) {
        setScan(result);
        setError(null);
      }
    } catch (err) {
      if (current.current === target) setError(errorMessage(err));
    } finally {
      if (current.current === target) setScanning(false);
    }
  }, []);

  useEffect(() => {
    setScan(distro ? cache.get(distro) ?? null : null);
    setError(null);
    if (distro && running) refresh();
  }, [distro, running, refresh]);

  /** Ports refresh on their own (cheaper than a full scan). */
  const setPorts = useCallback((ports: ListeningPort[]) => {
    setScan((s) => {
      if (!s) return s;
      const next = { ...s, listeningPorts: ports, portsError: null };
      cache.set(s.distro, next);
      return next;
    });
  }, []);

  return { scan: scan?.distro === distro ? scan : null, scanning, error, setError, refresh, setPorts };
}
