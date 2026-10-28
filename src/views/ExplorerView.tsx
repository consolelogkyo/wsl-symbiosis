import { useCallback, useEffect, useState, type ReactNode } from "react";
import { Folder, Loader2, Play, Plus, PowerOff, X } from "lucide-react";
import { useActivityContext } from "../hooks/useActivity";
import { isActive, type TaskInfo } from "../lib/projects";
import type { Distro } from "../lib/wsl";
import { type ToolId, parentPath } from "../lib/explorer";
import { shortenHome } from "../lib/packages";
import type { PendingAction } from "../hooks/useWsl";
import { BTN_PRIMARY } from "../lib/ui";
import { type Shortcuts, formatBinding, formatGestures, matchesClick, matchesKey } from "../lib/shortcuts";
import { ExplorerPane } from "../components/explorer/ExplorerPane";

interface Tab {
  key: string;
  distro: string;
  /** Where the tab opened (null = home); the pane navigates on its own from there. */
  initialPath: string | null;
  /** Folder the pane is showing, reported back for the label and "Nova aba". */
  path: string | null;
}

interface Props {
  distro: Distro | null;
  distros: Distro[];
  /** The Explorer screen is on display (it stays mounted, hidden, so tabs survive). */
  active: boolean;
  pending: PendingAction;
  onStart: () => void;
  onTerminalHere: (path: string) => void;
  /** IDE configured as default in the settings (the labeled "Abrir em" button). */
  defaultIde: ToolId | null;
  /** Tab shortcuts, configurable in Settings → Atalhos. */
  shortcuts: Shortcuts;
}

let nextKey = 1;

function tabLabel(path: string | null): string {
  if (!path) return "~";
  const short = shortenHome(path);
  if (short === "~" || short === "/") return short;
  return short.split("/").pop() || short;
}

