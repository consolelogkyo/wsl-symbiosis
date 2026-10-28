import { useCallback, useEffect, useRef, useState } from "react";
import {
  Activity,
  AlertTriangle,
  Check,
  Copy,
  Eraser,
  GitBranch,
  HardDrive,
  Loader2,
  Network,
  Plus,
  RefreshCw,
  RotateCcw,
  SquareTerminal,
  X,
  type LucideIcon,
} from "lucide-react";
import type { Distro } from "../lib/wsl";
import { SNIPPETS } from "../lib/terminal";
import { shortenHome } from "../lib/packages";
import { PaneApi, TabStatus, TerminalPane } from "../components/terminal/TerminalPane";
import { BTN_PRIMARY } from "../lib/ui";

export interface TerminalRequest {
  distro: string;
  path: string;
  /** Changes on every "Terminal aqui" click, even for the same path. */
  nonce: number;
}

interface Tab {
  key: string;
  distro: string;
  cwd: string | null;
  generation: number;
  status: TabStatus;
  exitCode?: number | null;
  error?: string;
  /** The user typed something: "Terminal aqui" opens a new tab instead of reusing it. */
  touched: boolean;
  title?: string;
}

interface Props {
  distro: Distro | null;
  request: TerminalRequest | null;
  active: boolean;
}

const SNIPPET_ICONS: Record<string, LucideIcon> = {
  apt: RefreshCw,
  git: GitBranch,
  top: Activity,
  df: HardDrive,
  ports: Network,
  reload: RotateCcw,
};

let nextKey = 1;

function tabLabel(tab: Tab): string {
  if (!tab.cwd) return "~";
  const short = shortenHome(tab.cwd);
  const parts = short.split("/").filter(Boolean);
  return parts.length > 2 ? `…/${parts.slice(-2).join("/")}` : short;
}

