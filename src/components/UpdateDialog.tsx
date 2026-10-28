import { useEffect } from "react";
import { AlertTriangle, ArrowUpCircle, Download, Loader2, RotateCw, X } from "lucide-react";
import { useUpdaterContext } from "../hooks/useUpdater";
import { formatBytes } from "../lib/format";
import { percentOf, type UpdateInfo } from "../lib/updater";
import { BTN_GHOST, BTN_PRIMARY } from "../lib/ui";

/** New version, release notes and the download/install flow. Opened from the Titlebar or Settings. */
export function UpdateDialog() {
  const { state, currentVersion, closeDialog, install } = useUpdaterContext();
  const update: UpdateInfo | undefined =
    state.status === "available" || state.status === "downloading" || state.status === "ready"
      ? state.update
      : state.status === "error"
        ? state.update
        : undefined;
  const working = state.status === "downloading" || state.status === "ready";

  // Capture phase + preventDefault: Escape closes only this dialog, not Settings underneath.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      e.preventDefault();
      e.stopPropagation();
      closeDialog();
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [closeDialog]);

  // Checked again in the meantime and there is nothing to show anymore.
  if (!update) return null;

  const pct = state.status === "downloading" ? percentOf(state.downloaded, state.total) : state.status === "ready" ? 100 : null;

  return (
    <div className="fixed inset-0 z-[60] flex items-center justify-center bg-black/70 p-6" onMouseDown={closeDialog}>
      <div
        role="dialog"
        aria-modal
        aria-label="Atualização disponível"
        onMouseDown={(e) => e.stopPropagation()}
        className="flex max-h-[min(640px,100%)] w-[min(560px,100%)] flex-col overflow-hidden rounded-xl border border-zinc-800 bg-zinc-950 shadow-2xl shadow-black"
      >
        <header className="flex items-start gap-3.5 border-b border-zinc-800 bg-void px-6 py-5">
          <div className="flex size-10 shrink-0 items-center justify-center rounded-lg border border-zinc-800 bg-zinc-950">
            <ArrowUpCircle className="size-5 text-zinc-100" />
          </div>
          <div className="min-w-0 flex-1">
            <div className="text-[11px] font-medium tracking-[0.2em] text-zinc-500 uppercase">Atualização disponível</div>
            <h2 className="mt-0.5 flex items-baseline gap-2 text-lg font-semibold tracking-tight text-zinc-100">
              WSL Symbiosis <span className="font-mono">v{update.version}</span>
            </h2>
            <p className="mt-1 font-mono text-[11px] text-zinc-500">
              instalada: v{currentVersion ?? update.currentVersion}
              {update.date && ` · publicada em ${formatDate(update.date)}`}
            </p>
          </div>
          <button type="button" onClick={closeDialog} aria-label="Fechar" className="rounded-md p-1.5 text-zinc-500 hover:bg-zinc-900 hover:text-zinc-100">
            <X className="size-4" />
          </button>
        </header>

        <div className="min-h-0 flex-1 overflow-y-auto px-6 py-5">
          <div className="mb-2.5 text-[11px] font-semibold tracking-wider text-zinc-300 uppercase">Novidades</div>
          {update.notes ? (
            <ReleaseNotes text={update.notes} />
          ) : (
            <p className="text-xs text-zinc-500">Esta versão não trouxe notas de lançamento.</p>
          )}
        </div>

        {working && (
          <div className="border-t border-zinc-800 px-6 py-4">
            <div className="mb-2 flex items-center gap-2 text-xs">
              <Loader2 className="size-3.5 animate-spin text-zinc-400" />
              <span className="text-zinc-200">
                {state.status === "ready" ? "Instalando… o app vai reiniciar sozinho" : "Baixando atualização"}
              </span>
              <span className="ml-auto font-mono text-[11px] text-zinc-500 tabular-nums">
                {state.status === "downloading" &&
                  `${formatBytes(state.downloaded)}${state.total ? ` / ${formatBytes(state.total)}` : ""}${
                    pct !== null ? ` · ${pct.toFixed(0)}%` : ""
                  }`}
              </span>
            </div>
            <div
              className="meter-cells h-2 overflow-hidden bg-zinc-800"
              role="progressbar"
              aria-valuemin={0}
              aria-valuemax={100}
              aria-valuenow={pct ?? undefined}
            >
              <div
                className={`h-full bg-zinc-100 transition-[width] duration-200 ${pct === null ? "w-1/3 animate-pulse" : ""}`}
                style={pct !== null ? { width: `${pct}%` } : undefined}
              />
            </div>
          </div>
        )}

        <footer className="flex items-center gap-3 border-t border-zinc-800 bg-void px-6 py-3.5">
          {state.status === "error" ? (
            <span className="flex min-w-0 flex-1 items-center gap-1.5 text-xs text-rose-300">
              <AlertTriangle className="size-3.5 shrink-0" />
              <span className="truncate" title={state.message}>
                {state.message}
              </span>
            </span>
          ) : (
            <span className="min-w-0 flex-1 text-[11px] text-zinc-500">
              {!working && "Terminais abertos e projetos em execução serão encerrados."}
            </span>
          )}
          <button type="button" onClick={closeDialog} className={`${BTN_GHOST} h-8`}>
            {working ? "Continuar em segundo plano" : "Depois"}
          </button>
          <button type="button" onClick={install} disabled={working} className={`${BTN_PRIMARY} h-8 px-4`}>
            {working ? (
              <Loader2 className="size-3.5 animate-spin" />
            ) : state.status === "error" ? (
              <RotateCw className="size-3.5" />
            ) : (
              <Download className="size-3.5" />
            )}
            {state.status === "error"
              ? "Tentar de novo"
              : state.status === "downloading"
                ? "Baixando…"
                : state.status === "ready"
                  ? "Instalando…"
                  : "Baixar e instalar agora"}
          </button>
        </footer>
      </div>
    </div>
  );
}

function formatDate(iso: string): string {
  const d = new Date(iso);
  return Number.isNaN(d.getTime()) ? iso : d.toLocaleDateString("pt-BR");
}

/** Minimal rendering of the release notes (markdown headings and bullets; no markdown lib). */
function ReleaseNotes({ text }: { text: string }) {
  const clean = (s: string) => s.replace(/\*\*|__|`/g, "");
  return (
    <div className="space-y-1 text-[13px] leading-relaxed text-zinc-300">
      {text.split(/\r?\n/).map((line, i) => {
        const trimmed = line.trim();
        if (!trimmed) return <div key={i} className="h-1.5" />;
        const heading = /^#{1,6}\s+(.*)$/.exec(trimmed);
        if (heading) {
          return (
            <div key={i} className="pt-2 text-[11px] font-semibold tracking-wider text-zinc-400 uppercase first:pt-0">
              {clean(heading[1])}
            </div>
          );
        }
        const bullet = /^[-*+]\s+(.*)$/.exec(trimmed);
        if (bullet) {
          return (
            <div key={i} className={`flex gap-2 ${line.startsWith(" ") ? "pl-4" : ""}`}>
              <span className="mt-[9px] size-1 shrink-0 bg-zinc-500" />
              <span>{clean(bullet[1])}</span>
            </div>
          );
        }
        return <p key={i}>{clean(trimmed)}</p>;
      })}
    </div>
  );
}
