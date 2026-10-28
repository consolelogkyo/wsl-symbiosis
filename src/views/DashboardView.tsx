import { useEffect, useState, type ReactNode } from "react";
import {
  Activity,
  CheckCircle2,
  Cpu,
  Eraser,
  HardDrive,
  Layers,
  Loader2,
  MemoryStick,
  Monitor,
  Play,
  PowerOff,
  Star,
  TriangleAlert,
} from "lucide-react";
import type { Distro, DropCachesResult, GuestMetrics, HostMetrics, WslInfo, WslResourceMetrics } from "../lib/wsl";
import { errorMessage, freeCachedMemory } from "../lib/wsl";
import { LEVEL_BAR, LEVEL_DOT, LEVEL_TEXT, formatBytes, level, percent } from "../lib/format";
import { BTN_GHOST, BTN_PRIMARY, CARD, LABEL } from "../lib/ui";
import type { PendingAction } from "../hooks/useWsl";
import { StatusDot } from "../components/StatusDot";
import { MeterCard } from "../components/dashboard/MeterCard";
import { ProcessTable } from "../components/dashboard/ProcessTable";
import { InfoTip } from "../components/InfoTip";

interface Props {
  distros: Distro[];
  selected: Distro | null;
  info: WslInfo | null;
  pending: PendingAction;
  /** Polled in App (the sidebar's live panel shares it). */
  telemetry: { metrics: WslResourceMetrics | null; error: string | null; refresh: () => Promise<void> };
  onSelect: (name: string) => void;
  onStart: () => void;
}

function Stat({ label, value, icon: Icon }: { label: string; value: string; icon: typeof Cpu }) {
  return (
    <div className={`${CARD} px-4 py-3.5`}>
      <div className="flex items-center gap-2">
        <Icon className="size-3.5 text-zinc-600" />
        <span className={LABEL}>{label}</span>
      </div>
      <div className="mt-2 truncate font-mono text-lg text-zinc-100">{value}</div>
    </div>
  );
}

function SectionTitle({ children, live, info }: { children: string; live?: boolean; info?: ReactNode }) {
  return (
    <div className="mb-3 flex items-center gap-2.5">
      <h2 className={LABEL}>{children}</h2>
      {info && <InfoTip>{info}</InfoTip>}
      {live && (
        <span className="flex items-center gap-1.5 font-mono text-[10px] tracking-wider text-zinc-600 uppercase">
          <span className="size-1.5 animate-pulse rounded-full bg-emerald-400" />
          ao vivo · 3s
        </span>
      )}
      <span className="h-px flex-1 bg-zinc-800" />
    </div>
  );
}

function HostMeter({ host }: { host: HostMetrics }) {
  const vmMemory = host.vm?.memory ?? 0;
  const helpers = host.wslHelpers;
  // Same number Task Manager adds up: the VM plus WSL's own Windows processes.
  const wslMemory = vmMemory + helpers.memory;
  const hostPct = percent(host.usedMemory, host.totalMemory);
  const wslPct = percent(wslMemory, host.totalMemory);
  return (
    <MeterCard
      icon={Monitor}
      title="Peso do WSL no Windows"
      value={wslMemory > 0 ? formatBytes(wslMemory) : "—"}
      dot={host.vm ? LEVEL_DOT[level(hostPct)] : undefined}
      caption={host.vm ? `${wslPct.toFixed(0)}% da RAM do host` : "VM do WSL desligada"}
      total={host.totalMemory}
      segments={[
        { label: `VM ${host.vm?.name ?? "vmmemWSL"} ${formatBytes(vmMemory)}`, value: vmMemory, className: LEVEL_BAR[level(hostPct)] },
        {
          label: `Serviços WSL ${formatBytes(helpers.memory)}`,
          value: helpers.memory,
          className: "bg-zinc-400",
        },
        {
          label: "Outros apps",
          value: Math.max(0, host.usedMemory - wslMemory),
          className: "bg-zinc-600",
        },
      ]}
      footer={
        <span>
          Host {formatBytes(host.usedMemory)} / {formatBytes(host.totalMemory)}
          <span className={`ml-1.5 ${LEVEL_TEXT[level(hostPct)]}`}>({hostPct.toFixed(0)}%)</span>
          {host.vm && <span className="text-zinc-500"> · CPU da VM {host.vm.cpuUsage.toFixed(1)}%</span>}
          <span className="text-zinc-600"> · {helpers.processes} processos WSL</span>
        </span>
      }
    />
  );
}

