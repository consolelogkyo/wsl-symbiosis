import { openUrl } from "@tauri-apps/plugin-opener";
import { ExternalLink, ScrollText, X } from "lucide-react";
import { useActivityContext } from "../hooks/useActivity";

const DOT = { success: "bg-emerald-400", error: "bg-rose-500", info: "bg-zinc-300" } as const;

/** Bottom-right notifications: project up (with its URL), install done, failures. Rendered
 *  inside the App's bottom-right stack, above the git queue panel. */
export function Toaster() {
  const { toasts, dismissToast, openLog } = useActivityContext();
  if (toasts.length === 0) return null;
  return (
    <>
      {toasts.map((t) => (
        <div
          key={t.id}
          role="status"
          className="pointer-events-auto rounded-lg border border-zinc-700 bg-zinc-900 p-3 shadow-2xl shadow-black/70"
        >
          <div className="flex items-center gap-2">
            <span className={`size-1.5 shrink-0 rounded-full ${DOT[t.tone]}`} />
            <span className="min-w-0 flex-1 truncate text-sm font-semibold text-zinc-100">{t.title}</span>
            <button
              type="button"
              onClick={() => dismissToast(t.id)}
              aria-label="Fechar aviso"
              className="rounded p-0.5 text-zinc-500 hover:text-zinc-100"
            >
              <X className="size-3.5" />
            </button>
          </div>
          {t.detail && (
            <p className={`mt-1 truncate pl-3.5 text-xs ${t.url ? "font-mono text-zinc-200" : "text-zinc-400"}`} title={t.detail}>
              {t.detail}
            </p>
          )}
          {(t.url || t.taskId) && (
            <div className="mt-2.5 flex gap-1.5 pl-3.5">
              {t.url && (
                <button
                  type="button"
                  onClick={() => openUrl(t.url!).catch(() => {})}
                  className="inline-flex items-center gap-1.5 rounded-full bg-zinc-100 px-2.5 py-0.5 text-[11px] font-semibold text-void hover:bg-white"
                >
                  <ExternalLink className="size-3" />
                  Abrir
                </button>
              )}
              {t.taskId && (
                <button
                  type="button"
                  onClick={() => {
                    openLog(t.taskId!);
                    dismissToast(t.id);
                  }}
                  className="inline-flex items-center gap-1.5 rounded-full border border-zinc-700 px-2.5 py-0.5 text-[11px] font-medium text-zinc-300 hover:border-zinc-500 hover:text-zinc-100"
                >
                  <ScrollText className="size-3" />
                  Logs
                </button>
              )}
            </div>
          )}
        </div>
      ))}
    </>
  );
}
