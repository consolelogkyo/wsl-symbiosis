import { useCallback, useEffect, useRef, useState } from "react";
import {
  Distro,
  WslInfo,
  errorMessage,
  getWslInfo,
  listDistros,
  preferredDistro,
  shutdownWsl,
  startDistro,
} from "../lib/wsl";
import { startDockerEngine, stopDockerEngine } from "../lib/docker";
import type { Progress } from "./useProgress";

const POLL_INTERVAL_MS = 5000;

export type PendingAction = "start" | "shutdown" | null;

/** The docker-desktop distro belongs to Docker Desktop: its button turns Docker on and off. */
export const isDockerDistro = (name: string | undefined) => !!name && /^docker-desktop/i.test(name);

/** `preferred`: default distro from the settings (undefined while they load).
 *  `active`: false while the window sits in the tray (polling pauses; the tray loop takes over).
 *  `track`: shows start/shutdown in the progress panel. */
export function useWsl(preferred?: string | null, active = true, track?: Progress["track"]) {
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
      // Keep the current selection if it still exists, otherwise fall back to a real distro
      // (Docker Desktop often makes docker-desktop WSL's default).
      setSelectedName((current) =>
        current && list.some((d) => d.name === current) ? current : preferredDistro(list)?.name ?? null,
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
    if (!active) return;
    refresh();
    const id = window.setInterval(refresh, POLL_INTERVAL_MS);
    return () => window.clearInterval(id);
  }, [refresh, active]);

  // Apply the configured default distro once, as soon as both settings and list are in.
  const preferredApplied = useRef(false);
  useEffect(() => {
    if (preferredApplied.current || preferred === undefined || distros.length === 0) return;
    preferredApplied.current = true;
    if (preferred && distros.some((d) => d.name === preferred)) setSelectedName(preferred);
  }, [preferred, distros]);

  const runAction = useCallback(
    async (action: Exclude<PendingAction, null>, title: string, detail: string, task: () => Promise<unknown>) => {
      setPending(action);
      try {
        if (track) await track(title, task, { detail });
        else await task();
        setError(null);
      } catch (err) {
        setError(errorMessage(err));
      } finally {
        setPending(null);
        await refresh();
      }
    },
    [refresh, track],
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
    start: () => {
      if (!selected) return;
      if (isDockerDistro(selected.name)) {
        return runAction("start", "Ligando o Docker", "Abrindo o Docker Desktop e esperando ele ficar pronto", () =>
          startDockerEngine(selected.name),
        );
      }
      return runAction("start", `Iniciando ${selected.name}`, "Ligando a distro", () => startDistro(selected.name));
    },
    shutdown: () => {
      if (isDockerDistro(selected?.name)) {
        return runAction("shutdown", "Desligando o Docker", "Fechando o Docker Desktop de forma segura", () => stopDockerEngine(selected!.name));
      }
      return runAction("shutdown", "Desligando o WSL", "Fechando terminais, o Docker Desktop (se ligado) e as distros", shutdownWsl);
    },
  };
}