/** Tabs are per distro: the bar shows the selected distro's tabs; switching back restores them. */
export function ExplorerView({ distro, distros, active, pending, onStart, onTerminalHere, defaultIde, shortcuts }: Props) {
  const name = distro?.name ?? null;
  const running = distro?.state === "Running";
  const [tabs, setTabs] = useState<Tab[]>([]);
  const [activeByDistro, setActiveByDistro] = useState<Record<string, string>>({});
  const { tasks } = useActivityContext();

  const visibleTabs = tabs.filter((t) => t.distro === name);
  const activeKey =
    (name && visibleTabs.some((t) => t.key === activeByDistro[name]) ? activeByDistro[name] : visibleTabs[0]?.key) ?? null;
  const activeTab = visibleTabs.find((t) => t.key === activeKey) ?? null;

  /** Opens a tab next to the active one (like a browser); `focus: false` = background tab. */
  const openTab = useCallback(
    (tabDistro: string, path: string | null, focus = true) => {
      const key = `explorer-${nextKey++}`;
      setTabs((t) => {
        const at = t.findIndex((x) => x.key === activeKey);
        const next = [...t];
        next.splice(at < 0 ? t.length : at + 1, 0, { key, distro: tabDistro, initialPath: path, path });
        return next;
      });
      if (focus) setActiveByDistro((a) => ({ ...a, [tabDistro]: key }));
    },
    [activeKey],
  );

  const closeTab = useCallback(
    (key: string) => {
      const same = tabs.filter((t) => t.distro === name);
      if (same.length <= 1) return; // a distro always keeps one tab
      if (key === activeKey && name) {
        const idx = same.findIndex((t) => t.key === key);
        const neighbor = same[idx + 1] ?? same[idx - 1];
        setActiveByDistro((a) => ({ ...a, [name]: neighbor.key }));
      }
      setTabs((t) => t.filter((x) => x.key !== key));
    },
    [tabs, name, activeKey],
  );

  // First tab of a distro: its home. Idempotent (StrictMode runs effects twice).
  useEffect(() => {
    if (!name) return;
    const key = `explorer-${nextKey++}`;
    setTabs((t) => (t.some((x) => x.distro === name) ? t : [...t, { key, distro: name, initialPath: null, path: null }]));
  }, [name]);

  // New tab here · close · next / previous (defaults Ctrl+T, Ctrl+W, Ctrl+Tab, Ctrl+Shift+Tab).
  // Not while a dialog (file editor, settings) is open: closing would drop unsaved edits.
  useEffect(() => {
    if (!active || !running || !name) return;
    const onKey = (e: KeyboardEvent) => {
      if (document.querySelector('[aria-modal="true"]')) return;
      const cycle = (step: number) => {
        if (visibleTabs.length < 2) return;
        const idx = visibleTabs.findIndex((t) => t.key === activeKey);
        const next = visibleTabs[(idx + step + visibleTabs.length) % visibleTabs.length];
        setActiveByDistro((a) => ({ ...a, [name]: next.key }));
      };
      if (matchesKey(e, shortcuts["explorer.newTab"])) openTab(name, activeTab?.path ?? null);
      else if (matchesKey(e, shortcuts["explorer.closeTab"])) activeKey && closeTab(activeKey);
      else if (matchesKey(e, shortcuts["explorer.nextTab"])) cycle(1);
      else if (matchesKey(e, shortcuts["explorer.prevTab"])) cycle(-1);
      else return;
      e.preventDefault();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [active, running, name, activeTab, activeKey, visibleTabs, openTab, closeTab, shortcuts]);

  const newTabKey = shortcuts["explorer.newTab"];
  const newTabGestures = formatGestures(shortcuts["explorer.openInNewTab"]);
  const newTabTitle = [
    `Nova aba nesta pasta${newTabKey ? ` (${formatBinding(newTabKey)})` : ""}.`,
    newTabGestures && `${newTabGestures} numa pasta também abre uma aba.`,
    "Atalhos: Configurações → Atalhos.",
  ]
    .filter(Boolean)
    .join(" ");

  if (!distro) {
    return <Centered>Nenhuma distro selecionada.</Centered>;
  }

  return (
    <div className="flex h-full flex-col">
      {running ? (
        <div className="flex items-end gap-1 overflow-x-auto border-b border-zinc-800 bg-void px-2 pt-2">
          {visibleTabs.map((tab) => {
            const isActiveTab = tab.key === activeKey;
            return (
              <div
                key={tab.key}
                onClick={(e) => {
                  if (matchesClick(e, shortcuts["explorer.closeTabClick"])) closeTab(tab.key);
                  else setActiveByDistro((a) => ({ ...a, [tab.distro]: tab.key }));
                }}
                onMouseDown={(e) => e.button === 1 && e.preventDefault()} // no autoscroll cursor
                onAuxClick={(e) => matchesClick(e, shortcuts["explorer.closeTabClick"]) && closeTab(tab.key)}
                title={tab.path ?? "~"}
                className={`group relative -mb-px flex max-w-52 shrink-0 cursor-pointer items-center gap-2 rounded-t-md border border-b-0 px-3 py-1.5 text-xs transition-colors ${
                  isActiveTab
                    ? "border-zinc-800 bg-zinc-950 text-zinc-100"
                    : "border-transparent text-zinc-500 hover:bg-zinc-900 hover:text-zinc-200"
                }`}
              >
                {isActiveTab && <span className="absolute inset-x-0 -top-px h-0.5 bg-zinc-100" />}
                <Folder className="size-3.5 shrink-0" />
                <span className="truncate font-mono text-[11px]">{tabLabel(tab.path)}</span>
                <TabActivity tasks={tasks} distro={tab.distro} path={tab.path} />
                {visibleTabs.length > 1 && (
                  <button
                    type="button"
                    onClick={(e) => {
                      e.stopPropagation();
                      closeTab(tab.key);
                    }}
                    aria-label="Fechar aba"
                    className={`rounded p-0.5 text-zinc-500 hover:bg-zinc-800 hover:text-zinc-100 ${
                      isActiveTab ? "" : "opacity-0 group-hover:opacity-100"
                    }`}
                  >
                    <X className="size-3" />
                  </button>
                )}
              </div>
            );
          })}
          <button
            type="button"
            onClick={() => openTab(distro.name, activeTab?.path ?? null)}
            title={newTabTitle}
            className="mb-1 flex shrink-0 items-center gap-1 rounded-full px-2.5 py-1 text-xs whitespace-nowrap text-zinc-400 transition-colors hover:bg-zinc-900 hover:text-zinc-100"
          >
            <Plus className="size-3.5" />
            Nova aba
          </button>
        </div>
      ) : (
        <Centered>
          <PowerOff className="size-5 text-zinc-600" />
          <p className="text-sm font-medium text-zinc-100">{distro.name} está parada</p>
          <p className="text-xs text-zinc-500">Inicie a distro para navegar pelos arquivos.</p>
          <button type="button" onClick={onStart} disabled={pending !== null} className={`${BTN_PRIMARY} mt-1`}>
            {pending === "start" ? <Loader2 className="size-3.5 animate-spin" /> : <Play className="size-3.5" />}
            Iniciar Distro
          </button>
        </Centered>
      )}

      {/* Every tab stays mounted (hidden), so each keeps its folder, history and filter. */}
      {tabs.map((tab) => (
        <ExplorerPane
          key={tab.key}
          distro={tab.distro}
          running={distros.find((d) => d.name === tab.distro)?.state === "Running"}
          initialPath={tab.initialPath}
          visible={running && tab.key === activeKey}
          onPath={(path) => setTabs((t) => t.map((x) => (x.key === tab.key ? { ...x, path } : x)))}
          newTabClick={shortcuts["explorer.openInNewTab"]}
          onOpenInNewTab={(path) => openTab(tab.distro, path, false)}
          onTerminalHere={onTerminalHere}
          defaultIde={defaultIde}
        />
      ))}
    </div>
  );
}

/** Running project (dot) or install (spinner) inside the folder the tab shows. */
function TabActivity({ tasks, distro, path }: { tasks: TaskInfo[]; distro: string; path: string | null }) {
  if (!path) return null;
  const here = tasks.filter((t) => t.distro === distro && isActive(t) && (t.path === path || parentPath(t.path) === path));
  if (here.some((t) => t.kind === "run")) {
    return <span className="size-1.5 shrink-0 rounded-full bg-emerald-400" title="Projeto rodando nesta pasta" />;
  }
  if (here.length > 0) return <Loader2 className="size-3 shrink-0 animate-spin text-zinc-500" />;
  return null;
}

function Centered({ children }: { children: ReactNode }) {
  return <div className="flex h-full flex-col items-center justify-center gap-3 p-8 text-center">{children}</div>;
}