export function TerminalView({ distro, request, active }: Props) {
  const [tabs, setTabs] = useState<Tab[]>([]);
  const [activeKey, setActiveKey] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  const apis = useRef(new Map<string, PaneApi>());
  const handledNonce = useRef<number | null>(null);

  const openTab = useCallback((tabDistro: string, cwd: string | null) => {
    const key = `tab-${nextKey++}`;
    setTabs((t) => [...t, { key, distro: tabDistro, cwd, generation: 0, status: "connecting", touched: false }]);
    setActiveKey(key);
  }, []);

  const patchTab = useCallback((key: string, patch: Partial<Tab>) => {
    setTabs((t) => t.map((tab) => (tab.key === key ? { ...tab, ...patch } : tab)));
  }, []);

  const restart = useCallback((key: string) => {
    setTabs((t) => t.map((tab) => (tab.key === key ? { ...tab, generation: tab.generation + 1 } : tab)));
  }, []);

  const closeTab = (key: string) => {
    setTabs((t) => {
      const idx = t.findIndex((tab) => tab.key === key);
      const rest = t.filter((tab) => tab.key !== key);
      if (key === activeKey) setActiveKey(rest[Math.min(idx, rest.length - 1)]?.key ?? null);
      return rest;
    });
  };

  // "Terminal aqui": reuse an untouched tab already at that folder, else open a new one.
  useEffect(() => {
    if (!request || handledNonce.current === request.nonce) return;
    handledNonce.current = request.nonce;
    const reusable = tabs.find(
      (t) => t.distro === request.distro && t.cwd === request.path && !t.touched && t.status !== "exited" && t.status !== "error",
    );
    if (reusable) setActiveKey(reusable.key);
    else openTab(request.distro, request.path);
  }, [request, tabs, openTab]);

  // First visit: open a shell in the selected distro's home (once, even under StrictMode's
  // double effect run).
  const autoOpened = useRef(false);
  useEffect(() => {
    if (autoOpened.current || !active || tabs.length > 0 || !distro || request) return;
    autoOpened.current = true;
    openTab(distro.name, null);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [active]);

  const activeTab = tabs.find((t) => t.key === activeKey) ?? null;
  const api = activeKey ? apis.current.get(activeKey) : undefined;

  return (
    <div className="flex h-full flex-col">
      {/* Tabs */}
      <div className="flex items-center gap-1 border-b border-zinc-800 bg-void px-2 pt-2">
        <div className="flex min-w-0 flex-1 items-end gap-1 overflow-x-auto">
          {tabs.map((tab) => {
            const isActive = tab.key === activeKey;
            return (
              <div
                key={tab.key}
                onClick={() => setActiveKey(tab.key)}
                title={tab.title ?? `${tab.distro}:${tab.cwd ?? "~"}`}
                className={`group relative -mb-px flex max-w-60 shrink-0 cursor-pointer items-center gap-2 rounded-t-md border border-b-0 px-3 py-1.5 text-xs transition-colors ${
                  isActive
                    ? "border-zinc-800 bg-zinc-950 text-zinc-100"
                    : "border-transparent text-zinc-500 hover:bg-zinc-900 hover:text-zinc-200"
                }`}
              >
                {isActive && <span className="absolute inset-x-0 -top-px h-0.5 bg-zinc-100" />}
                <TabIndicator status={tab.status} />
                <span className="font-semibold">{tab.distro}</span>
                <span className="truncate font-mono text-[11px] text-zinc-500">{tabLabel(tab)}</span>
                <button
                  type="button"
                  onClick={(e) => {
                    e.stopPropagation();
                    closeTab(tab.key);
                  }}
                  aria-label="Fechar aba"
                  className={`rounded p-0.5 text-zinc-500 hover:bg-zinc-800 hover:text-zinc-100 ${
                    isActive ? "" : "opacity-0 group-hover:opacity-100"
                  }`}
                >
                  <X className="size-3" />
                </button>
              </div>
            );
          })}
          <button
            type="button"
            onClick={() => distro && openTab(distro.name, null)}
            disabled={!distro}
            title={distro ? `Nova aba em ${distro.name}` : "Selecione uma distro"}
            className="mb-1 flex shrink-0 items-center gap-1 rounded-full px-2.5 py-1 text-xs text-zinc-400 transition-colors hover:bg-zinc-900 hover:text-zinc-100 disabled:opacity-40"
          >
            <Plus className="size-3.5" />
            Nova aba
          </button>
        </div>

        <div className="mb-1 flex shrink-0 items-center gap-1">
          <ToolbarButton
            icon={copied ? Check : Copy}
            label="Copiar seleção"
            disabled={!api}
            onClick={async () => {
              if (await api?.copySelection()) {
                setCopied(true);
                window.setTimeout(() => setCopied(false), 1200);
              }
            }}
          />
          <ToolbarButton icon={Eraser} label="Limpar" disabled={!api} onClick={() => api?.clear()} />
          <ToolbarButton
            icon={RotateCcw}
            label="Reiniciar sessão"
            disabled={!activeTab}
            onClick={() => activeTab && restart(activeTab.key)}
          />
        </div>
      </div>

      {/* Quick snippets */}
      <div className="flex items-center gap-1.5 overflow-x-auto border-b border-zinc-800 bg-zinc-950 px-3 py-1.5">
        <span className="mr-1.5 shrink-0 text-[10px] font-medium tracking-wider text-zinc-600 uppercase">Comandos rápidos</span>
        {SNIPPETS.map((s) => {
          const Icon = SNIPPET_ICONS[s.id] ?? SquareTerminal;
          return (
            <button
              key={s.id}
              type="button"
              title={s.command}
              disabled={activeTab?.status !== "running"}
              onClick={() => api?.send(`${s.command}\r`)}
              className="flex shrink-0 items-center gap-1.5 rounded-full border border-zinc-800 px-2.5 py-1 text-[11px] text-zinc-300 transition-colors hover:border-zinc-100 hover:bg-zinc-100 hover:text-void disabled:opacity-40 disabled:hover:border-zinc-800 disabled:hover:bg-transparent disabled:hover:text-zinc-300"
            >
              <Icon className="size-3" />
              {s.label}
            </button>
          );
        })}
      </div>

      {activeTab?.status === "error" && (
        <div className="flex items-center gap-2 border-b border-rose-500/20 px-4 py-1.5 font-mono text-xs text-rose-300">
          <AlertTriangle className="size-3.5 shrink-0" />
          {activeTab.error}
        </div>
      )}

      {/* Panes: all stay mounted so switching tabs never loses scrollback. */}
      <div className="relative min-h-0 flex-1 bg-zinc-950 p-2">
        {tabs.map((tab) => (
          <TerminalPane
            key={tab.key}
            distro={tab.distro}
            cwd={tab.cwd}
            generation={tab.generation}
            visible={active && tab.key === activeKey}
            onStatus={(patch) => patchTab(tab.key, patch)}
            onRestart={() => restart(tab.key)}
            registerApi={(a) => (a ? apis.current.set(tab.key, a) : apis.current.delete(tab.key))}
          />
        ))}
        {tabs.length === 0 && (
          <div className="flex h-full flex-col items-center justify-center gap-3 text-center">
            <SquareTerminal className="size-6 text-zinc-600" />
            <p className="text-sm text-zinc-400">Nenhum terminal aberto</p>
            <button
              type="button"
              onClick={() => distro && openTab(distro.name, null)}
              disabled={!distro}
              className={BTN_PRIMARY}
            >
              <Plus className="size-3.5" />
              Abrir terminal {distro ? `em ${distro.name}` : ""}
            </button>
          </div>
        )}
      </div>
    </div>
  );
}

function TabIndicator({ status }: { status: TabStatus }) {
  if (status === "connecting") return <Loader2 className="size-3 shrink-0 animate-spin text-zinc-500" />;
  const color = status === "running" ? "bg-emerald-400" : status === "error" ? "bg-rose-400" : "bg-zinc-600";
  return <span className={`size-1.5 shrink-0 rounded-full ${color}`} />;
}

function ToolbarButton({
  icon: Icon,
  label,
  onClick,
  disabled,
}: {
  icon: LucideIcon;
  label: string;
  onClick: () => void;
  disabled?: boolean;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={disabled}
      title={label}
      className="flex items-center gap-1.5 rounded-full px-2.5 py-1 text-xs text-zinc-400 transition-colors hover:bg-zinc-900 hover:text-zinc-100 disabled:opacity-40"
    >
      <Icon className="size-3.5" />
      <span className="hidden xl:inline">{label}</span>
    </button>
  );
}
