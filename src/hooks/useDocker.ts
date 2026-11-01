import { useCallback, useEffect, useRef, useState } from "react";
import { type ContainerInfo, type DockerStatus, getDockerStatus, listContainers, startDockerEngine } from "../lib/docker";
import { errorMessage } from "../lib/wsl";
import { useProgressContext } from "./useProgress";

const LIST_INTERVAL = 5000;
/** While Docker is off: notice when it's turned on outside the app (Docker Desktop, tray). */
const STATUS_INTERVAL = 10000;

/** Docker engine + containers of the Bancos & Containers screen. Polls only while `active`. */
export function useDocker(distro: string | null, active: boolean) {
  const [status, setStatus] = useState<DockerStatus | null>(null);
  const [containers, setContainers] = useState<ContainerInfo[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [starting, setStarting] = useState(false);
  const inFlight = useRef(false);
  const { track } = useProgressContext();
  const target = distro ?? "";

  const checkStatus = useCallback(async () => {
    try {
      const s = await getDockerStatus(target);
      setStatus(s);
      if (s.state !== "ready") setContainers(null);
      return s;
    } catch (err) {
      setError(errorMessage(err));
      return null;
    }
  }, [target]);

  const refresh = useCallback(async () => {
    if (inFlight.current) return;
    inFlight.current = true;
    try {
      setContainers(await listContainers(target));
      setError(null);
    } catch (err) {
      // Docker went away (closed from the tray, PC sleeping): show the "Ligar" state again.
      const s = await checkStatus();
      if (s?.state === "ready") setError(errorMessage(err));
    } finally {
      inFlight.current = false;
    }
  }, [target, checkStatus]);

  useEffect(() => {
    if (!active) return;
    void checkStatus();
  }, [active, checkStatus]);

  const ready = status?.state === "ready";
  useEffect(() => {
    if (!active) return;
    if (ready) {
      void refresh();
      const id = window.setInterval(() => void refresh(), LIST_INTERVAL);
      return () => window.clearInterval(id);
    }
    if (starting) return;
    const id = window.setInterval(() => void checkStatus(), STATUS_INTERVAL);
    return () => window.clearInterval(id);
  }, [active, ready, starting, refresh, checkStatus]);

  const startEngine = useCallback(async () => {
    setStarting(true);
    setError(null);
    try {
      setStatus(
        await track("Ligando o Docker", () => startDockerEngine(target), {
          detail: "Abrindo o Docker e esperando ele ficar pronto (até 1 minuto)",
        }),
      );
    } catch (err) {
      setError(errorMessage(err));
      await checkStatus();
    } finally {
      setStarting(false);
    }
  }, [target, checkStatus, track]);

  return { status, containers, error, setError, starting, startEngine, refresh, checkStatus };
}