function GuestMeters({ distro, guest, onChanged }: { distro: string; guest: GuestMetrics; onChanged: () => void }) {
  const [dropping, setDropping] = useState(false);
  const [dropResult, setDropResult] = useState<DropCachesResult | null>(null);
  const [dropError, setDropError] = useState<string | null>(null);

  useEffect(() => {
    setDropResult(null);
    setDropError(null);
  }, [distro]);

  const dropCaches = async () => {
    setDropping(true);
    setDropError(null);
    try {
      setDropResult(await freeCachedMemory(distro));
      onChanged();
    } catch (err) {
      setDropError(errorMessage(err));
    } finally {
      setDropping(false);
    }
  };

  const { memory, disk, load } = guest;
  const usedPct = percent(memory.used, memory.total);
  const diskPct = disk ? percent(disk.used, disk.total) : 0;
  const cpu = guest.cpuUsage ?? 0;
  const loadPct = percent(load.one, guest.cpuCount);

  return (
    <>
      <MeterCard
        icon={MemoryStick}
        title="Memória da distro"
        value={formatBytes(memory.used)}
        valueClassName={LEVEL_TEXT[level(usedPct)]}
        dot={LEVEL_DOT[level(usedPct)]}
        caption={`de ${formatBytes(memory.total)} · ${formatBytes(memory.available)} disponível`}
        total={memory.total}
        segments={[
          { label: `Uso real ${formatBytes(memory.used)}`, value: memory.used, className: LEVEL_BAR[level(usedPct)] },
          { label: `Cache ${formatBytes(memory.cache)}`, value: memory.cache, className: "bg-zinc-500" },
        ]}
        action={
          <button
            type="button"
            onClick={dropCaches}
            disabled={dropping}
            title="sync; echo 3 > /proc/sys/vm/drop_caches (vale para a VM inteira)"
            className={`${BTN_GHOST} -my-1 px-2 py-0.5 text-[11px]`}
          >
            {dropping ? <Loader2 className="size-3 animate-spin" /> : <Eraser className="size-3" />}
            Liberar cache
          </button>
        }
        footer={
          dropError ? (
            <span className="text-rose-300">{dropError}</span>
          ) : dropResult ? (
            <span className="flex items-center gap-1.5 text-zinc-100">
              <CheckCircle2 className="size-3.5 shrink-0 text-emerald-400" />
              Cache {formatBytes(dropResult.cacheBefore)} → {formatBytes(dropResult.cacheAfter)}
              <span className="text-zinc-500">
                · liberou {formatBytes(Math.max(0, dropResult.cacheBefore - dropResult.cacheAfter))}
              </span>
            </span>
          ) : (
            <span className="text-zinc-500">Cache é recuperável: o kernel o devolve sob demanda.</span>
          )
        }
      />

      <MeterCard
        icon={HardDrive}
        title="Disco virtual (VHDX) em /"
        value={disk ? formatBytes(disk.used) : "—"}
        valueClassName={LEVEL_TEXT[level(diskPct)]}
        dot={disk ? LEVEL_DOT[level(diskPct)] : undefined}
        caption={disk ? `de ${formatBytes(disk.total)} · ${diskPct.toFixed(0)}%` : "df indisponível"}
        total={disk?.total ?? 0}
        legend={false}
        segments={disk ? [{ label: "Usado", value: disk.used, className: LEVEL_BAR[level(diskPct)] }] : []}
        footer={
          disk && (
            <span className="text-zinc-500">
              {formatBytes(disk.available)} livres · <span className="font-mono">{disk.filesystem}</span>
            </span>
          )
        }
      />

      <MeterCard
        icon={Activity}
        title="Carga do sistema"
        value={guest.cpuUsage === null ? "…" : `${cpu.toFixed(1)}%`}
        valueClassName={LEVEL_TEXT[level(Math.max(cpu, loadPct))]}
        dot={LEVEL_DOT[level(Math.max(cpu, loadPct))]}
        caption={`CPU · ${guest.cpuCount} vCPUs`}
        total={100}
        legend={false}
        segments={[{ label: "CPU", value: cpu, className: LEVEL_BAR[level(cpu)] }]}
        footer={
          <span className="flex items-center gap-2">
            <span className="text-zinc-500">Load</span>
            <span className="font-mono text-zinc-200 tabular-nums">
              {load.one.toFixed(2)} · {load.five.toFixed(2)} · {load.fifteen.toFixed(2)}
            </span>
            <span className="text-zinc-600">
              {load.running}/{load.total} tarefas
            </span>
          </span>
        }
      />
    </>
  );
}

