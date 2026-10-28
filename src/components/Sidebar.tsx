import { useEffect, useState } from "react";
import { getVersion } from "@tauri-apps/api/app";
import { Settings2 } from "lucide-react";
import type { Distro, WslResourceMetrics } from "../lib/wsl";
import { LEVEL_DOT, formatBytes, level, percent } from "../lib/format";
import { NAV_ITEMS, type NavItem, type ViewId } from "../views";
import { RunningPanel } from "./RunningPanel";

interface Props {
  view: ViewId;
  onNavigate: (view: ViewId) => void;
  selected: Distro | null;
  metrics: WslResourceMetrics | null;
  onOpenSettings: () => void;
}

export function Sidebar({ view, onNavigate, selected, metrics, onOpenSettings }: Props) {
  const [version, setVersion] = useState<string | null>(null);
  useEffect(() => {
    getVersion().then(setVersion).catch(() => setVersion(null));
  }, []);

  return (
    <aside className="flex w-56 shrink-0 flex-col border-r border-[#18181e] bg-void">
      <nav className="min-h-0 flex-1 space-y-px overflow-y-auto px-2 pt-4">
        <div className="mb-2 px-2.5 text-[11px] font-medium tracking-wider text-zinc-600 uppercase">Módulos</div>
        {NAV_ITEMS.map((item) => (
          <NavButton key={item.id} item={item} active={item.id === view} onNavigate={onNavigate} />
        ))}
      </nav>

      <RunningPanel distro={selected?.name ?? null} />
      <LivePanel selected={selected} metrics={metrics} />

      <div className="flex items-center border-t border-[#18181e] px-4 py-2.5">
        <span className="font-mono text-[10px] tracking-wider text-zinc-600">{version && `v${version}`}</span>
        <button
          type="button"
          onClick={onOpenSettings}
          title="Diagnóstico & Configurações"
          className="ml-auto rounded p-1 text-zinc-600 transition-colors hover:bg-zinc-900 hover:text-zinc-200"
        >
          <Settings2 className="size-3.5" />
        </button>
      </div>
    </aside>
  );
}

function NavButton({ item, active, onNavigate }: { item: NavItem; active: boolean; onNavigate: (view: ViewId) => void }) {
  const { id, label, icon: Icon } = item;
  return (
    <button
      type="button"
      onClick={() => onNavigate(id)}
      aria-current={active ? "page" : undefined}
      className={`group relative flex w-full items-center gap-2.5 rounded-md px-2.5 py-2 text-[13px] transition-colors ${
        active ? "bg-zinc-900 font-medium text-zinc-100" : "text-zinc-500 hover:bg-zinc-900/60 hover:text-zinc-200"
      }`}
    >
      {active && <span className="absolute inset-y-1 -left-2 w-0.5 bg-zinc-100" />}
      <Icon className={`size-4 ${active ? "text-zinc-100" : "text-zinc-600 group-hover:text-zinc-300"}`} />
      <span className="truncate">{label}</span>
    </button>
  );
}

/** Always-on hardware-monitor style summary (VM + selected distro). */
function LivePanel({ selected, metrics }: { selected: Distro | null; metrics: WslResourceMetrics | null }) {
  const running = selected?.state === "Running";
  const host = metrics?.host ?? null;
  const guest = metrics?.guest ?? null;
  const vmMem = host?.vm?.memory ?? null;
  const ramPct = guest ? percent(guest.memory.used, guest.memory.total) : null;
  const cpu = guest?.cpuUsage ?? null;

  return (
    <section className="mx-2 mb-2 rounded-lg border border-zinc-800 bg-zinc-950 px-3 py-2.5">
      <div className="mb-2 flex items-center gap-1.5">
        <span className={`size-1.5 rounded-full ${running ? "animate-pulse bg-emerald-400" : "bg-zinc-600"}`} />
        <span className="text-[10px] font-semibold tracking-[0.2em] text-zinc-400 uppercase">Ao vivo</span>
        <span className="ml-auto truncate font-mono text-[10px] text-zinc-600">{selected?.name ?? "—"}</span>
      </div>
      <dl className="space-y-1.5">
        <Row label="VM RAM (vmmem)" value={vmMem !== null ? formatBytes(vmMem) : "—"} />
        <Row
          label="RAM Distro"
          value={guest ? formatBytes(guest.memory.used) : "—"}
          dot={ramPct !== null ? LEVEL_DOT[level(ramPct)] : undefined}
        />
        <Row
          label="CPU / Load"
          value={guest ? `${cpu === null ? "…" : `${cpu.toFixed(0)}%`} · ${guest.load.one.toFixed(2)}` : "—"}
          dot={cpu !== null ? LEVEL_DOT[level(cpu)] : undefined}
        />
        <Row label="Estado" value={selected ? (running ? "ativo" : "parado") : "—"} strong={running} />
      </dl>
    </section>
  );
}

function Row({ label, value, dot, strong }: { label: string; value: string; dot?: string; strong?: boolean }) {
  return (
    <div className="flex items-center gap-2">
      <dt className="text-[10px] tracking-wider whitespace-nowrap text-zinc-500 uppercase">{label}</dt>
      <dd
        className={`ml-auto flex items-center gap-1.5 font-mono text-[11px] whitespace-nowrap tabular-nums ${
          strong === false ? "text-zinc-500" : "text-zinc-100"
        }`}
      >
        {dot && <span className={`size-1 rounded-full ${dot}`} />}
        {value}
      </dd>
    </div>
  );
}
