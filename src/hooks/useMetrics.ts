import { useCallback, useEffect, useRef, useState } from "react";
import { WslResourceMetrics, errorMessage, getResourceMetrics } from "../lib/wsl";

/**
 * Polls host + guest telemetry. Lives in App (the sidebar's live panel is always visible);
 * the interval is shorter while the Dashboard is open.
 */
export function useMetrics(distro: string | null, intervalMs: number) {
  const [metrics, setMetrics] = useState<WslResourceMetrics | null>(null);
  const [error, setError] = useState<string | null>(null);
  const inFlight = useRef(false);
  const current = useRef(distro);
  current.current = distro;

  const refresh = useCallback(async () => {
    const target = current.current;
    if (!target || inFlight.current) return;
    inFlight.current = true;
    try {
      const result = await getResourceMetrics(target);
      // Drop responses for a distro that is no longer selected.
      if (current.current === target) {
        setMetrics(result);
        setError(null);
      }
    } catch (err) {
      if (current.current === target) setError(errorMessage(err));
    } finally {
      inFlight.current = false;
    }
  }, []);

  useEffect(() => {
    setMetrics(null);
    setError(null);
  }, [distro]);

  useEffect(() => {
    if (!distro) return;
    refresh();
    const id = window.setInterval(refresh, intervalMs);
    return () => window.clearInterval(id);
  }, [distro, intervalMs, refresh]);

  // A response from the previous distro may still be on screen right after switching.
  const fresh = metrics?.distro === distro ? metrics : null;
  return { metrics: fresh, error, refresh };
}