function GuestPlaceholder({ selected, pending, onStart, error }: {
  selected: Distro;
  pending: PendingAction;
  onStart: () => void;
  error: string | null;
}) {
  const running = selected.state === "Running";
  return (
    <div className="col-span-1 flex flex-col items-center justify-center gap-3 rounded-lg border border-dashed border-zinc-800 p-6 text-center 2xl:col-span-3">
      {error ? (
        <>
          <TriangleAlert className="size-5 text-amber-400" />
          <p className="max-w-md text-sm text-zinc-400">Não foi possível ler as métricas de {selected.name}.</p>
          <p className="max-w-md font-mono text-xs break-all text-zinc-500">{error}</p>
        </>
      ) : running ? (
        <Loader2 className="size-5 animate-spin text-zinc-600" />
      ) : (
        <>
          <PowerOff className="size-5 text-zinc-600" />
          <div>
            <p className="text-sm font-medium text-zinc-100">{selected.name} está parada</p>
            <p className="mt-1 text-xs text-zinc-500">Memória, disco, carga e processos aparecem com a distro em execução.</p>
          </div>
          <button type="button" onClick={onStart} disabled={pending !== null} className={`${BTN_PRIMARY} mt-1`}>
            {pending === "start" ? <Loader2 className="size-3.5 animate-spin" /> : <Play className="size-3.5" />}
            Iniciar Distro para monitorar processos
          </button>
        </>
      )}
    </div>
  );
}

export function DashboardView({ distros, selected, info, pending, telemetry, onSelect, onStart }: Props) {
  const running = distros.filter((d) => d.state === "Running").length;
  const { metrics, error, refresh } = telemetry;
  const guest = metrics?.guest ?? null;

  return (
    <div className="space-y-7 p-6">
      <div className="grid grid-cols-2 gap-3 xl:grid-cols-4">
        <Stat icon={Layers} label="Distros instaladas" value={String(distros.length)} />
        <Stat icon={Cpu} label="Em execução" value={`${running} / ${distros.length}`} />
        <Stat icon={HardDrive} label="Versão do WSL" value={info?.wslVersion ?? "—"} />
        <Stat icon={HardDrive} label="Kernel" value={info?.kernelVersion ?? "—"} />
      </div>

      {selected && (
        <section>
          <SectionTitle
            live={metrics !== null}
            info={
              <>
                Memória, CPU e load vêm do kernel da VM do WSL2, compartilhado por todas as distros; disco e
                processos são de <span className="font-medium text-zinc-100">{selected.name}</span>.
              </>
            }
          >
            Recursos
          </SectionTitle>
          {error && !metrics && (
            <div className="mb-3 rounded-lg border border-rose-500/20 px-3 py-2 font-mono text-xs text-rose-300">{error}</div>
          )}
          <div className="grid grid-cols-1 gap-3 md:grid-cols-2 2xl:grid-cols-4">
            {metrics ? (
              <HostMeter host={metrics.host} />
            ) : (
              <div className={`${CARD} h-40 animate-pulse`} />
            )}
            {guest ? (
              <GuestMeters distro={selected.name} guest={guest} onChanged={refresh} />
            ) : (
              <GuestPlaceholder
                selected={selected}
                pending={pending}
                onStart={onStart}
                error={metrics?.guestError ?? null}
              />
            )}
          </div>
        </section>
      )}

      {selected && guest && (
        <ProcessTable distro={selected.name} processes={guest.processes} onKilled={refresh} />
      )}

      <section className="overflow-hidden rounded-lg border border-zinc-800 bg-zinc-900">
        <div className={`border-b border-zinc-800 px-4 py-2.5 ${LABEL}`}>Distribuições</div>
        <table className="w-full text-sm">
          <thead>
            <tr className="border-b border-zinc-800 text-left text-[10px] tracking-wider text-zinc-600 uppercase">
              <th className="px-4 py-2 font-medium">Nome</th>
              <th className="px-4 py-2 font-medium">Estado</th>
              <th className="px-4 py-2 font-medium">Versão</th>
              <th className="px-4 py-2 font-medium">Padrão</th>
            </tr>
          </thead>
          <tbody>
            {distros.map((d) => {
              const active = d.name === selected?.name;
              return (
                <tr
                  key={d.name}
                  onClick={() => onSelect(d.name)}
                  className={`relative cursor-pointer border-b border-zinc-800/70 transition-colors last:border-0 ${
                    active ? "bg-zinc-800/50" : "hover:bg-zinc-800/30"
                  }`}
                >
                  <td className="px-4 py-2.5 font-medium text-zinc-100">
                    <span className="flex items-center gap-2.5">
                      <span className={`h-3.5 w-0.5 ${active ? "bg-zinc-100" : "bg-transparent"}`} />
                      {d.name}
                    </span>
                  </td>
                  <td className="px-4 py-2.5">
                    <span className="flex items-center gap-2 text-zinc-400">
                      <StatusDot state={d.state} />
                      {d.stateLabel}
                    </span>
                  </td>
                  <td className="px-4 py-2.5 font-mono text-xs text-zinc-400">WSL {d.version}</td>
                  <td className="px-4 py-2.5">
                    {d.isDefault && <Star className="size-3.5 fill-zinc-300 text-zinc-300" />}
                  </td>
                </tr>
              );
            })}
            {distros.length === 0 && (
              <tr>
                <td colSpan={4} className="px-4 py-8 text-center text-zinc-500">
                  Nenhuma distro encontrada.
                </td>
              </tr>
            )}
          </tbody>
        </table>
      </section>
    </div>
  );
}
