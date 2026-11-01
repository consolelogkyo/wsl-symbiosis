import { useEffect, useState, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { AlertTriangle, Check, Copy, X } from "lucide-react";
import { CONTAINER_STATE_LABEL } from "../../lib/docker";
import { BTN_GHOST } from "../../lib/ui";

/** Modal frame of this screen (portal: see CLAUDE.md, dialogs inside views). */
export function DialogShell(p: {
  icon: ReactNode;
  eyebrow: string;
  title: string;
  description?: ReactNode;
  /** Blocks Escape, the backdrop and ✕ while work is running. */
  busy?: boolean;
  width?: string;
  onClose: () => void;
  footer?: ReactNode;
  children: ReactNode;
}) {
  const { busy, onClose } = p;
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape" || busy) return;
      e.preventDefault();
      e.stopPropagation();
      onClose();
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [busy, onClose]);

  return createPortal(
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/70 p-6" onMouseDown={() => !busy && onClose()}>
      <div
        role="dialog"
        aria-modal
        aria-label={p.title}
        onMouseDown={(e) => e.stopPropagation()}
        className={`flex max-h-full ${p.width ?? "w-[min(580px,100%)]"} flex-col overflow-hidden rounded-xl border border-zinc-800 bg-zinc-950 shadow-2xl shadow-black`}
      >
        <header className="flex shrink-0 items-start gap-3.5 border-b border-zinc-800 bg-void px-6 py-5">
          <div className="flex size-10 shrink-0 items-center justify-center rounded-lg border border-zinc-800 bg-zinc-950">{p.icon}</div>
          <div className="min-w-0 flex-1">
            <div className="text-[11px] font-medium tracking-[0.2em] text-zinc-500 uppercase">{p.eyebrow}</div>
            <h2 className="mt-0.5 text-lg font-semibold tracking-tight text-zinc-100">{p.title}</h2>
            {p.description && <div className="mt-1 text-xs leading-relaxed text-zinc-500">{p.description}</div>}
          </div>
          <button
            type="button"
            onClick={onClose}
            disabled={busy}
            aria-label="Fechar"
            className="rounded-md p-1.5 text-zinc-500 hover:bg-zinc-900 hover:text-zinc-100 disabled:opacity-30"
          >
            <X className="size-4" />
          </button>
        </header>
        <div className="min-h-0 flex-1 overflow-y-auto">{p.children}</div>
        {p.footer && (
          <footer className="flex shrink-0 items-center justify-end gap-2 border-t border-zinc-800 bg-void px-6 py-3.5">{p.footer}</footer>
        )}
      </div>
    </div>,
    document.body,
  );
}

/** Copies `value`; the label turns into "Copiado!" for a moment. */
export function CopyButton({ value, label, className = BTN_GHOST, title }: { value: string; label: string; className?: string; title?: string }) {
  const [copied, setCopied] = useState(false);
  useEffect(() => {
    if (!copied) return;
    const id = window.setTimeout(() => setCopied(false), 1600);
    return () => window.clearTimeout(id);
  }, [copied]);
  return (
    <button
      type="button"
      title={title}
      onClick={() =>
        navigator.clipboard
          .writeText(value)
          .then(() => setCopied(true))
          .catch(() => {})
      }
      className={`${className} whitespace-nowrap`}
    >
      {copied ? <Check className="size-3.5 text-emerald-400" /> : <Copy className="size-3.5" />}
      {copied ? "Copiado!" : label}
    </button>
  );
}

export function StateBadge({ state, title }: { state: string; title?: string }) {
  const dot =
    state === "running" ? "bg-emerald-400" : state === "restarting" || state === "paused" ? "bg-amber-400" : state === "dead" ? "bg-rose-400" : "bg-zinc-600";
  return (
    <span title={title} className="flex shrink-0 items-center gap-1.5 text-[11px] whitespace-nowrap text-zinc-400">
      <span className={`size-1.5 rounded-full ${dot}`} />
      {CONTAINER_STATE_LABEL[state] ?? state}
    </span>
  );
}

export function ErrorBox({ children }: { children: ReactNode }) {
  return (
    <div className="flex items-start gap-2.5 rounded-lg border border-rose-500/20 bg-rose-500/[0.04] px-4 py-3 text-xs text-rose-300">
      <AlertTriangle className="mt-px size-3.5 shrink-0" />
      <span className="min-w-0 break-words whitespace-pre-wrap">{children}</span>
    </div>
  );
}

/** Masks the password in a connection URL for display (`postgres:••••@`). */
export function maskUrl(url: string, password: string | null): string {
  if (!password) return url;
  return url.replace(/(:\/\/[^:/@]*:)[^@]*@/, "$1••••••@");
}
