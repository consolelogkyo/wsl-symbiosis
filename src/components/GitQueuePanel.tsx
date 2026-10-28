import { useState } from "react";
import { Check, ChevronDown, ChevronUp, Clock, CloudDownload, Loader2, Minus, RotateCcw, X } from "lucide-react";
import { type PullItem, useGitQueueContext } from "../hooks/useGitQueue";
import { formatStat } from "../lib/git";

/** Progress of the git pull queue: one line per repository, in queue order. */
export function GitQueuePanel() {
  const { items, cancelPending, retryFailed, clear } = useGitQueueContext();
  const [collapsed, setCollapsed] = useState(false);
  const [open, setOpen] = useState<number | null>(null);
  if (items.length === 0) return null;

  const done = items.filter((i) => i.status !== "queued" && i.status !== "running").length;
  const running = items.some((i) => i.status === "running" || i.status === "queued");
  const failed = items.filter((i) => i.status === "failed").length;
  const retryable = items.some((i) => i.status === "failed" || i.status === "cancelled");

  return (
    <div className="pointer-events-auto flex max-h-[60vh] flex-col overflow-hidden rounded-lg border border-zinc-700 bg-zinc-900 shadow-2xl shadow-black/70">
      <div className="flex shrink-0 items-center gap-2 px-3 py-2.5">
        {running ? <Loader2 className="size-3.5 shrink-0 animate-spin text-zinc-300" /> : <CloudDownload className="size-3.5 shrink-0 text-zinc-300" />}
        <span className="min-w-0 flex-1 truncate text-sm font-semibold text-zinc-100">
          {running ? "Atualizando repositórios" : "git pull concluído"}
        </span>
        <span className="font-mono text-[11px] text-zinc-500 tabular-nums">
          {done}/{items.length}
        </span>
        <button
          type="button"
          onClick={() => setCollapsed((c) => !c)}
          aria-label={collapsed ? "Expandir" : "Recolher"}
          className="rounded p-0.5 text-zinc-500 hover:text-zinc-100"
        >
          {collapsed ? <ChevronUp className="size-3.5" /> : <ChevronDown className="size-3.5" />}
        </button>
        {!running && (
          <button type="button" onClick={clear} aria-label="Fechar" className="rounded p-0.5 text-zinc-500 hover:text-zinc-100">
            <X className="size-3.5" />
          </button>
        )}
      </div>
      <div className="h-0.5 shrink-0 bg-zinc-800">
        <div className="h-full bg-zinc-100 transition-[width]" style={{ width: `${(done / items.length) * 100}%` }} />
      </div>

      {!collapsed && (
        <>
          <div className="min-h-0 flex-1 overflow-y-auto overscroll-contain py-1">
            {items.map((item) => (
              <Row key={item.id} item={item} open={open === item.id} onToggle={() => setOpen(open === item.id ? null : item.id)} />
            ))}
          </div>
          <div className="flex shrink-0 items-center gap-1.5 border-t border-zinc-800 px-2 py-2">
            {failed > 0 && <span className="pl-1 text-[11px] text-rose-300">{failed} com erro</span>}
            <span className="ml-auto flex gap-1.5">
              {running && items.some((i) => i.status === "queued") && (
                <button
                  type="button"
                  onClick={cancelPending}
                  className="rounded-full border border-zinc-700 px-2.5 py-0.5 text-[11px] font-medium text-zinc-300 hover:border-zinc-500 hover:text-zinc-100"
                >
                  Cancelar fila
                </button>
              )}
              {!running && retryable && (
                <button
                  type="button"
                  onClick={retryFailed}
                  className="inline-flex items-center gap-1 rounded-full border border-zinc-700 px-2.5 py-0.5 text-[11px] font-medium text-zinc-300 hover:border-zinc-500 hover:text-zinc-100"
                >
                  <RotateCcw className="size-3" />
                  Tentar de novo
                </button>
              )}
            </span>
          </div>
        </>
      )}
    </div>
  );
}

function Row({ item, open, onToggle }: { item: PullItem; open: boolean; onToggle: () => void }) {
  const r = item.result;
  const summary =
    item.status === "updated" && r
      ? [`${r.commits} commit${r.commits > 1 ? "s" : ""}`, formatStat(r.stat)].filter(Boolean).join(" · ")
      : item.status === "upToDate"
        ? "em dia"
        : item.status === "failed"
          ? item.error?.title
          : item.status === "cancelled"
            ? "cancelado"
            : item.status === "running"
              ? "atualizando…"
              : "na fila";
  const failed = item.status === "failed";

  return (
    <div className={failed ? "cursor-pointer" : ""} onClick={failed ? onToggle : undefined}>
      <div className="flex items-center gap-2 px-3 py-1" title={`${item.path}${r ? ` · ${r.branch}` : ""}`}>
        <StatusIcon status={item.status} />
        <span
          className={`max-w-[55%] min-w-0 shrink-0 truncate text-xs ${
            item.status === "queued" || item.status === "cancelled" ? "text-zinc-500" : "text-zinc-200"
          }`}
        >
          {item.name}
        </span>
        {r && <span className="shrink-0 font-mono text-[10px] text-zinc-600">{r.branch}</span>}
        {r?.viaSsh && (
          <span
            title="O remoto é HTTPS e pediu senha: o app usou o SSH (Configurações → Git)"
            className="shrink-0 rounded-sm border border-zinc-700 px-1 font-mono text-[9px] text-zinc-400"
          >
            SSH
          </span>
        )}
        <span
          className={`ml-auto min-w-0 flex-1 truncate text-right text-[11px] ${
            failed ? "text-rose-300" : item.status === "updated" ? "font-mono text-zinc-300" : "text-zinc-500"
          }`}
        >
          {summary}
        </span>
      </div>
      {failed && open && item.error && (
        <div className="mx-3 mb-1.5 rounded border border-rose-500/20 bg-rose-500/[0.04] px-2 py-1.5">
          <p className="text-[11px] text-rose-300">{item.error.title}</p>
          {item.error.detail && (
            <pre className="mt-1 max-h-28 overflow-auto font-mono text-[10px] whitespace-pre-wrap text-rose-300/70">{item.error.detail}</pre>
          )}
        </div>
      )}
    </div>
  );
}

function StatusIcon({ status }: { status: PullItem["status"] }) {
  switch (status) {
    case "running":
      return <Loader2 className="size-3.5 shrink-0 animate-spin text-zinc-300" />;
    case "queued":
      return <Clock className="size-3.5 shrink-0 text-zinc-600" />;
    case "updated":
      return <Check className="size-3.5 shrink-0 text-emerald-400" />;
    case "upToDate":
      return <Check className="size-3.5 shrink-0 text-zinc-500" />;
    case "failed":
      return <X className="size-3.5 shrink-0 text-rose-400" />;
    case "cancelled":
      return <Minus className="size-3.5 shrink-0 text-zinc-600" />;
  }
}
