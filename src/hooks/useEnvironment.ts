import { useCallback, useEffect, useRef, useState } from "react";
import { EnvironmentScan, ListeningPort, scanEnvironment } from "../lib/packages";
import { errorMessage } from "../lib/wsl";

/** Last scan per distro: reopening the tab shows it instantly while a fresh scan runs. */
const cache = new Map<string, EnvironmentScan>();

export function useEnvironment(distro: string | null, running: boolean) {
  const [scan, setScan] = useState<EnvironmentScan | null>(distro ? cache.get(distro) ?? null : null);
  const [scanning, setScanning] = useState(false);
  // A scan the user asked for (Atualizar, Node switch, npm install): the screen shows it. The
  // automatic one on opening runs quietly over the cached result, as before.
  const [requested, setRequested] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const current = useRef(distro);
  current.current = distro;

  const refresh = useCallback(async (requestedByUser = true) => {
    const target = current.current;
    if (!target) return;
    setScanning(true);
    setRequested(requestedByUser);
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
    if (distro && running) refresh(false);
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

  return {
    scan: scan?.distro === distro ? scan : null,
    scanning,
    /** Scanning because the user asked: dim the old data and show the bar. */
    updating: scanning && requested,
    error,
    setError,
    refresh,
    setPorts,
  };
}
