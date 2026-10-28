import { useEffect, useState, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { AlertTriangle, ArrowRight, CheckCircle2, HardDrive, Loader2, Minimize2, ShieldAlert, X } from "lucide-react";
import { formatBytes } from "../../lib/format";
import { BTN_GHOST, BTN_PRIMARY } from "../../lib/ui";
import { type VhdxCompactResult, type VhdxInfo, compactDistroVhdx, errorMessage } from "../../lib/wsl";
import { InfoTip } from "../InfoTip";

interface Props {
  distro: string;
  vhdx: VhdxInfo;
  /** Used space inside Linux, when the distro is running (for the estimate). */
  linuxUsed: number | null;
  runningProjects: number;
  onClose: () => void;
  /** WSL was shut down (with or without a successful compaction). */
  onWslRestarted: () => void;
}

type Phase = { step: "confirm" } | { step: "running" } | { step: "done"; result: VhdxCompactResult } | { step: "error"; message: string };

/** Confirm → fstrim + wsl --shutdown + Optimize-VHD/diskpart (UAC) → before/after. */
export function CompactVhdxDialog({ distro, vhdx, linuxUsed, runningProjects, onClose, onWslRestarted }: Props) {
  const [phase, setPhase] = useState<Phase>({ step: "confirm" });
  const running = phase.step === "running";

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape" || running) return;
      e.preventDefault();
      e.stopPropagation();
      onClose();
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [onClose, running]);

  const start = async () => {
    setPhase({ step: "running" });
    try {
      const result = await compactDistroVhdx(distro);
      setPhase({ step: "done", result });
    } catch (err) {
      setPhase({ step: "error", message: errorMessage(err) });
    } finally {
      onWslRestarted();
    }
  };

  const estimate = linuxUsed !== null ? Math.max(0, vhdx.fileSize - linuxUsed) : null;

  // Portaled: inside a screen's layout, spacing utilities (space-y-*) would shrink the backdrop.
  return createPortal(
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/70 p-6" onMouseDown={() => !running && onClose()}>
      <div
        role="dialog"
        aria-modal
        aria-label="Compactar disco virtual"
        onMouseDown={(e) => e.stopPropagation()}
        className="max-h-full w-[min(560px,100%)] overflow-y-auto rounded-xl border border-zinc-800 bg-zinc-950 shadow-2xl shadow-black"
      >
        <header className="flex items-start gap-3.5 border-b border-zinc-800 bg-void px-6 py-5">
          <div className="flex size-10 shrink-0 items-center justify-center rounded-lg border border-zinc-800 bg-zinc-950">
            <HardDrive className="size-5 text-zinc-100" />
          </div>
          <div className="min-w-0 flex-1">
            <div className="text-[11px] font-medium tracking-[0.2em] text-zinc-500 uppercase">Disco do Linux · {distro}</div>
            <h2 className="mt-0.5 flex items-center gap-2 text-lg font-semibold tracking-tight text-zinc-100">
              Compactar VHDX
              <InfoTip>{VHDX_EXPLAINED}</InfoTip>
            </h2>
            <p className="mt-1 truncate font-mono text-[11px] text-zinc-500" title={vhdx.path}>
              {vhdx.path}
            </p>
          </div>
          <button type="button" onClick={onClose} disabled={running} aria-label="Fechar" className="rounded-md p-1.5 text-zinc-500 hover:bg-zinc-900 hover:text-zinc-100 disabled:opacity-30">
            <X className="size-4" />
          </button>
        </header>

        <div className="px-6 py-5">
          {phase.step === "confirm" && (
            <>
              <div className="grid grid-cols-2 divide-x divide-zinc-800 rounded-lg border border-zinc-800 bg-zinc-900">
                <Cell label="Arquivo no Windows" value={formatBytes(vhdx.fileSize)} />
                <Cell label="Recuperável (estimativa)" value={estimate !== null ? `~${formatBytes(estimate)}` : "ligue a distro para estimar"} muted={estimate === null} />
              </div>
              <p className="mt-4 text-xs leading-relaxed text-zinc-400">
                Quando você apaga arquivos no Linux, o espaço fica livre por dentro, mas o arquivo do disco no Windows continua do mesmo
                tamanho. Compactar devolve esse espaço vazio para o seu disco {vhdx.path.slice(0, 2).toUpperCase()}. O que acontece:
              </p>
              <ol className="mt-3 space-y-3 text-xs leading-relaxed text-zinc-400">
                <Step n={1}>
                  <span className="text-zinc-100">O Linux marca o espaço dos arquivos que você já apagou</span>, para que ele possa ser
                  devolvido. Se a distro estiver desligada, ela liga por alguns segundos.
                </Step>
                <Step n={2}>
                  <span className="text-zinc-100">O WSL é desligado</span>: terminais abertos fecham
                  {runningProjects > 0 ? (
                    <>
                      {" "}e <span className="text-amber-300">{runningProjects === 1 ? "o projeto em execução para" : `os ${runningProjects} projetos em execução param`}</span>
                    </>
                  ) : (
                    " e projetos em execução param"
                  )}
                  . Depois é só ligar de novo.
                </Step>
                <Step n={3}>
                  <span className="text-zinc-100">O Windows pede permissão de administrador</span> (clique em <b className="font-medium text-zinc-200">Sim</b>)
                  para encolher o arquivo. Em discos grandes, leva alguns minutos.
                </Step>
              </ol>
              <p className="mt-4 flex items-start gap-2 text-[11px] leading-relaxed text-zinc-500">
                <ShieldAlert className="mt-px size-3.5 shrink-0" />
                Nenhum arquivo do Linux é apagado: só o espaço vazio do disco virtual volta para o Windows.
              </p>
            </>
          )}

          {phase.step === "running" && (
            <div className="flex flex-col items-center gap-3 py-6 text-center">
              <Loader2 className="size-6 animate-spin text-zinc-300" />
              <p className="text-sm font-medium text-zinc-100">Compactando o disco virtual…</p>
              <p className="max-w-sm text-xs leading-relaxed text-zinc-500">
                Confirme o pedido de administrador do Windows, se aparecer. Não feche o app: em discos grandes isso leva alguns minutos.
              </p>
            </div>
          )}

          {phase.step === "done" && (
            <div className="space-y-3">
              <div className="flex items-center justify-center gap-4 rounded-lg border border-zinc-800 bg-zinc-900 px-4 py-5 font-mono">
                <span className="text-xl text-zinc-400 tabular-nums">{formatBytes(phase.result.sizeBefore)}</span>
                <ArrowRight className="size-4 text-zinc-600" />
                <span className="text-2xl font-medium text-zinc-100 tabular-nums">{formatBytes(phase.result.sizeAfter)}</span>
              </div>
              <p className="flex items-center justify-center gap-2 text-sm text-zinc-100">
                <CheckCircle2 className="size-4 text-emerald-400" />
                {phase.result.reclaimedBytes > 0
                  ? `${formatBytes(phase.result.reclaimedBytes)} devolvidos ao Windows`
                  : "O disco já estava compacto"}
              </p>
              <p className="text-center text-xs leading-relaxed text-zinc-500">
                O WSL ficou desligado: clique em <b className="font-medium text-zinc-300">Iniciar</b> (ou abra um terminal) para voltar a usar.
              </p>
              <p className="text-center font-mono text-[10px] text-zinc-600">
                {phase.result.method === "optimize-vhd" ? "Optimize-VHD" : "diskpart"}
                {!phase.result.trimmed && " · sem fstrim nesta distro"}
              </p>
            </div>
          )}

          {phase.step === "error" && (
            <div className="flex items-start gap-2.5 rounded-lg border border-rose-500/20 bg-rose-500/[0.04] px-4 py-3 text-xs text-rose-300">
              <AlertTriangle className="mt-px size-3.5 shrink-0" />
              <span className="font-mono break-words whitespace-pre-wrap">{phase.message}</span>
            </div>
          )}
        </div>

        <footer className="flex items-center justify-end gap-2 border-t border-zinc-800 bg-void px-6 py-3.5">
          {phase.step === "confirm" ? (
            <>
              <button type="button" onClick={onClose} className={`${BTN_GHOST} h-8`}>
                Cancelar
              </button>
              <button type="button" onClick={start} className={`${BTN_PRIMARY} h-8 px-4`}>
                <Minimize2 className="size-3.5" />
                Desligar WSL e compactar
              </button>
            </>
          ) : (
            <button type="button" onClick={onClose} disabled={running} className={`${BTN_GHOST} h-8`}>
              {phase.step === "error" ? "Fechar" : "Concluir"}
            </button>
          )}
        </footer>
      </div>
    </div>,
    document.body,
  );
}

