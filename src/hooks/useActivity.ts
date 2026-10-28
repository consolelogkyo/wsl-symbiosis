import { createContext, useCallback, useContext, useEffect, useRef, useState } from "react";
import { ListeningPort, isUserPort, listListeningPorts, portUrl } from "../lib/packages";
import {
  TaskInfo,
  TaskKind,
  dismissProjectTask,
  isActive,
  isInside,
  listProjectTasks,
  onProjectTask,
  startProject,
  stopProcessTree,
  stopProject,
} from "../lib/projects";
import { errorMessage } from "../lib/wsl";

const PORTS_INTERVAL_MS = 5000;

export interface Toast {
  id: number;
  tone: "success" | "error" | "info";
  title: string;
  detail?: string;
  url?: string;
  taskId?: string;
}

/** What's running: tasks started from the app + every listening port of the user's work. */
export interface Activity {
  tasks: TaskInfo[];
  ports: ListeningPort[];
  /** Listening ports of the user's own processes (not system daemons). */
  userPorts: ListeningPort[];
  toasts: Toast[];
  logTaskId: string | null;
  start: (path: string, kind: TaskKind, script?: string) => Promise<TaskInfo | null>;
  stop: (task: TaskInfo) => Promise<void>;
  stopPort: (port: ListeningPort) => Promise<void>;
  dismiss: (taskId: string) => void;
  dismissToast: (id: number) => void;
  /** Shows a toast (used by features outside this hook, like the git queue). */
  notify: (toast: Omit<Toast, "id">) => void;
  openLog: (taskId: string | null) => void;
  /** The active (or most recent) task of a project folder. */
  taskFor: (path: string, kind?: TaskKind) => TaskInfo | undefined;
  /** URLs a task is serving: printed by the dev server, or ports owned by its folder. */
  urlsFor: (task: TaskInfo) => string[];
  refreshPorts: () => void;
}

export const ActivityContext = createContext<Activity | null>(null);

export function useActivityContext(): Activity {
  const ctx = useContext(ActivityContext);
  if (!ctx) throw new Error("ActivityContext ausente");
  return ctx;
}

let toastSeq = 1;

