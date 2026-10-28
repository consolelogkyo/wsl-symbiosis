import { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { openUrl } from "@tauri-apps/plugin-opener";
import { ChevronDown, Download, Eraser, Loader2, Play, ScrollText, Square } from "lucide-react";
import type { DirEntry } from "../../lib/explorer";
import { ProjectInfo, inspectProject, isActive } from "../../lib/projects";
import { errorMessage } from "../../lib/wsl";
import { useActivityContext } from "../../hooks/useActivity";
import { HEAVY_FOLDERS } from "./ProjectBadges";
import { type MenuPlacement, closeOnOutside, placeMenu } from "../../lib/menu";

const CHIP = "flex shrink-0 items-center gap-1 rounded-full border px-2 py-0.5 text-[11px] font-medium transition-colors";

/**
 * Width of the actions slot (also used as a spacer by rows that aren't projects). Narrow rows
 * (container query on the row) keep only run/stop: install and cleanup chips are hidden.
 */
export const ACTIONS_SLOT = "w-40 @min-5xl:w-56";
/** Hidden on narrow rows. */
const WIDE_ONLY = "@max-5xl:hidden";
const MENU_WIDTH = 288;

interface Props {
  distro: string;
  entry: DirEntry;
  /** Asks the row to confirm `rm -rf <folder>/<heavy>`. */
  onCleanup: (heavy: string) => void;
}

/**
 * Right-hand slot of a project row: run/stop (Node), install/remove node_modules, and the
 * live state (URL while serving, spinner while installing). Fixed width so the "Abrir em"
 * buttons line up across rows.
 */
export function ProjectActions({ distro, entry, onCleanup }: Props) {
  const { start, stop, openLog, taskFor, urlsFor } = useActivityContext();
  const markers = entry.projectMarkers;
  const isNode = markers.includes("package.json");
  const hasNodeModules = markers.includes("node_modules");
  const task = taskFor(entry.path);
  const active = task && isActive(task);
  const heavy = HEAVY_FOLDERS.filter((h) => markers.includes(h) && !(isNode && h === "node_modules"));
  const stopClick = (e: React.MouseEvent) => e.stopPropagation();

  let main: React.ReactNode = null;
  if (task && active && task.kind === "install") {
    main = (
      <button type="button" onClick={() => openLog(task.id)} title="Ver saída da instalação" className={`${CHIP} border-sky-500/30 bg-sky-500/10 text-sky-300`}>
        <Loader2 className="size-3 animate-spin" />
        Instalando…
      </button>
    );
  } else if (task && active) {
    const url = urlsFor(task)[0];
    main = (
      <>
        {url ? (
          <button
            type="button"
            onClick={() => openUrl(url).catch(() => {})}
            title={`Abrir ${url}`}
            className={`${CHIP} border-emerald-500/30 bg-emerald-500/10 font-mono text-emerald-300 hover:bg-emerald-500/20`}
          >
            <span className="size-1.5 rounded-full bg-emerald-400" />
            {url.replace(/^https?:\/\/localhost/, "").replace(/\/$/, "")}
          </button>
        ) : (
          <button type="button" onClick={() => openLog(task.id)} title="Ver logs" className={`${CHIP} border-zinc-700 text-zinc-300`}>
            <Loader2 className="size-3 animate-spin" />
            {task.status === "stopping" ? "Parando…" : "Iniciando…"}
          </button>
        )}
        <button type="button" onClick={() => openLog(task.id)} title="Logs" className={`${WIDE_ONLY} rounded-full p-1 text-zinc-500 hover:bg-zinc-800 hover:text-zinc-100`}>
          <ScrollText className="size-3.5" />
        </button>
        <button
          type="button"
          onClick={() => stop(task)}
          disabled={task.status === "stopping"}
          title="Parar o projeto"
          className={`${CHIP} border-rose-500/40 text-rose-300 hover:bg-rose-500/15 disabled:opacity-50`}
        >
          <Square className="size-3 fill-current" />
          Parar
        </button>
      </>
    );
  } else if (isNode && !hasNodeModules) {
    main = (
      <button
        type="button"
        onClick={() => start(entry.path, "install")}
        title="Instala as dependências (npm/pnpm/yarn/bun install, conforme o lockfile)"
        className={`${CHIP} ${WIDE_ONLY} border-sky-500/30 bg-sky-500/10 text-sky-300 hover:border-sky-400/60 hover:bg-sky-500/20`}
      >
        <Download className="size-3" />
        Instalar
      </button>
    );
  } else if (isNode) {
    main = (
      <>
        <RunButton distro={distro} path={entry.path} onRun={(script) => start(entry.path, "run", script)} />
        <button
          type="button"
          onClick={() => onCleanup("node_modules")}
          title="rm -rf node_modules (libera espaço; reinstale quando precisar)"
          className={`${CHIP} ${WIDE_ONLY} border-amber-500/30 bg-amber-500/10 font-mono text-[10px] text-amber-300 hover:border-amber-400/60 hover:bg-amber-500/20`}
        >
          <Eraser className="size-3" />
          node_modules
        </button>
      </>
    );
  }

  const failed = task && !active && task.status === "failed";

  return (
    <div className={`flex ${ACTIONS_SLOT} shrink-0 items-center justify-end gap-1`} onClick={stopClick}>
      {failed && (
        <button
          type="button"
          onClick={() => openLog(task.id)}
          title={`${task.command} falhou (código ${task.exitCode ?? "?"}): ver logs`}
          className="flex items-center gap-1 rounded-full px-1.5 py-0.5 text-[10px] text-rose-300 hover:bg-rose-500/10"
        >
          <span className="size-1.5 rounded-full bg-rose-500" />
          falhou
        </button>
      )}
      {main}
      {heavy.map((h) => (
        <button
          key={h}
          type="button"
          onClick={() => onCleanup(h)}
          title={`rm -rf ${h} (limpeza rápida)`}
          className={`${CHIP} ${WIDE_ONLY} border-amber-500/30 bg-amber-500/10 font-mono text-[10px] text-amber-300 hover:border-amber-400/60 hover:bg-amber-500/20`}
        >
          <Eraser className="size-3" />
          {h}
        </button>
      ))}
    </div>
  );
}

/** "▶ Rodar" runs the suggested script (dev/start…); the chevron lists every script. */
function RunButton({ distro, path, onRun }: { distro: string; path: string; onRun: (script?: string) => void }) {
  const [menu, setMenu] = useState<MenuPlacement | null>(null);
  const [info, setInfo] = useState<ProjectInfo | null>(null);
  const [error, setError] = useState<string | null>(null);
  const anchor = useRef<HTMLButtonElement>(null);
  const panel = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!menu) return;
    setInfo(null);
    setError(null);
    inspectProject(distro, path).then(setInfo).catch((err) => setError(errorMessage(err)));
    // Scrolling the script list itself must not close the menu.
    return closeOnOutside(panel, () => setMenu(null));
  }, [menu, distro, path]);

  return (
    <div className="flex shrink-0 items-center rounded-full border border-emerald-500/30 bg-emerald-500/10 text-emerald-300">
      <button
        type="button"
        onClick={() => onRun()}
        title="Rodar o script de desenvolvimento (dev / start)"
        className="flex items-center gap-1 rounded-l-full py-0.5 pr-1.5 pl-2 text-[11px] font-semibold hover:bg-emerald-500/20"
      >
        <Play className="size-3 fill-current" />
        Rodar
      </button>
      <button
        ref={anchor}
        type="button"
        onClick={() => setMenu(menu ? null : placeMenu(anchor.current!.getBoundingClientRect(), MENU_WIDTH))}
        title="Escolher script"
        className="rounded-r-full border-l border-emerald-500/30 py-0.5 pr-1.5 pl-1 hover:bg-emerald-500/20"
      >
        <ChevronDown className="size-3" />
      </button>

      {menu &&
        createPortal(
          <div
            ref={panel}
            onPointerDown={(e) => e.stopPropagation()}
            style={{ top: menu.top, bottom: menu.bottom, left: menu.left, width: MENU_WIDTH, maxHeight: menu.maxHeight }}
            className="fixed z-50 flex flex-col overflow-hidden rounded-lg border border-zinc-700 bg-zinc-900 p-1 shadow-2xl shadow-black/70"
          >
            <div className="shrink-0 px-2.5 pt-1.5 pb-1 text-[10px] font-medium tracking-wider text-zinc-500 uppercase">
              Scripts {info && `· ${info.packageManager}`}
              {info && info.scripts.length > 0 && <span className="text-zinc-600"> · {info.scripts.length}</span>}
            </div>
            <div className="min-h-0 overflow-y-auto overscroll-contain">
              {!info && !error && (
                <div className="flex justify-center py-3">
                  <Loader2 className="size-4 animate-spin text-zinc-500" />
                </div>
              )}
              {error && <div className="px-2.5 py-2 text-xs text-rose-300">{error}</div>}
              {info?.scripts.length === 0 && <div className="px-2.5 py-2 text-xs text-zinc-500">Nenhum script no package.json.</div>}
              {info?.scripts.map((s) => (
                <button
                  key={s.name}
                  type="button"
                  onClick={() => {
                    setMenu(null);
                    onRun(s.name);
                  }}
                  className="flex w-full flex-col items-start rounded-md px-2.5 py-1.5 text-left hover:bg-zinc-800"
                >
                  <span className="flex items-center gap-2 text-sm font-medium text-zinc-100">
                    {s.name}
                    {s.name === info.suggested && (
                      <span className="rounded-full bg-zinc-100 px-1.5 text-[9px] font-semibold tracking-wider text-void uppercase">padrão</span>
                    )}
                  </span>
                  <span className="w-full truncate font-mono text-[11px] text-zinc-500">{s.command}</span>
                </button>
              ))}
            </div>
          </div>,
          document.body,
        )}
    </div>
  );
}
