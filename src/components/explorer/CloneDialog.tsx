import { useEffect, useRef, useState } from "react";
import { AlertTriangle, FolderGit2, Loader2, X } from "lucide-react";
import { gitClone, isCloneUrl, repoNameFromUrl, splitGitError } from "../../lib/git";
import { shortenHome } from "../../lib/packages";
import { errorMessage } from "../../lib/wsl";
import { useActivityContext } from "../../hooks/useActivity";
import { BTN_GHOST, BTN_PRIMARY, FIELD, INPUT } from "../../lib/ui";

interface Props {
  distro: string;
  /** Folder the repository is cloned into (the one open in the tab). */
  parent: string;
  /** Existing names in `parent`, to warn before git refuses. */
  existing: string[];
  onClose: () => void;
  /** The clone finished (also after "Continuar em segundo plano"). */
  onCloned: (path: string) => void;
}

/** "git clone": paste an HTTPS or SSH link; the repository lands in a new folder here. */
export function CloneDialog({ distro, parent, existing, onClose, onCloned }: Props) {
  const [url, setUrl] = useState("");
  const [name, setName] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<{ title: string; detail: string } | null>(null);
  const { notify } = useActivityContext();
  const background = useRef(false);

  const derived = repoNameFromUrl(url);
  const folder = name.trim() || derived;
  const validUrl = isCloneUrl(url);
  const taken = !!folder && existing.includes(folder);
  const canClone = validUrl && !!folder && !taken && !busy;

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && !busy && onClose();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [busy, onClose]);

  const clone = async () => {
    if (!canClone) return;
    setBusy(true);
    setError(null);
    try {
      const result = await gitClone(distro, parent, url.trim(), name.trim() || null);
      onCloned(result.path);
      if (background.current) {
        notify({ tone: "success", title: `${result.name} clonado`, detail: shortenHome(result.path) + (result.viaSsh ? " · via SSH" : "") });
      } else {
        if (result.viaSsh) notify({ tone: "info", title: `${result.name} clonado via SSH`, detail: "O link HTTPS pediu senha (Configurações → Git)" });
        onClose();
      }
    } catch (err) {
      const e = splitGitError(errorMessage(err));
      if (background.current) notify({ tone: "error", title: `Falha ao clonar ${folder}`, detail: e.title });
      else setError(e);
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="fixed inset-0 z-50 flex items-start justify-center bg-black/70 p-6 pt-[15vh]" onMouseDown={() => !busy && onClose()}>
      <div
        role="dialog"
        aria-modal
        aria-label="Clonar repositório"
        onMouseDown={(e) => e.stopPropagation()}
        className="w-[min(560px,100%)] overflow-hidden rounded-xl border border-zinc-800 bg-zinc-950 shadow-2xl shadow-black"
      >
        <header className="flex items-start gap-3 border-b border-zinc-800 bg-void px-5 py-4">
          <FolderGit2 className="mt-0.5 size-4 shrink-0 text-zinc-100" />
          <div className="min-w-0 flex-1">
            <h2 className="text-sm font-semibold text-zinc-100">Clonar repositório</h2>
            <p className="mt-0.5 truncate font-mono text-[11px] text-zinc-500" title={parent}>
              em {shortenHome(parent)}/{folder || "…"}
            </p>
          </div>
          <button type="button" onClick={onClose} disabled={busy} aria-label="Fechar" className="rounded-md p-1 text-zinc-500 hover:bg-zinc-900 hover:text-zinc-100 disabled:opacity-40">
            <X className="size-4" />
          </button>
        </header>

        <form
          className="space-y-4 px-5 py-4"
          onSubmit={(e) => {
            e.preventDefault();
            clone();
          }}
        >
          <label className="block">
            <span className="mb-1.5 block text-[11px] font-medium tracking-wider text-zinc-500 uppercase">Link do repositório</span>
            <span className={`${FIELD} ${url && !validUrl ? "border-rose-500/50" : ""}`}>
              <input
                autoFocus
                value={url}
                disabled={busy}
                onChange={(e) => {
                  setUrl(e.target.value);
                  setError(null);
                }}
                spellCheck={false}
                placeholder="https://github.com/org/repo.git  ou  git@github.com:org/repo.git"
                className={`${INPUT} py-0.5 font-mono`}
              />
            </span>
            {url && !validUrl && (
              <span className="mt-1 block text-[11px] text-rose-300">Cole o endereço HTTPS (https://...) ou SSH (git@...:...) do repositório.</span>
            )}
          </label>

          <label className="block">
            <span className="mb-1.5 block text-[11px] font-medium tracking-wider text-zinc-500 uppercase">Nome da pasta</span>
            <span className={`${FIELD} ${taken ? "border-amber-500/50" : ""}`}>
              <input
                value={name}
                disabled={busy}
                onChange={(e) => {
                  setName(e.target.value);
                  setError(null);
                }}
                spellCheck={false}
                placeholder={derived || "igual ao do repositório"}
                className={`${INPUT} py-0.5 font-mono`}
              />
            </span>
            {taken && <span className="mt-1 block text-[11px] text-amber-300">Já existe "{folder}" nesta pasta: escolha outro nome.</span>}
          </label>

          {error && (
            <div className="rounded-md border border-rose-500/20 bg-rose-500/[0.04] px-3 py-2 text-xs text-rose-300">
              <div className="flex items-start gap-1.5">
                <AlertTriangle className="mt-px size-3.5 shrink-0" />
                <span>{error.title}</span>
              </div>
              {error.detail && (
                <pre className="mt-1.5 max-h-28 overflow-auto font-mono text-[10px] whitespace-pre-wrap text-rose-300/70">{error.detail}</pre>
              )}
            </div>
          )}

          <div className="flex items-center gap-2 pt-1">
            <span className="min-w-0 flex-1 text-[11px] text-zinc-600">
              {busy ? "Baixando… repositórios grandes podem levar alguns minutos." : "HTTPS que pedir senha usa o SSH (Configurações → Git)."}
            </span>
            {busy ? (
              <button
                type="button"
                onClick={() => {
                  background.current = true;
                  onClose();
                }}
                className={`${BTN_GHOST} h-8 whitespace-nowrap`}
              >
                Continuar em segundo plano
              </button>
            ) : (
              <button type="button" onClick={onClose} className={`${BTN_GHOST} h-8`}>
                Cancelar
              </button>
            )}
            <button type="submit" disabled={!canClone} className={`${BTN_PRIMARY} h-8 px-4 whitespace-nowrap`}>
              {busy ? <Loader2 className="size-3.5 animate-spin" /> : <FolderGit2 className="size-3.5" />}
              {busy ? "Clonando…" : "Clonar"}
            </button>
          </div>
        </form>
      </div>
    </div>
  );
}
