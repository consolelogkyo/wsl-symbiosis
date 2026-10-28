import { useCallback, useEffect, useRef, useState } from "react";
import { DirectoryListing, getDistroHome, listDirectory } from "../lib/explorer";
import { errorMessage } from "../lib/wsl";

/** One Explorer tab: starts at `initialPath` (or home) and, after a distro restart, resumes
 *  at the last folder it showed. */
export function useExplorer(distro: string | null, running: boolean, initialPath: string | null = null) {
  const [home, setHome] = useState<string | null>(null);
  const [listing, setListing] = useState<DirectoryListing | null>(null);
  const [history, setHistory] = useState<string[]>([]);
  const [showHidden, setShowHidden] = useState(false);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const request = useRef(0);
  const lastPath = useRef(initialPath);
  const hiddenRef = useRef(showHidden);
  hiddenRef.current = showHidden;

  const load = useCallback(
    async (path: string, opts: { push?: string | null } = {}) => {
      if (!distro) return;
      const id = ++request.current;
      setLoading(true);
      try {
        const result = await listDirectory(distro, path, hiddenRef.current);
        if (id !== request.current) return;
        setListing(result);
        lastPath.current = result.path;
        if (opts.push) setHistory((h) => [...h.slice(-49), opts.push!]);
        setError(null);
      } catch (err) {
        if (id === request.current) setError(errorMessage(err));
      } finally {
        if (id === request.current) setLoading(false);
      }
    },
    [distro],
  );

  // (Re)start when the distro or its running state changes.
  useEffect(() => {
    setListing(null);
    setHistory([]);
    setError(null);
    setHome(null);
    if (!distro || !running) return;
    let cancelled = false;
    getDistroHome(distro)
      .then((h) => {
        if (cancelled) return;
        setHome(h);
        load(lastPath.current ?? h);
      })
      .catch((err) => !cancelled && setError(errorMessage(err)));
    return () => {
      cancelled = true;
    };
  }, [distro, running, load]);

  const current = listing?.path ?? null;

  const navigate = useCallback((path: string) => load(path, { push: current }), [load, current]);
  const refresh = useCallback(() => current && load(current), [load, current]);
  const back = useCallback(() => {
    const prev = history[history.length - 1];
    if (!prev) return;
    setHistory((h) => h.slice(0, -1));
    load(prev);
  }, [history, load]);

  const toggleHidden = useCallback(() => {
    hiddenRef.current = !hiddenRef.current;
    setShowHidden(hiddenRef.current);
    if (current) load(current);
  }, [current, load]);

  return {
    home,
    listing,
    loading,
    error,
    setError,
    showHidden,
    toggleHidden,
    canGoBack: history.length > 0,
    navigate,
    refresh,
    back,
  };
}
