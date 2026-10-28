import { useEffect, useState } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { Globe, Loader2, RefreshCw, Skull, X } from "lucide-react";
import { ListeningPort, listListeningPorts } from "../../lib/packages";
import { KillSignal, errorMessage, killLinuxProcess } from "../../lib/wsl";
import { BTN_DANGER, BTN_DANGER_SOLID, BTN_GHOST, LABEL } from "../../lib/ui";

const POLL_INTERVAL_MS = 5000;

interface Props {
  distro: string;
  ports: ListeningPort[];
  error: string | null;
  onPorts: (ports: ListeningPort[]) => void;
}

function scopeOf(address: string): { label: string; className: string } {
  if (address === "0.0.0.0" || address === "::" || address === "*") {
    return { label: "todas as interfaces", className: "text-zinc-300" };
  }
  if (address.startsWith("127.") || address === "::1") return { label: "somente local", className: "text-zinc-500" };
  return { label: "interface específica", className: "text-zinc-500" };
}

export function PortsTable({ distro, ports, error, onPorts }: Props) {
  const [refreshing, setRefreshing] = useState(false);
  const [confirming, setConfirming] = useState<string | null>(null);
  const [killing, setKilling] = useState<string | null>(null);
  const [message, setMessage] = useState<string | null>(error);

  const refresh = async () => {
    setRefreshing(true);
    try {
      onPorts(await listListeningPorts(distro));
      setMessage(null);
    } catch (err) {
      setMessage(errorMessage(err));
    } finally {
      setRefreshing(false);
    }
  };

  // Ports come and go as dev servers start/stop: keep the list live while this tab is open.
  useEffect(() => {
    const id = window.setInterval(() => {
      listListeningPorts(distro).then(onPorts).catch(() => {});
    }, POLL_INTERVAL_MS);
    return () => window.clearInterval(id);
  }, [distro, onPorts]);

  const kill = async (key: string, pid: number, signal: KillSignal) => {
    setKilling(key);
    try {
      await killLinuxProcess(distro, pid, signal);
      setMessage(null);
      await refresh();
    } catch (err) {
      setMessage(`PID ${pid}: ${errorMessage(err)}`);
    } finally {
      setKilling(null);
      setConfirming(null);
    }
  };

  return (
    <section className="overflow-hidden rounded-lg border border-zinc-800 bg-zinc-900">
      <div className="flex items-center gap-3 border-b border-zinc-800 px-4 py-2">
        <span className="size-1.5 animate-pulse rounded-full bg-emerald-400" />
        <span className={LABEL}>Portas TCP em escuta</span>
        <span className="text-[11px] text-zinc-600">{ports.length} · atualiza a cada 5s</span>
        <button
          type="button"
          onClick={refresh}
          className="ml-auto rounded p-1 text-zinc-500 hover:bg-zinc-800 hover:text-zinc-200"
          title="Atualizar portas"
        >
          {refreshing ? <Loader2 className="size-3.5 animate-spin" /> : <RefreshCw className="size-3.5" />}
        </button>
      </div>

      {message && (
        <div className="flex items-center gap-2 border-b border-rose-500/20 px-4 py-2 font-mono text-xs text-rose-300">
          <span className="flex-1">{message}</span>
          <button type="button" onClick={() => setMessage(null)} className="text-rose-400/70 hover:text-rose-200">
            <X className="size-3.5" />
          </button>
        </div>
      )}

      {ports.map((p) => {
        const key = `${p.localAddress}:${p.port}`;
        const scope = scopeOf(p.localAddress);
        return (
          <div key={key} className="group flex items-center gap-4 border-b border-zinc-800/50 px-4 py-2.5 last:border-0 hover:bg-zinc-800/30">
            <span className="w-24 shrink-0 font-mono text-xl font-medium tracking-tight text-zinc-100 tabular-nums">:{p.port}</span>
            <div className="w-44 shrink-0">
              <div className="font-mono text-xs text-zinc-300">{p.localAddress}</div>
              <div className={`text-[11px] ${scope.className}`}>{scope.label}</div>
            </div>
            <div className="min-w-0 flex-1">
              <div className="flex min-w-0 items-baseline gap-2">
                {p.cwd && p.cwd !== "/" ? (
                  <span className="truncate text-sm font-semibold text-zinc-100" title={p.cwd}>
                    {p.cwd.split("/").filter(Boolean).pop()}
                  </span>
                ) : null}
                {p.process ? (
                  <span className="truncate font-mono text-xs text-zinc-400">{p.process}</span>
                ) : (
                  <span className="text-xs text-zinc-600">processo desconhecido</span>
                )}
              </div>
              <div className="truncate font-mono text-[11px] text-zinc-500" title={p.command ?? undefined}>
                {p.pid !== null
                  ? `PID ${p.pid}${p.user ? ` · ${p.user}` : ""}${p.cwd && p.cwd !== "/" ? ` · ${p.cwd}` : ""}`
                  : "sem PID: outra distro ou o próprio WSL"}
              </div>
            </div>

            <button
              type="button"
              onClick={() => openUrl(`http://localhost:${p.port}`).catch((err) => setMessage(errorMessage(err)))}
              title={`Abrir http://localhost:${p.port} no navegador`}
              className={BTN_GHOST}
            >
              <Globe className="size-3.5" />
              Abrir
            </button>

            <div className="flex w-44 shrink-0 justify-end">
              {p.pid === null ? null : killing === key ? (
                <Loader2 className="size-3.5 animate-spin text-zinc-500" />
              ) : confirming === key ? (
                <div className="flex items-center gap-1">
                  <button
                    type="button"
                    onClick={() => kill(key, p.pid!, "TERM")}
                    title="SIGTERM: pede para o processo encerrar"
                    className="rounded-full border border-zinc-600 px-2 py-0.5 text-[11px] font-semibold text-zinc-200 hover:border-zinc-300 hover:text-zinc-100"
                  >
                    TERM
                  </button>
                  <button
                    type="button"
                    onClick={() => kill(key, p.pid!, "KILL")}
                    title="SIGKILL: encerra imediatamente"
                    className={BTN_DANGER_SOLID}
                  >
                    KILL
                  </button>
                  <button type="button" onClick={() => setConfirming(null)} aria-label="Cancelar" className="p-0.5 text-zinc-500 hover:text-zinc-200">
                    <X className="size-3.5" />
                  </button>
                </div>
              ) : (
                <button
                  type="button"
                  onClick={() => setConfirming(key)}
                  className={BTN_DANGER}
                >
                  <Skull className="size-3.5" />
                  Liberar porta
                </button>
              )}
            </div>
          </div>
        );
      })}

      {ports.length === 0 && (
        <div className="px-4 py-10 text-center text-xs text-zinc-500">Nenhuma porta TCP em escuta.</div>
      )}
    </section>
  );
}
