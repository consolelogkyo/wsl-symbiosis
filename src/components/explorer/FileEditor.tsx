import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { AlertTriangle, FileText, Loader2, Lock, Save, X } from "lucide-react";
import { TextFileContent, readTextFile, writeTextFile } from "../../lib/explorer";
import { errorMessage } from "../../lib/wsl";
import { formatBytes } from "../../lib/format";
import { BTN_DANGER_SOLID, BTN_GHOST, BTN_PRIMARY } from "../../lib/ui";

interface Props {
  distro: string;
  path: string;
  onClose: () => void;
  onSaved: () => void;
}

type Status = "loading" | "clean" | "dirty" | "saving" | "error";

export function FileEditor({ distro, path, onClose, onSaved }: Props) {
  const [file, setFile] = useState<TextFileContent | null>(null);
  const [text, setText] = useState("");
  const [saved, setSaved] = useState("");
  const [status, setStatus] = useState<Status>("loading");
  const [error, setError] = useState<string | null>(null);
  const [confirmClose, setConfirmClose] = useState(false);
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const gutterRef = useRef<HTMLPreElement>(null);

  useEffect(() => {
    let alive = true;
    setStatus("loading");
    setFile(null);
    setError(null);
    readTextFile(distro, path)
      .then((f) => {
        if (!alive) return;
        // The textarea normalizes CRLF to LF; mirror that so "dirty" means a real edit.
        const content = f.content.replace(/\r\n/g, "\n");
        setFile(f);
        setText(content);
        setSaved(content);
        setStatus("clean");
      })
      .catch((err) => {
        if (!alive) return;
        setError(errorMessage(err));
        setStatus("error");
      });
    return () => {
      alive = false;
    };
  }, [distro, path]);

  const readOnly = !file || file.binary || file.truncated;
  const dirty = text !== saved;
  const lineCount = useMemo(() => text.split("\n").length, [text]);
  const gutter = useMemo(
    () => Array.from({ length: lineCount }, (_, i) => i + 1).join("\n"),
    [lineCount],
  );

  const save = useCallback(async () => {
    if (readOnly || !dirty || status === "saving") return;
    setStatus("saving");
    try {
      await writeTextFile(distro, path, text);
      setSaved(text);
      setStatus("clean");
      setError(null);
      onSaved();
    } catch (err) {
      setError(errorMessage(err));
      setStatus("dirty");
    }
  }, [readOnly, dirty, status, distro, path, text, onSaved]);

  const requestClose = useCallback(() => {
    if (dirty) setConfirmClose(true);
    else onClose();
  }, [dirty, onClose]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "s") {
        e.preventDefault();
        save();
      } else if (e.key === "Escape") {
        requestClose();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [save, requestClose]);

  const name = path.split("/").pop() ?? path;
  const shownStatus: Status = status === "clean" && dirty ? "dirty" : status;

  return (
    <div role="dialog" aria-modal className="fixed inset-0 z-40 flex justify-end bg-black/60" onMouseDown={requestClose}>
      <aside
        onMouseDown={(e) => e.stopPropagation()}
        className="flex h-full w-[min(860px,70vw)] flex-col border-l border-zinc-800 bg-zinc-950 shadow-2xl shadow-black/60"
      >
        <header className="flex items-center gap-3 border-b border-zinc-800 bg-void px-4 py-3">
          <FileText className="size-4 shrink-0 text-zinc-100" />
          <div className="min-w-0 flex-1">
            <div className="truncate text-sm font-medium text-zinc-100">{name}</div>
            <div className="truncate font-mono text-[11px] text-zinc-500">{path}</div>
          </div>
          <StatusPill status={shownStatus} readOnly={readOnly && status !== "loading" && status !== "error"} />
          {confirmClose ? (
            <div className="flex items-center gap-1.5">
              <span className="text-xs text-zinc-300">Descartar alterações?</span>
              <button
                type="button"
                onClick={onClose}
                className={BTN_DANGER_SOLID}
              >
                Descartar
              </button>
              <button
                type="button"
                onClick={() => setConfirmClose(false)}
                className={`${BTN_GHOST} py-0.5 text-[11px]`}
              >
                Continuar editando
              </button>
            </div>
          ) : (
            <>
              <button
                type="button"
                onClick={save}
                disabled={readOnly || !dirty || status === "saving"}
                className={BTN_PRIMARY}
              >
                <Save className="size-3.5" />
                Salvar
              </button>
              <button
                type="button"
                onClick={requestClose}
                aria-label="Fechar"
                className="rounded-md p-1.5 text-zinc-500 hover:bg-zinc-800 hover:text-zinc-200"
              >
                <X className="size-4" />
              </button>
            </>
          )}
        </header>

        {error && (
          <div className="border-b border-rose-500/20 px-4 py-2 font-mono text-xs text-rose-300">{error}</div>
        )}
        {file && !file.binary && (file.truncated || file.lineEnding === "crlf" || file.lineEnding === "mixed") && (
          <div className="flex items-center gap-2 border-b border-zinc-800 bg-zinc-900 px-4 py-2 text-xs text-zinc-300">
            <AlertTriangle className="size-3.5 shrink-0 text-amber-400" />
            {file.truncated
              ? `Arquivo grande (${formatBytes(file.size)}): mostrando só o início, em modo somente leitura.`
              : "O arquivo usa quebras de linha CRLF; ao salvar elas serão convertidas para LF."}
          </div>
        )}

        <div className="relative min-h-0 flex-1">
          {status === "loading" ? (
            <div className="flex h-full items-center justify-center">
              <Loader2 className="size-5 animate-spin text-zinc-600" />
            </div>
          ) : file?.binary ? (
            <div className="flex h-full flex-col items-center justify-center gap-2 text-center">
              <Lock className="size-5 text-zinc-500" />
              <p className="text-sm text-zinc-300">Arquivo binário ou fora de UTF-8</p>
              <p className="text-xs text-zinc-500">{formatBytes(file.size)} · não é possível editar aqui</p>
            </div>
          ) : file ? (
            <div className="flex h-full font-mono text-[13px] leading-5">
              <pre
                ref={gutterRef}
                aria-hidden
                className="w-12 shrink-0 overflow-hidden border-r border-zinc-800/80 bg-zinc-950 py-3 pr-2 text-right text-zinc-600 select-none"
              >
                {gutter}
              </pre>
              <textarea
                ref={textareaRef}
                value={text}
                readOnly={readOnly}
                spellCheck={false}
                autoFocus
                wrap="off"
                onChange={(e) => setText(e.target.value)}
                onScroll={(e) => {
                  if (gutterRef.current) gutterRef.current.scrollTop = e.currentTarget.scrollTop;
                }}
                onKeyDown={(e) => {
                  if (e.key !== "Tab" || readOnly) return;
                  e.preventDefault();
                  const t = e.currentTarget;
                  const { selectionStart: s, selectionEnd: end } = t;
                  setText(text.slice(0, s) + "  " + text.slice(end));
                  requestAnimationFrame(() => t.setSelectionRange(s + 2, s + 2));
                }}
                className="h-full flex-1 resize-none bg-zinc-950 px-3 py-3 text-zinc-200 caret-zinc-100 select-text focus:outline-none"
              />
            </div>
          ) : null}
        </div>

        <footer className="flex items-center gap-4 border-t border-zinc-800 bg-void px-4 py-1.5 font-mono text-[11px] text-zinc-500">
          <span>{lineCount} linhas</span>
          <span>{formatBytes(new TextEncoder().encode(text).length)}</span>
          <span>UTF-8</span>
          <span className="text-zinc-300">LF</span>
          <span className="ml-auto font-sans">Ctrl+S salva · Esc fecha</span>
        </footer>
      </aside>
    </div>
  );
}

function StatusPill({ status, readOnly }: { status: Status; readOnly: boolean }) {
  if (readOnly) {
    return (
      <span className="flex items-center gap-1 rounded-full border border-zinc-800 px-2 py-0.5 text-[11px] text-zinc-400">
        <Lock className="size-3" /> Somente leitura
      </span>
    );
  }
  switch (status) {
    case "dirty":
      return (
        <span className="flex items-center gap-1.5 rounded-full border border-zinc-700 px-2 py-0.5 text-[11px] text-zinc-200">
          <span className="size-1.5 rounded-full bg-amber-400" /> Modificado
        </span>
      );
    case "saving":
      return (
        <span className="flex items-center gap-1 rounded-full border border-zinc-800 px-2 py-0.5 text-[11px] text-zinc-400">
          <Loader2 className="size-3 animate-spin" /> Salvando
        </span>
      );
    case "clean":
      return (
        <span className="flex items-center gap-1.5 rounded-full border border-zinc-800 px-2 py-0.5 text-[11px] text-zinc-400">
          <span className="size-1.5 rounded-full bg-emerald-400" /> Salvo
        </span>
      );
    default:
      return null;
  }
}
