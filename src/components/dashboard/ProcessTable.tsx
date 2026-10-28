import { useMemo, useState } from "react";
import { Loader2, Search, Skull, X } from "lucide-react";
import { KillSignal, LinuxProcess, errorMessage, killLinuxProcess } from "../../lib/wsl";
import { formatBytes } from "../../lib/format";
import { BTN_DANGER_SOLID, FIELD, INPUT, LABEL } from "../../lib/ui";

const DEFAULT_ROWS = 8;

interface Props {
  distro: string;
  processes: LinuxProcess[];
  onKilled: () => void;
}

export function ProcessTable({ distro, processes, onKilled }: Props) {
  const [query, setQuery] = useState("");
  const [confirming, setConfirming] = useState<number | null>(null);
  const [killing, setKilling] = useState<number | null>(null);
  const [error, setError] = useState<string | null>(null);

  const hasCpu = processes.some((p) => p.cpu !== null);
  const rows = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!q) return processes.slice(0, DEFAULT_ROWS);
    return processes.filter(
      (p) => p.command.toLowerCase().includes(q) || p.user.toLowerCase().includes(q) || String(p.pid) === q,
    );
  }, [processes, query]);

  const kill = async (pid: number, signal: KillSignal) => {
    setKilling(pid);
    setError(null);
    try {
      await killLinuxProcess(distro, pid, signal);
      onKilled();
    } catch (err) {
      setError(`PID ${pid}: ${errorMessage(err)}`);
    } finally {
      setKilling(null);
      setConfirming(null);
    }
  };

  return (
    <section className="overflow-hidden rounded-lg border border-zinc-800 bg-zinc-900">
      <div className="flex items-center gap-3 border-b border-zinc-800 px-4 py-2">
        <span className={LABEL}>
          Processos ativos na distro
        </span>
        <span className="text-[11px] text-zinc-600">
          {query ? `${rows.length} de ${processes.length}` : `top ${Math.min(DEFAULT_ROWS, processes.length)} por memória`}
        </span>
        <label className={`${FIELD} ml-auto w-56`}>
          <Search className="size-3.5 text-zinc-500" />
          <input
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="Buscar comando, usuário ou PID"
            className={INPUT}
          />
        </label>
      </div>

      {error && (
        <div className="flex items-center gap-2 border-b border-rose-500/20 px-4 py-2 text-xs text-rose-300">
          <span className="flex-1 font-mono">{error}</span>
          <button type="button" onClick={() => setError(null)} className="text-rose-400/70 hover:text-rose-200">
            <X className="size-3.5" />
          </button>
        </div>
      )}

      <table className="w-full table-fixed text-sm">
        <thead>
          <tr className="border-b border-zinc-800 text-left text-[10px] tracking-wider text-zinc-600 uppercase">
            <th className="w-20 px-4 py-2 font-normal">PID</th>
            <th className="w-24 px-2 py-2 font-normal">Usuário</th>
            {hasCpu && <th className="w-16 px-2 py-2 text-right font-normal">CPU</th>}
            <th className="w-24 px-2 py-2 text-right font-normal">Memória</th>
            <th className="px-4 py-2 font-normal">Comando</th>
            <th className="w-44 px-4 py-2" />
          </tr>
        </thead>
        <tbody>
          {rows.map((p) => (
            <tr key={p.pid} className="group border-b border-zinc-800/50 last:border-0 hover:bg-zinc-800/30">
              <td className="px-4 py-2 font-mono text-xs text-zinc-400 tabular-nums">{p.pid}</td>
              <td className="truncate px-2 py-2 text-xs text-zinc-400">{p.user}</td>
              {hasCpu && (
                <td className="px-2 py-2 text-right font-mono text-xs text-zinc-400 tabular-nums">
                  {p.cpu === null ? "—" : `${p.cpu.toFixed(1)}%`}
                </td>
              )}
              <td className="px-2 py-2 text-right font-mono text-xs text-zinc-300 tabular-nums">{formatBytes(p.rss)}</td>
              <td className="truncate px-4 py-2 font-mono text-xs text-zinc-300" title={p.command}>
                {p.command}
              </td>
              <td className="px-4 py-1.5 text-right">
                {killing === p.pid ? (
                  <Loader2 className="ml-auto size-3.5 animate-spin text-zinc-500" />
                ) : confirming === p.pid ? (
                  <div className="flex items-center justify-end gap-1">
                    <button
                      type="button"
                      onClick={() => kill(p.pid, "TERM")}
                      title="SIGTERM: pede para o processo encerrar"
                      className="rounded-full border border-zinc-600 px-2 py-0.5 text-[11px] font-semibold text-zinc-200 hover:border-zinc-300 hover:text-zinc-100"
                    >
                      TERM
                    </button>
                    <button
                      type="button"
                      onClick={() => kill(p.pid, "KILL")}
                      title="SIGKILL: encerra imediatamente"
                      className={BTN_DANGER_SOLID}
                    >
                      KILL
                    </button>
                    <button
                      type="button"
                      onClick={() => setConfirming(null)}
                      className="rounded p-0.5 text-zinc-500 hover:text-zinc-200"
                      aria-label="Cancelar"
                    >
                      <X className="size-3.5" />
                    </button>
                  </div>
                ) : (
                  <button
                    type="button"
                    onClick={() => setConfirming(p.pid)}
                    disabled={p.pid <= 1}
                    className="inline-flex items-center gap-1 rounded-full border border-transparent px-2 py-0.5 text-[11px] text-zinc-500 opacity-0 transition-colors group-hover:opacity-100 hover:border-rose-500/40 hover:text-rose-300 focus:opacity-100 disabled:hidden"
                  >
                    <Skull className="size-3" />
                    Encerrar
                  </button>
                )}
              </td>
            </tr>
          ))}
          {rows.length === 0 && (
            <tr>
              <td colSpan={6} className="px-4 py-6 text-center text-xs text-zinc-500">
                {query ? "Nenhum processo encontrado." : "Sem processos."}
              </td>
            </tr>
          )}
        </tbody>
      </table>
    </section>
  );
}
