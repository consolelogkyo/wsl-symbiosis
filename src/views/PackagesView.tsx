import { useState } from "react";
import { AlertTriangle, Boxes, Loader2, Package, Play, PowerOff, Radio, RefreshCw, TerminalSquare } from "lucide-react";
import type { Distro } from "../lib/wsl";
import type { PendingAction } from "../hooks/useWsl";
import { useEnvironment } from "../hooks/useEnvironment";
import { ToolGrid } from "../components/packages/ToolGrid";
import { NvmCard } from "../components/packages/NvmCard";
import { NpmGlobals } from "../components/packages/NpmGlobals";
import { PortsTable } from "../components/packages/PortsTable";
import { BANNER_ERROR, BTN_GHOST, BTN_PRIMARY } from "../lib/ui";

type Tab = "runtimes" | "npm" | "ports";

interface Props {
  distro: Distro | null;
  pending: PendingAction;
  onStart: () => void;
}

export function PackagesView({ distro, pending, onStart }: Props) {
  const running = distro?.state === "Running";
  const env = useEnvironment(distro?.name ?? null, running);
  const [tab, setTab] = useState<Tab>("runtimes");
  const scan = env.scan;

  if (!distro) {
    return <div className="flex h-full items-center justify-center text-sm text-zinc-500">Nenhuma distro selecionada.</div>;
  }
  if (!running) {
    return (
      <div className="flex h-full flex-col items-center justify-center gap-3 p-8 text-center">
        <PowerOff className="size-5 text-zinc-600" />
        <p className="text-sm font-medium text-zinc-100">{distro.name} está parada</p>
        <p className="text-xs text-zinc-500">Inicie a distro para escanear ferramentas, pacotes e portas.</p>
        <button
          type="button"
          onClick={onStart}
          disabled={pending !== null}
          className={`${BTN_PRIMARY} mt-1`}
        >
          {pending === "start" ? <Loader2 className="size-3.5 animate-spin" /> : <Play className="size-3.5" />}
          Iniciar Distro
        </button>
      </div>
    );
  }

  const tabs: { id: Tab; label: string; icon: typeof Boxes; count?: number }[] = [
    { id: "runtimes", label: "Runtimes & Stack", icon: Boxes, count: scan?.tools.filter((t) => t.installed).length },
    {
      id: "npm",
      label: "Pacotes globais (npm)",
      icon: Package,
      count:
        scan?.npmPrefix !== null && scan
          ? new Set([...scan.npmGlobals.map((p) => p.name), ...(scan.nvmGlobals ?? []).map((p) => p.name)]).size
          : undefined,
    },
    { id: "ports", label: "Portas em uso", icon: Radio, count: scan?.listeningPorts.length },
  ];

  return (
    <div className="flex h-full flex-col">
      <div className="@container flex items-center gap-1 border-b border-zinc-800 bg-zinc-950 px-4 pt-3">
        {tabs.map(({ id, label, icon: Icon, count }) => {
          const active = tab === id;
          return (
            <button
              key={id}
              type="button"
              onClick={() => setTab(id)}
              className={`relative flex items-center gap-2 rounded-t-md px-3 pt-1.5 pb-2.5 text-sm whitespace-nowrap transition-colors ${
                active ? "font-medium text-zinc-100" : "text-zinc-500 hover:text-zinc-200"
              }`}
            >
              <Icon className="size-4" />
              {label}
              {count !== undefined && (
                <span className={`rounded-full px-1.5 font-mono text-[10px] ${active ? "bg-zinc-100 text-void" : "bg-zinc-800 text-zinc-400"}`}>{count}</span>
              )}
              {active && <span className="absolute inset-x-2 -bottom-px h-0.5 bg-zinc-100" />}
            </button>
          );
        })}

        <div className="ml-auto flex items-center gap-3 pb-2">
          {scan && (
            <span className="flex items-center gap-1.5 font-mono text-[11px] whitespace-nowrap text-zinc-500 @max-4xl:hidden" title="Shell usado na sonda (carrega seu ~/.zshrc / ~/.bashrc)">
              <TerminalSquare className="size-3.5" />
              {scan.shell} · {(scan.durationMs / 1000).toFixed(1)}s
            </span>
          )}
          <button
            type="button"
            onClick={env.refresh}
            disabled={env.scanning}
            className={`${BTN_GHOST} whitespace-nowrap`}
          >
            {env.scanning ? <Loader2 className="size-3.5 animate-spin" /> : <RefreshCw className="size-3.5" />}
            Atualizar escaneamento
          </button>
        </div>
      </div>

      {env.error && (
        <div className={BANNER_ERROR}>
          <AlertTriangle className="mt-px size-3.5 shrink-0" />
          <span className="font-mono break-all whitespace-pre-wrap">{env.error}</span>
        </div>
      )}

      <div className="min-h-0 flex-1 overflow-auto p-6">
        {!scan ? (
          <div className="flex h-40 flex-col items-center justify-center gap-2 text-xs text-zinc-500">
            <Loader2 className="size-5 animate-spin text-zinc-600" />
            Escaneando {distro.name} pelo shell do usuário…
          </div>
        ) : tab === "runtimes" ? (
          <div className="space-y-5">
            {scan.nvm && scan.nvm.versions.length > 0 && (
              <NvmCard distro={distro.name} nvm={scan.nvm} onChanged={env.refresh} />
            )}
            <ToolGrid tools={scan.tools} />
          </div>
        ) : tab === "npm" ? (
          <NpmGlobals
            distro={distro.name}
            prefix={scan.npmPrefix}
            packages={scan.npmGlobals}
            nvmGlobals={scan.nvmGlobals ?? []}
            activeNode={scan.nvm?.versions.find((v) => v.active)?.version ?? null}
            onChanged={env.refresh}
          />
        ) : (
          <PortsTable distro={distro.name} ports={scan.listeningPorts} error={scan.portsError} onPorts={env.setPorts} />
        )}
      </div>
    </div>
  );
}
