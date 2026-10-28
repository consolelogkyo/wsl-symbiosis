import { openUrl } from "@tauri-apps/plugin-opener";
import { Loader2, ScrollText, Square } from "lucide-react";
import type { ListeningPort } from "../lib/packages";
import { portUrl } from "../lib/packages";
import { TaskInfo, isActive, isInside } from "../lib/projects";
import { useActivityContext } from "../hooks/useActivity";

const STATUS: Record<TaskInfo["status"], { label: string; dot: string }> = {
  starting: { label: "iniciando", dot: "bg-amber-400 animate-pulse" },
  running: { label: "rodando", dot: "bg-emerald-400" },
  stopping: { label: "parando", dot: "bg-amber-400 animate-pulse" },
  stopped: { label: "parado", dot: "bg-zinc-600" },
  exited: { label: "concluído", dot: "bg-emerald-400" },
  failed: { label: "falhou", dot: "bg-rose-500" },
};

export function TaskBadge({ task }: { task: TaskInfo }) {
  const s = STATUS[task.status];
  const label = task.kind === "install" && task.status === "running" ? "instalando" : s.label;
  return (
    <span className="flex shrink-0 items-center gap-1.5 text-[10px] tracking-wider text-zinc-400 uppercase">
      <span className={`size-1.5 rounded-full ${s.dot}`} />
      {label}
    </span>
  );
}

const basename = (path: string) => path.split("/").filter(Boolean).pop() ?? path;

/**
 * Sidebar "EM EXECUÇÃO": tasks started from the app plus every other dev server listening in
 * the selected distro (grouped by the folder it runs from), each with its links and a stop.
 */
export function RunningPanel({ distro }: { distro: string | null }) {
  const { tasks, userPorts, urlsFor, stop, stopPort, openLog } = useActivityContext();
  const active = tasks.filter((t) => t.distro === distro && isActive(t));
  // Ports not explained by one of our tasks, grouped by working directory.
  const orphans = new Map<string, ListeningPort[]>();
  for (const port of userPorts) {
    if (active.some((t) => port.cwd && isInside(port.cwd, t.path))) continue;
    const key = port.cwd ?? `pid ${port.pid}`;
    orphans.set(key, [...(orphans.get(key) ?? []), port]);
  }
  const count = active.length + orphans.size;

  return (
    <section className="mx-2 mb-2 flex max-h-[38vh] min-h-0 flex-col rounded-lg border border-zinc-800 bg-zinc-950">
      <div className="flex items-center gap-1.5 px-3 pt-2.5 pb-1.5">
        <span className={`size-1.5 rounded-full ${count > 0 ? "bg-emerald-400" : "bg-zinc-600"}`} />
        <span className="text-[10px] font-semibold tracking-[0.2em] text-zinc-400 uppercase">Em execução</span>
        <span className="ml-auto font-mono text-[10px] text-zinc-600">{count}</span>
      </div>
      <div className="min-h-0 space-y-px overflow-y-auto px-1.5 pb-1.5">
        {count === 0 && <p className="px-1.5 pb-1 text-[11px] text-zinc-600">Nenhum servidor rodando.</p>}

        {active.map((task) => (
          <Entry
            key={task.id}
            name={task.name}
            title={`${task.path}\n${task.command}`}
            urls={urlsFor(task)}
            badge={task.kind === "install" ? "instalando" : task.status === "running" ? undefined : STATUS[task.status].label}
            busy={task.status === "stopping"}
            onLogs={() => openLog(task.id)}
            onStop={() => stop(task)}
          />
        ))}

        {[...orphans.entries()].map(([cwd, ports]) => (
          <Entry
            key={cwd}
            name={basename(cwd)}
            title={`${cwd}\n${ports[0].command ?? ports[0].process ?? ""}`}
            urls={ports.map(portUrl)}
            onStop={() => Promise.all(ports.map(stopPort)).then(() => undefined)}
          />
        ))}
      </div>
    </section>
  );
}

function Entry({
  name,
  title,
  urls,
  badge,
  busy,
  onLogs,
  onStop,
}: {
  name: string;
  title: string;
  urls: string[];
  badge?: string;
  busy?: boolean;
  onLogs?: () => void;
  onStop: () => void;
}) {
  return (
    <div className="group rounded-md px-1.5 py-1 hover:bg-zinc-900" title={title}>
      <div className="flex items-center gap-1.5">
        <span className={`size-1.5 shrink-0 rounded-full ${badge ? "animate-pulse bg-amber-400" : "bg-emerald-400"}`} />
        <span className="min-w-0 flex-1 truncate text-[12px] font-medium text-zinc-100">{name}</span>
        {onLogs && (
          <button
            type="button"
            onClick={onLogs}
            title="Ver logs"
            className="rounded p-0.5 text-zinc-600 opacity-0 transition-opacity group-hover:opacity-100 hover:text-zinc-100"
          >
            <ScrollText className="size-3" />
          </button>
        )}
        <button
          type="button"
          onClick={onStop}
          disabled={busy}
          title="Parar"
          className="rounded p-0.5 text-zinc-500 transition-colors hover:text-rose-400 disabled:opacity-50"
        >
          {busy ? <Loader2 className="size-3 animate-spin" /> : <Square className="size-3" />}
        </button>
      </div>
      <div className="mt-0.5 flex flex-wrap gap-x-2 pl-3">
        {badge && <span className="text-[10px] text-zinc-500">{badge}…</span>}
        {urls.map((u) => (
          <button
            key={u}
            type="button"
            onClick={() => openUrl(u).catch(() => {})}
            className="font-mono text-[10px] text-zinc-400 underline-offset-2 hover:text-zinc-100 hover:underline"
          >
            {u.replace(/^https?:\/\/localhost/, "").replace(/\/$/, "") || u}
          </button>
        ))}
      </div>
    </div>
  );
}
