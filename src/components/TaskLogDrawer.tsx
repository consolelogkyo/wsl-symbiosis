import { useEffect, useRef, useState } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { ExternalLink, Loader2, ScrollText, Square, X } from "lucide-react";
import { getProjectLog, isActive } from "../lib/projects";
import { useActivityContext } from "../hooks/useActivity";
import { BTN_DANGER, BTN_GHOST } from "../lib/ui";
import { TaskBadge } from "./RunningPanel";

const POLL_MS = 1000;

/** Slide-over with a task's live output (dev server / install logs). */
export function TaskLogDrawer() {
  const { tasks, logTaskId, openLog, stop, dismiss, urlsFor } = useActivityContext();
  const task = tasks.find((t) => t.id === logTaskId) ?? null;
  const [lines, setLines] = useState<string[]>([]);
  const bottomRef = useRef<HTMLDivElement>(null);
  const stick = useRef(true);
  const active = task ? isActive(task) : false;

  useEffect(() => {
    if (!logTaskId) return;
    let alive = true;
    const load = () =>
      getProjectLog(logTaskId)
        .then((l) => alive && setLines(l))
        .catch(() => {});
    load();
    // Finished tasks don't log anymore: one read is enough.
    const id = active ? window.setInterval(load, POLL_MS) : undefined;
    return () => {
      alive = false;
      window.clearInterval(id);
    };
  }, [logTaskId, active]);

  useEffect(() => {
    if (stick.current) bottomRef.current?.scrollIntoView({ block: "end" });
  }, [lines]);

  useEffect(() => {
    if (!logTaskId) return;
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && openLog(null);
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [logTaskId, openLog]);

  if (!task) return null;
  const urls = urlsFor(task);

  return (
    <div className="fixed inset-0 z-40 flex justify-end bg-black/60" onMouseDown={() => openLog(null)}>
      <aside
        onMouseDown={(e) => e.stopPropagation()}
        className="flex h-full w-[min(760px,65vw)] flex-col border-l border-zinc-800 bg-zinc-950 shadow-2xl shadow-black/60"
      >
        <header className="flex items-center gap-3 border-b border-zinc-800 bg-void px-4 py-3">
          <ScrollText className="size-4 shrink-0 text-zinc-100" />
          <div className="min-w-0 flex-1">
            <div className="flex items-center gap-2">
              <span className="truncate text-sm font-semibold text-zinc-100">{task.name}</span>
              <TaskBadge task={task} />
            </div>
            <div className="truncate font-mono text-[11px] text-zinc-500">
              {task.path} · {task.command}
              {task.pid !== null && ` · PID ${task.pid}`}
            </div>
          </div>
          {urls.map((u) => (
            <button key={u} type="button" onClick={() => openUrl(u).catch(() => {})} className={BTN_GHOST} title={`Abrir ${u}`}>
              <ExternalLink className="size-3.5" />
              {u.replace(/^https?:\/\//, "")}
            </button>
          ))}
          {active ? (
            <button type="button" onClick={() => stop(task)} disabled={task.status === "stopping"} className={BTN_DANGER}>
              {task.status === "stopping" ? <Loader2 className="size-3.5 animate-spin" /> : <Square className="size-3.5" />}
              Parar
            </button>
          ) : (
            <button
              type="button"
              onClick={() => {
                dismiss(task.id);
                openLog(null);
              }}
              className={BTN_GHOST}
            >
              Remover da lista
            </button>
          )}
          <button
            type="button"
            onClick={() => openLog(null)}
            aria-label="Fechar"
            className="rounded-md p-1.5 text-zinc-500 hover:bg-zinc-800 hover:text-zinc-200"
          >
            <X className="size-4" />
          </button>
        </header>
        <div
          className="min-h-0 flex-1 overflow-auto px-4 py-3 font-mono text-[12px] leading-5 text-zinc-300 select-text"
          onScroll={(e) => {
            const el = e.currentTarget;
            stick.current = el.scrollHeight - el.scrollTop - el.clientHeight < 40;
          }}
        >
          {lines.length === 0 ? (
            <span className="text-zinc-600">Aguardando saída…</span>
          ) : (
            lines.map((line, i) => (
              <div
                key={i}
                className={`whitespace-pre-wrap break-all ${
                  line.startsWith("$ ") ? "text-zinc-100" : line.startsWith("[processo") ? "text-zinc-500" : ""
                }`}
              >
                {line || " "}
              </div>
            ))
          )}
          <div ref={bottomRef} />
        </div>
      </aside>
    </div>
  );
}
