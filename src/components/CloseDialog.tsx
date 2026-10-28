import { useEffect, useRef, useState } from "react";
import { Loader2, LogOut, PanelBottomClose, X } from "lucide-react";
import { type CloseBehavior, type SettingsSnapshot, setCloseBehavior } from "../lib/settings";
import { errorMessage } from "../lib/wsl";
import { hideToTray, quitApplication } from "../lib/tray";

interface Props {
  /** Dev servers started from the app (they keep running in the tray, or stop on quit). */
  runningProjects: number;
  onCancel: () => void;
  /** "Lembrar minha escolha" saved a new closeBehavior. */
  onRemembered: (saved: SettingsSnapshot) => void;
}

/** ✕ / Alt+F4 with `closeBehavior: "ask"`: keep running in the tray, or quit for real. */
export function CloseDialog({ runningProjects, onCancel, onRemembered }: Props) {
  const [remember, setRemember] = useState(false);
  const [busy, setBusy] = useState<CloseBehavior | null>(null);
  const [error, setError] = useState<string | null>(null);
  const trayButton = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    trayButton.current?.focus();
    // Capture phase: Escape cancels only this dialog, not whatever is open underneath.
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      e.preventDefault();
      e.stopPropagation();
      onCancel();
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [onCancel]);

  const choose = async (behavior: "tray" | "quit") => {
    setBusy(behavior);
    setError(null);
    try {
      if (remember) onRemembered(await setCloseBehavior(behavior));
      if (behavior === "tray") {
        await hideToTray();
        onCancel();
      } else {
        await quitApplication();
      }
    } catch (err) {
      setError(errorMessage(err));
      setBusy(null);
    }
  };

  const servers =
    runningProjects === 1 ? "o servidor em execução continua" : runningProjects > 1 ? `os ${runningProjects} servidores em execução continuam` : "seus servidores continuam";

  return (
    <div className="fixed inset-0 z-[70] flex items-center justify-center bg-black/70 p-6" onMouseDown={onCancel}>
      <div
        role="dialog"
        aria-modal
        aria-label="Fechar o WSL Symbiosis"
        onMouseDown={(e) => e.stopPropagation()}
        className="w-[min(520px,100%)] overflow-hidden rounded-xl border border-zinc-800 bg-zinc-950 shadow-2xl shadow-black"
      >
        <header className="flex items-start gap-3 border-b border-zinc-800 bg-void px-6 py-4">
          <div className="min-w-0 flex-1">
            <div className="text-[11px] font-medium tracking-[0.2em] text-zinc-500 uppercase">Fechar a janela</div>
            <h2 className="mt-0.5 text-base font-semibold tracking-tight text-zinc-100">O que o WSL Symbiosis deve fazer?</h2>
          </div>
          <button type="button" onClick={onCancel} aria-label="Cancelar" className="rounded-md p-1.5 text-zinc-500 hover:bg-zinc-900 hover:text-zinc-100">
            <X className="size-4" />
          </button>
        </header>

        <div className="space-y-2.5 px-6 py-5">
          <button
            ref={trayButton}
            type="button"
            onClick={() => choose("tray")}
            disabled={busy !== null}
            className="group flex w-full items-start gap-3.5 rounded-lg border border-zinc-100 bg-zinc-100 px-4 py-3.5 text-left text-void transition-colors hover:bg-white focus:outline-none focus-visible:ring-2 focus-visible:ring-zinc-100/40 focus-visible:ring-offset-2 focus-visible:ring-offset-zinc-950 disabled:opacity-60"
          >
            {busy === "tray" ? <Loader2 className="mt-0.5 size-4 shrink-0 animate-spin" /> : <PanelBottomClose className="mt-0.5 size-4 shrink-0" />}
            <span className="min-w-0">
              <span className="block text-sm font-semibold">Manter em segundo plano (bandeja)</span>
              <span className="mt-0.5 block text-xs leading-relaxed text-zinc-600">
                O app fica nos ícones ocultos da barra de tarefas monitorando o WSL, e {servers} no ar. Clique no ícone
                para voltar.
              </span>
            </span>
          </button>

          <button
            type="button"
            onClick={() => choose("quit")}
            disabled={busy !== null}
            className="group flex w-full items-start gap-3.5 rounded-lg border border-zinc-800 px-4 py-3.5 text-left transition-colors hover:border-rose-500/50 focus:outline-none focus-visible:border-rose-500/50 disabled:opacity-60"
          >
            {busy === "quit" ? (
              <Loader2 className="mt-0.5 size-4 shrink-0 animate-spin text-zinc-400" />
            ) : (
              <LogOut className="mt-0.5 size-4 shrink-0 text-zinc-400 transition-colors group-hover:text-rose-300" />
            )}
            <span className="min-w-0">
              <span className="block text-sm font-semibold text-zinc-100 transition-colors group-hover:text-rose-300">
                Fechar definitivamente
              </span>
              <span className="mt-0.5 block text-xs leading-relaxed text-zinc-500">
                Encerra os terminais, para os servidores iniciados pelo app e sai.
              </span>
            </span>
          </button>
        </div>

        <footer className="flex items-center gap-3 border-t border-zinc-800 bg-void px-6 py-3">
          <label className="flex flex-1 cursor-pointer items-center gap-2.5 text-xs text-zinc-400 select-none">
            <input
              type="checkbox"
              checked={remember}
              onChange={(e) => setRemember(e.target.checked)}
              className="size-3.5 accent-zinc-100"
            />
            Lembrar minha escolha
            <span className="text-zinc-600">(dá para mudar nas Configurações)</span>
          </label>
          {error && <span className="truncate text-xs text-rose-300" title={error}>{error}</span>}
        </footer>
      </div>
    </div>
  );
}