export function useActivity(distro: string | null, running: boolean): Activity {
  const [tasks, setTasks] = useState<TaskInfo[]>([]);
  const [ports, setPorts] = useState<ListeningPort[]>([]);
  const [toasts, setToasts] = useState<Toast[]>([]);
  const [logTaskId, setLogTaskId] = useState<string | null>(null);
  const announced = useRef(new Set<string>());
  const portsInFlight = useRef(false);
  const current = useRef(distro);
  current.current = distro;

  const pushToast = useCallback((t: Omit<Toast, "id">) => {
    const id = toastSeq++;
    setToasts((all) => [...all.slice(-3), { ...t, id }]);
    window.setTimeout(() => setToasts((all) => all.filter((x) => x.id !== id)), t.tone === "error" ? 12000 : 8000);
  }, []);

  // Tasks: initial list + live updates from the backend.
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let alive = true;
    listProjectTasks()
      .then((list) => alive && setTasks(list))
      .catch(() => {});
    onProjectTask((task) => {
      setTasks((all) => {
        const i = all.findIndex((t) => t.id === task.id);
        return i < 0 ? [...all, task] : all.map((t) => (t.id === task.id ? task : t));
      });
    })
      .then((fn) => (alive ? (unlisten = fn) : fn()))
      .catch(() => {});
    return () => {
      alive = false;
      unlisten?.();
    };
  }, []);

  const refreshPorts = useCallback(() => {
    const target = current.current;
    if (!target || portsInFlight.current) return;
    portsInFlight.current = true;
    listListeningPorts(target)
      .then((p) => current.current === target && setPorts(p))
      .catch(() => {})
      .finally(() => (portsInFlight.current = false));
  }, []);

  useEffect(() => {
    setPorts([]);
    if (!distro || !running) return;
    refreshPorts();
    const id = window.setInterval(refreshPorts, PORTS_INTERVAL_MS);
    return () => window.clearInterval(id);
  }, [distro, running, refreshPorts]);

  const urlsFor = useCallback(
    (task: TaskInfo) => {
      const fromPorts = ports
        .filter((p) => task.distro === distro && p.cwd && isInside(p.cwd, task.path))
        .map(portUrl);
      const seen = new Set<string>();
      return [...task.urls, ...fromPorts].filter((u) => {
        const key = u.replace(/\/$/, "");
        if ([...seen].some((s) => s === key || key.startsWith(`${s}/`))) return false;
        seen.add(key);
        return true;
      });
    },
    [ports, distro],
  );

  // Notifications on state changes (each one once).
  useEffect(() => {
    for (const task of tasks) {
      const once = (key: string, t: Omit<Toast, "id">) => {
        const k = `${task.id}:${key}`;
        if (announced.current.has(k)) return;
        announced.current.add(k);
        pushToast(t);
      };
      if (task.kind === "run" && task.status === "running") {
        const url = urlsFor(task)[0];
        if (url) once("up", { tone: "success", title: `${task.name} está rodando`, detail: url, url, taskId: task.id });
      }
      if (task.status === "failed") {
        once("failed", {
          tone: "error",
          title: task.kind === "install" ? `Falha ao instalar ${task.name}` : `${task.name} parou`,
          detail: `${task.command} · código ${task.exitCode ?? "?"}`,
          taskId: task.id,
        });
      }
      if (task.kind === "install" && task.status === "exited") {
        once("installed", { tone: "success", title: `Dependências instaladas`, detail: task.name, taskId: task.id });
      }
    }
  }, [tasks, urlsFor, pushToast]);

  // A task changing state usually means ports changed too.
  const statusKey = tasks.map((t) => `${t.id}:${t.status}`).join("|");
  useEffect(() => {
    if (statusKey) window.setTimeout(refreshPorts, 1500);
  }, [statusKey, refreshPorts]);

  const start = useCallback(
    async (path: string, kind: TaskKind, script?: string) => {
      if (!distro) return null;
      try {
        const task = await startProject(distro, path, kind, script);
        setTasks((all) => (all.some((t) => t.id === task.id) ? all : [...all, task]));
        if (kind === "install") {
          pushToast({ tone: "info", title: `Instalando dependências`, detail: `${task.name} · ${task.command}`, taskId: task.id });
        }
        return task;
      } catch (err) {
        pushToast({ tone: "error", title: kind === "install" ? "Não foi possível instalar" : "Não foi possível rodar", detail: errorMessage(err) });
        return null;
      }
    },
    [distro, pushToast],
  );

  const stop = useCallback(
    async (task: TaskInfo) => {
      try {
        await stopProject(task.id);
      } catch (err) {
        pushToast({ tone: "error", title: `Falha ao parar ${task.name}`, detail: errorMessage(err) });
      }
    },
    [pushToast],
  );

  const stopPort = useCallback(
    async (port: ListeningPort) => {
      if (!distro || port.pid === null) return;
      // A port served by one of our tasks: stop the task (its whole tree, cleanly).
      const owner = tasks.find((t) => isActive(t) && t.distro === distro && port.cwd && isInside(port.cwd, t.path));
      try {
        if (owner) await stopProject(owner.id);
        else await stopProcessTree(distro, port.pid);
        setPorts((all) => all.filter((p) => p.port !== port.port));
        window.setTimeout(refreshPorts, 1200);
      } catch (err) {
        pushToast({ tone: "error", title: `Falha ao parar a porta ${port.port}`, detail: errorMessage(err) });
      }
    },
    [distro, tasks, refreshPorts, pushToast],
  );

  const dismiss = useCallback((taskId: string) => {
    dismissProjectTask(taskId)
      .then(() => setTasks((all) => all.filter((t) => t.id !== taskId)))
      .catch(() => {});
  }, []);

  const taskFor = useCallback(
    (path: string, kind?: TaskKind) => {
      const mine = tasks.filter((t) => t.distro === distro && t.path === path && (!kind || t.kind === kind));
      return mine.find(isActive) ?? mine[mine.length - 1];
    },
    [tasks, distro],
  );

  return {
    tasks,
    ports,
    userPorts: ports.filter(isUserPort),
    toasts,
    logTaskId,
    start,
    stop,
    stopPort,
    dismiss,
    dismissToast: (id) => setToasts((all) => all.filter((t) => t.id !== id)),
    notify: pushToast,
    openLog: setLogTaskId,
    taskFor,
    urlsFor,
    refreshPorts,
  };
}
