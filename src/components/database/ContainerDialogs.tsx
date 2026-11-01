import { useCallback, useEffect, useRef, useState } from "react";
import { Loader2, RefreshCw, ScrollText, Trash2 } from "lucide-react";
import { type ContainerInfo, type LogLine, getContainerLogs, removeContainer } from "../../lib/docker";
import { errorMessage } from "../../lib/wsl";
import { BTN_GHOST, BTN_PRIMARY } from "../../lib/ui";
import { InfoTip } from "../InfoTip";
import { DialogShell, ErrorBox } from "./common";

const TAIL = 400;

/** "14:08:23" today, "23/05 14:08:23" on other days (stopped containers keep old logs). */
function localTime(iso: string): string {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return "";
  const time = d.toLocaleTimeString("pt-BR", { hour12: false });
  return d.toDateString() === new Date().toDateString() ? time : `${d.toLocaleDateString("pt-BR", { day: "2-digit", month: "2-digit" })} ${time}`;
}

/** Last lines the container printed (stdout + stderr, in time order). */
export function ContainerLogsDialog({ distro, container, onClose }: { distro: string; container: ContainerInfo; onClose: () => void }) {
  const [lines, setLines] = useState<LogLine[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const bottom = useRef<HTMLDivElement>(null);

  const load = useCallback(async () => {
    setLoading(true);
    try {
      setLines(await getContainerLogs(distro, container.id, TAIL));
      setError(null);
    } catch (err) {
      setError(errorMessage(err));
    } finally {
      setLoading(false);
    }
  }, [distro, container.id]);

  useEffect(() => {
    void load();
  }, [load]);
  // Braces matter: scrollIntoView returns a Promise in recent WebView2, and an effect that returns
  // anything but a function crashes React ("destroy is not a function").
  useEffect(() => {
    bottom.current?.scrollIntoView({ block: "end" });
  }, [lines]);

  return (
    <DialogShell
      icon={<ScrollText className="size-5 text-zinc-100" />}
      eyebrow={`Logs · ${container.name}`}
      title="O que o container escreveu"
      description={
        <>
          As últimas {TAIL} linhas. Útil quando algo não liga: o motivo costuma estar no fim.{" "}
          <InfoTip>
            É o <span className="font-mono">docker logs</span>: tudo o que o programa do container imprimiu, com a hora de cada linha.
          </InfoTip>
        </>
      }
      width="w-[min(900px,100%)]"
      onClose={onClose}
      footer={
        <>
          <button type="button" onClick={load} disabled={loading} className={`${BTN_GHOST} h-8`}>
            {loading ? <Loader2 className="size-3.5 animate-spin" /> : <RefreshCw className="size-3.5" />}
            Atualizar
          </button>
          <button type="button" onClick={onClose} className={`${BTN_PRIMARY} h-8 px-4`}>
            Fechar
          </button>
        </>
      }
    >
      <div className="px-6 py-5">
        {error && <ErrorBox>{error}</ErrorBox>}
        {!lines && !error && (
          <div className="flex items-center gap-2 py-6 text-xs text-zinc-500">
            <Loader2 className="size-4 animate-spin" />
            Lendo os logs…
          </div>
        )}
        {lines && (
          <div className="max-h-[55vh] overflow-auto rounded-lg border border-zinc-800 bg-void p-3 font-mono text-[11px] leading-relaxed">
            {lines.length === 0 && <span className="text-zinc-600">O container ainda não escreveu nada.</span>}
            {lines.map((l, i) => (
              <div key={i} className="flex gap-3 whitespace-pre-wrap">
                <span className="shrink-0 text-zinc-600 tabular-nums">{localTime(l.time)}</span>
                <span className="min-w-0 break-all text-zinc-300">{l.text}</span>
              </div>
            ))}
            <div ref={bottom} />
          </div>
        )}
      </div>
    </DialogShell>
  );
}

/** Remove a container; deleting its data is a separate, explicit choice. */
export function RemoveContainerDialog({
  distro,
  container,
  onClose,
  onRemoved,
}: {
  distro: string;
  container: ContainerInfo;
  onClose: () => void;
  onRemoved: (message: string) => void;
}) {
  const [eraseData, setEraseData] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const isDb = container.database !== null;
  const hasVolumes = container.volumes.length > 0;

  const remove = async () => {
    setBusy(true);
    setError(null);
    try {
      const r = await removeContainer(distro, container.id, eraseData);
      onRemoved(
        r.removedVolumes.length > 0
          ? "Removido, junto com os dados."
          : r.keptVolumes.length > 0
            ? "Removido. Os dados ficaram guardados no Docker."
            : "Removido.",
      );
    } catch (err) {
      setError(errorMessage(err));
      setBusy(false);
    }
  };

  return (
    <DialogShell
      icon={<Trash2 className="size-5 text-zinc-100" />}
      eyebrow={`Remover · ${container.name}`}
      title={isDb ? "Remover este banco de dados?" : "Remover este container?"}
      description={
        isDb
          ? "O banco para de funcionar e some da lista. Projetos que usam este link de conexão deixam de conectar."
          : "O container para e some da lista. A imagem baixada continua no Docker."
      }
      busy={busy}
      onClose={onClose}
      footer={
        <>
          <button type="button" onClick={onClose} disabled={busy} className={`${BTN_GHOST} h-8`}>
            Cancelar
          </button>
          <button
            type="button"
            onClick={remove}
            disabled={busy}
            className={`inline-flex h-8 items-center gap-1.5 rounded-full border px-4 text-xs font-semibold transition-colors disabled:opacity-40 ${
              eraseData ? "border-rose-500/40 text-rose-300 hover:bg-rose-500/15" : "border-zinc-700 text-zinc-100 hover:border-zinc-500"
            }`}
          >
            {busy ? <Loader2 className="size-3.5 animate-spin" /> : <Trash2 className="size-3.5" />}
            {eraseData ? "Remover e apagar os dados" : "Remover"}
          </button>
        </>
      }
    >
      <div className="space-y-4 px-6 py-5">
        {hasVolumes ? (
          <label className="flex cursor-pointer items-start gap-3 rounded-lg border border-zinc-800 bg-zinc-900 p-4">
            <input type="checkbox" checked={eraseData} onChange={(e) => setEraseData(e.target.checked)} className="mt-0.5 accent-rose-400" />
            <span className="text-xs leading-relaxed">
              <span className="block text-sm font-medium text-zinc-100">Apagar também os dados</span>
              <span className="text-zinc-500">
                {isDb ? "Todas as tabelas e registros deste banco" : "Os arquivos que este container guardou"} serão apagados de vez. Sem marcar,
                eles ficam guardados no Docker ocupando espaço em disco (não memória).{" "}
                <InfoTip>
                  {container.volumes.length === 1 ? "Volume" : "Volumes"}: <span className="font-mono">{container.volumes.join(", ")}</span>
                </InfoTip>
              </span>
            </span>
          </label>
        ) : (
          <p className="text-xs leading-relaxed text-zinc-500">Este container não guarda dados em volumes: não há nada além dele para apagar.</p>
        )}
        {eraseData && (
          <p className="text-xs leading-relaxed text-rose-300">Isso não pode ser desfeito. Se os dados importam, exporte antes pelo Editor SQL ou por uma ferramenta de backup.</p>
        )}
        {error && <ErrorBox>{error}</ErrorBox>}
      </div>
    </DialogShell>
  );
}
