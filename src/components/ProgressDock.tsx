import { useEffect, useState } from "react";
import { Check, Loader2, X } from "lucide-react";
import { useActivityContext } from "../hooks/useActivity";
import { type ProgressOp, useProgressContext } from "../hooks/useProgress";
import { type TaskInfo, getProjectLogTail, isActive } from "../lib/projects";
import { ProgressBar } from "./ProgressBar";

/** A dev server that hasn't printed its address yet is "starting"; after this, it's just running
 *  (scripts that never print a URL leave the panel, the sidebar keeps showing them). */
const RUN_STARTUP_MS = 90_000;

/** Long operations, bottom right: tracked actions (useProgress) + project tasks still getting
 *  ready (installs; dev servers until their address shows up). */
export function ProgressDock() {
  const { ops } = useProgressContext();
  const activity = useActivityContext();
  const [now, setNow] = useState(Date.now());

  const tasks = activity.tasks.filter(
    (t) =>
      isActive(t) &&
      t.status !== "stopping" &&
      (t.kind === "install" || (activity.urlsFor(t).length === 0 && now - t.startedAt < RUN_STARTUP_MS)),
  );
  const busy = ops.length > 0 || tasks.length > 0;

  // One clock for every card (elapsed time).
  useEffect(() => {
    if (!busy) return;
    const id = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(id);
  }, [busy]);

  if (!busy) return null;
  return (
    <>
      {tasks.map((t) => (
        <TaskCard key={t.id} task={t} now={now} onOpenLog={() => activity.openLog(t.id)} />
      ))}
      {ops.map((op) => (
        <Card key={op.id} op={op} now={now} />
      ))}
    </>
  );
}

function elapsed(ms: number): string {
  const s = Math.max(0, Math.floor(ms / 1000));
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
}

function Card({ op, now, onClick }: { op: ProgressOp; now: number; onClick?: () => void }) {
  const icon =
    op.status === "done" ? (
      <Check className="size-3.5 shrink-0 text-emerald-400" />
    ) : op.status === "error" ? (
      <X className="size-3.5 shrink-0 text-rose-400" />
    ) : (
      <Loader2 className="size-3.5 shrink-0 animate-spin text-zinc-300" />
    );
  return (
    <div
      role="status"
      onClick={onClick}
      className={`anim-rise pointer-events-auto rounded-lg border border-zinc-700 bg-zinc-900 px-3 py-2.5 shadow-2xl shadow-black/70 ${
        onClick ? "cursor-pointer hover:border-zinc-600" : ""
      }`}
      title={onClick ? "Ver o log completo" : undefined}
    >
      <div className="flex items-center gap-2">
        {icon}
        <span className="min-w-0 flex-1 truncate text-xs font-semibold text-zinc-100">{op.title}</span>
        <span className="font-mono text-[11px] text-zinc-500 tabular-nums">
          {op.status === "done" ? "pronto" : op.status === "error" ? "falhou" : elapsed(now - op.startedAt)}
        </span>
      </div>
      <div className="mt-1.5 flex items-center gap-2">
        <span
          className={`min-w-0 flex-1 truncate font-mono text-[10px] ${op.status === "error" ? "text-rose-300" : "text-zinc-500"}`}
          title={op.detail ?? undefined}
        >
          {op.detail ?? (op.status === "running" ? "Trabalhando…" : "")}
        </span>
        {op.percent !== null && op.status !== "error" && (
          <span className="font-mono text-[10px] text-zinc-400 tabular-nums">{Math.round(op.percent)}%</span>
        )}
      </div>
      <ProgressBar percent={op.status === "error" ? 100 : op.percent} className={`mt-1.5 h-1.5 ${op.status === "error" ? "opacity-40" : ""}`} />
    </div>
  );
}

/** A project task, with its last line of output as the status (polled while it's shown). */
function TaskCard({ task, now, onOpenLog }: { task: TaskInfo; now: number; onOpenLog: () => void }) {
  const [line, setLine] = useState<string | null>(null);
  useEffect(() => {
    let alive = true;
    const poll = () =>
      getProjectLogTail(task.id)
        .then((l) => alive && setLine(l))
        .catch(() => {});
    poll();
    const id = window.setInterval(poll, 1000);
    return () => {
      alive = false;
      window.clearInterval(id);
    };
  }, [task.id]);

  const op: ProgressOp = {
    id: task.id,
    title: task.kind === "install" ? `Instalando dependências · ${task.name}` : `Iniciando ${task.name}`,
    detail: line ?? task.command,
    percent: null,
    startedAt: task.startedAt,
    status: "running",
  };
  return <Card op={op} now={now} onClick={onOpenLog} />;
}