function Cell({ label, value, muted }: { label: string; value: string; muted?: boolean }) {
  return (
    <div className="min-w-0 px-4 py-3">
      <div className="text-[10px] tracking-wider text-zinc-500 uppercase">{label}</div>
      <div className={`mt-1 truncate font-mono ${muted ? "text-xs text-zinc-500" : "text-lg text-zinc-100"}`}>{value}</div>
    </div>
  );
}

function Step({ n, children }: { n: number; children: ReactNode }) {
  return (
    <li className="flex items-center gap-3">
      <span className="flex size-6 shrink-0 items-center justify-center rounded-full border border-zinc-700 bg-zinc-900 font-mono text-[11px] leading-none text-zinc-300">
        {n}
      </span>
      <span>{children}</span>
    </li>
  );
}

/** What a VHDX is, for people who never heard the word. Shared with the Dashboard's disk card. */
export const VHDX_EXPLAINED = (
  <>
    <b className="font-semibold text-zinc-100">VHDX</b> é o arquivo onde o Linux inteiro mora: programas, configurações e todos os seus
    projetos. Para o Windows, a distro é esse único arquivo (um disco virtual). Ele cresce quando você cria arquivos no Linux, mas não
    diminui sozinho quando você apaga: o espaço fica reservado até você compactar.
  </>
);
