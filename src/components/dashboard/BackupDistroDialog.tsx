import { useEffect, useState, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { open as pickFile, save as pickSave } from "@tauri-apps/plugin-dialog";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { AlertTriangle, ArchiveRestore, CheckCircle2, FolderOpen, Loader2, PackageOpen, Save, X } from "lucide-react";
import { formatBytes } from "../../lib/format";
import { BTN_GHOST, BTN_PRIMARY, FIELD, INPUT } from "../../lib/ui";
import { errorMessage } from "../../lib/wsl";
import {
  type BackupDefaults,
  backupFileName,
  distroNameError,
  exportDistroBackup,
  formatDuration,
  getBackupDefaults,
  importDistroBackup,
} from "../../lib/backup";
import { InfoTip } from "../InfoTip";

type Mode = { kind: "export"; distro: string; estimate: number | null } | { kind: "import" };

interface Props {
  mode: Mode;
  /** Every installed distro (the restored one needs a new name). */
  distros: string[];
  onClose: () => void;
  /** A distro was created (import). */
  onImported: () => void;
}

type Phase = { step: "form" } | { step: "running"; startedAt: number } | { step: "done"; message: ReactNode; file?: string } | { step: "error"; message: string };

/** "Fazer backup (.tar)" and "Importar backup": wsl --export / --import with plain explanations. */
export function BackupDistroDialog({ mode, distros, onClose, onImported }: Props) {
  const [defaults, setDefaults] = useState<BackupDefaults | null>(null);
  const [phase, setPhase] = useState<Phase>({ step: "form" });
  const [now, setNow] = useState(Date.now());
  // Export
  const [destination, setDestination] = useState<string | null>(null);
  // Import
  const [source, setSource] = useState<string | null>(null);
  const [name, setName] = useState("");
  const [installDir, setInstallDir] = useState<string | null>(null);
  const running = phase.step === "running";

  useEffect(() => {
    getBackupDefaults()
      .then((d) => {
        setDefaults(d);
        if (mode.kind === "export") setDestination(`${d.backupDir}\\${backupFileName(mode.distro)}`);
      })
      .catch(() => {});
  }, [mode]);

  // Elapsed time while wsl.exe works (it reports no usable progress).
  useEffect(() => {
    if (!running) return;
    const id = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(id);
  }, [running]);

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

  const chooseDestination = async () => {
    const picked = await pickSave({
      title: "Onde salvar o backup",
      defaultPath: destination ?? undefined,
      filters: [{ name: "Backup do WSL (.tar)", extensions: ["tar"] }],
    }).catch(() => null);
    if (picked) setDestination(/\.tar$/i.test(picked) ? picked : `${picked}.tar`);
  };

  const chooseSource = async () => {
    const picked = await pickFile({
      title: "Escolha o backup (.tar)",
      defaultPath: defaults?.backupDir,
      multiple: false,
      directory: false,
      filters: [{ name: "Backup do WSL (.tar)", extensions: ["tar"] }],
    }).catch(() => null);
    if (typeof picked !== "string") return;
    setSource(picked);
    if (!name) {
      // "Ubuntu_2026-10-28.tar" → "Ubuntu-restaurada" when Ubuntu exists.
      const base = (picked.split("\\").pop() ?? "Ubuntu").replace(/\.tar$/i, "").split("_")[0].replace(/[^A-Za-z0-9._-]/g, "") || "Linux";
      let candidate = base;
      for (let i = 1; distros.some((d) => d.toLowerCase() === candidate.toLowerCase()); i++) {
        candidate = i === 1 ? `${base}-restaurada` : `${base}-restaurada-${i}`;
      }
      setName(candidate);
    }
  };

  const chooseInstallDir = async () => {
    const picked = await pickFile({ title: "Pasta da nova distro", directory: true, multiple: false, defaultPath: defaults?.installRoot }).catch(
      () => null,
    );
    if (typeof picked === "string") setInstallDir(name ? `${picked}\\${name}` : picked);
  };

  const effectiveInstallDir = installDir ?? (defaults && name ? `${defaults.installRoot}\\${name}` : null);
  const nameError = distroNameError(name, distros);

  const start = async () => {
    setPhase({ step: "running", startedAt: Date.now() });
    try {
      if (mode.kind === "export") {
        const r = await exportDistroBackup(mode.distro, destination!);
        setPhase({
          step: "done",
          file: r.file,
          message: (
            <>
              Backup salvo: <span className="font-mono text-zinc-100">{formatBytes(r.size)}</span> em {formatDuration(r.seconds)}.
            </>
          ),
        });
      } else {
        const r = await importDistroBackup(name, effectiveInstallDir!, source!);
        onImported();
        setPhase({
          step: "done",
          message: (
            <>
              Pronto! A distro <span className="font-medium text-zinc-100">{r.distro}</span> foi criada
              {r.defaultUser ? (
                <>
                  {" "}e abre com o usuário <span className="font-mono text-zinc-100">{r.defaultUser}</span>
                </>
              ) : (
                " (ela abre como root: o backup não tinha um usuário comum)"
              )}
              . Escolha-a no seletor de distros da barra superior.
            </>
          ),
        });
      }
    } catch (err) {
      setPhase({ step: "error", message: errorMessage(err) });
    }
  };

  const canStart =
    mode.kind === "export" ? !!destination : !!source && !!name && !nameError && !!effectiveInstallDir;

  return createPortal(
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/70 p-6" onMouseDown={() => !running && onClose()}>
      <div
        role="dialog"
        aria-modal
        aria-label={mode.kind === "export" ? "Fazer backup da distro" : "Restaurar backup"}
        onMouseDown={(e) => e.stopPropagation()}
        className="max-h-full w-[min(580px,100%)] overflow-y-auto rounded-xl border border-zinc-800 bg-zinc-950 shadow-2xl shadow-black"
      >
        <header className="flex items-start gap-3.5 border-b border-zinc-800 bg-void px-6 py-5">
          <div className="flex size-10 shrink-0 items-center justify-center rounded-lg border border-zinc-800 bg-zinc-950">
            {mode.kind === "export" ? <PackageOpen className="size-5 text-zinc-100" /> : <ArchiveRestore className="size-5 text-zinc-100" />}
          </div>
          <div className="min-w-0 flex-1">
            <div className="text-[11px] font-medium tracking-[0.2em] text-zinc-500 uppercase">
              {mode.kind === "export" ? `Backup · ${mode.distro}` : "Backup"}
            </div>
            <h2 className="mt-0.5 text-lg font-semibold tracking-tight text-zinc-100">
              {mode.kind === "export" ? "Fazer uma cópia de segurança" : "Restaurar um backup"}
            </h2>
            <p className="mt-1 text-xs leading-relaxed text-zinc-500">
              {mode.kind === "export"
                ? "Salva o Linux inteiro (programas, configurações e projetos) em um único arquivo no Windows. Use antes de mudanças grandes, para levar o ambiente a outro computador ou para criar uma cópia."
                : "Cria uma distro a partir de um arquivo de backup. Dá para restaurar com outro nome, como uma cópia: a distro original não é tocada."}
            </p>
          </div>
          <button type="button" onClick={onClose} disabled={running} aria-label="Fechar" className="rounded-md p-1.5 text-zinc-500 hover:bg-zinc-900 hover:text-zinc-100 disabled:opacity-30">
            <X className="size-4" />
          </button>
        </header>

        <div className="space-y-4 px-6 py-5">
          {phase.step === "form" && mode.kind === "export" && (
            <>
              <PathField label="Onde salvar" value={destination} placeholder="Escolha um local" onPick={chooseDestination} pickLabel="Escolher onde salvar" />
              <ul className="space-y-1.5 text-xs leading-relaxed text-zinc-500">
                <li>
                  • O arquivo fica com mais ou menos o tamanho do que o Linux usa
                  {mode.estimate !== null && (
                    <>
                      {" "}
                      (<span className="font-mono text-zinc-300">~{formatBytes(mode.estimate)}</span>)
                    </>
                  )}
                  : confira se há espaço livre nesse disco.
                </li>
                <li>• Em distros grandes leva vários minutos. Deixe o app aberto até terminar.</li>
                <li>
                  • Para usar depois: <b className="font-medium text-zinc-300">Importar backup</b>, na tabela de distros do Dashboard.{" "}
                  <InfoTip>
                    Por baixo é o <span className="font-mono">wsl --export</span>: um arquivo .tar com todos os arquivos da distro.
                  </InfoTip>
                </li>
              </ul>
            </>
          )}

          {phase.step === "form" && mode.kind === "import" && (
            <>
              <PathField label="Arquivo de backup (.tar)" value={source} placeholder="Nenhum arquivo escolhido" onPick={chooseSource} pickLabel="Escolher arquivo" />
              <label className="block">
                <span className="mb-1 flex items-center gap-1.5 text-[11px] text-zinc-400">
                  Nome da nova distro
                  <InfoTip>É o nome que aparece no seletor de distros e no Windows Terminal. Sem espaços.</InfoTip>
                </span>
                <span className={`${FIELD} ${nameError ? "border-rose-500/50" : ""}`}>
                  <input value={name} onChange={(e) => setName(e.target.value.trim())} placeholder="Ubuntu-restaurada" className={`${INPUT} py-0.5 font-mono text-sm`} />
                </span>
                {nameError && <span className="mt-1 block text-[11px] text-rose-300">{nameError}</span>}
              </label>
              <PathField
                label="Onde guardar a nova distro"
                info="É a pasta onde fica o disco virtual (VHDX) da nova distro. Precisa de espaço livre parecido com o tamanho do backup."
                value={effectiveInstallDir}
                placeholder="Escolha um nome primeiro"
                onPick={chooseInstallDir}
                pickLabel="Escolher pasta"
              />
            </>
          )}

          {phase.step === "running" && (
            <div className="flex flex-col items-center gap-3 py-6 text-center">
              <Loader2 className="size-6 animate-spin text-zinc-300" />
              <p className="text-sm font-medium text-zinc-100">{mode.kind === "export" ? "Salvando o backup…" : "Restaurando o backup…"}</p>
              <p className="font-mono text-xs text-zinc-500 tabular-nums">{formatDuration((now - phase.startedAt) / 1000)}</p>
              <p className="max-w-sm text-xs leading-relaxed text-zinc-500">Em distros grandes isso leva vários minutos. Não feche o app.</p>
            </div>
          )}

          {phase.step === "done" && (
            <p className="flex items-start gap-2 text-sm leading-relaxed text-zinc-300">
              <CheckCircle2 className="mt-0.5 size-4 shrink-0 text-emerald-400" />
              <span>{phase.message}</span>
            </p>
          )}

          {phase.step === "error" && (
            <div className="flex items-start gap-2.5 rounded-lg border border-rose-500/20 bg-rose-500/[0.04] px-4 py-3 text-xs text-rose-300">
              <AlertTriangle className="mt-px size-3.5 shrink-0" />
              <span className="break-words whitespace-pre-wrap">{phase.message}</span>
            </div>
          )}
        </div>

        <footer className="flex items-center justify-end gap-2 border-t border-zinc-800 bg-void px-6 py-3.5">
          {phase.step === "form" ? (
            <>
              <button type="button" onClick={onClose} className={`${BTN_GHOST} h-8`}>
                Cancelar
              </button>
              <button type="button" onClick={start} disabled={!canStart} className={`${BTN_PRIMARY} h-8 px-4`}>
                {mode.kind === "export" ? <Save className="size-3.5" /> : <ArchiveRestore className="size-3.5" />}
                {mode.kind === "export" ? "Salvar backup" : "Restaurar backup"}
              </button>
            </>
          ) : (
            <>
              {phase.step === "done" && phase.file && (
                <button type="button" onClick={() => revealItemInDir(phase.file!).catch(() => {})} className={`${BTN_GHOST} h-8`}>
                  <FolderOpen className="size-3.5" />
                  Mostrar no Explorer
                </button>
              )}
              {phase.step === "error" && (
                <button type="button" onClick={() => setPhase({ step: "form" })} className={`${BTN_GHOST} h-8`}>
                  Voltar
                </button>
              )}
              <button type="button" onClick={onClose} disabled={running} className={`${BTN_PRIMARY} h-8 px-4`}>
                {phase.step === "error" ? "Fechar" : "Concluir"}
              </button>
            </>
          )}
        </footer>
      </div>
    </div>,
    document.body,
  );
}

function PathField(p: { label: string; info?: string; value: string | null; placeholder: string; onPick: () => void; pickLabel: string }) {
  return (
    <div>
      <span className="mb-1 flex items-center gap-1.5 text-[11px] text-zinc-400">
        {p.label}
        {p.info && <InfoTip>{p.info}</InfoTip>}
      </span>
      <div className="flex items-center gap-2">
        <span className={`${FIELD} min-w-0 flex-1`}>
          <span className={`truncate py-0.5 font-mono text-xs ${p.value ? "text-zinc-200" : "text-zinc-600"}`} title={p.value ?? undefined}>
            {p.value ?? p.placeholder}
          </span>
        </span>
        <button type="button" onClick={p.onPick} className={`${BTN_GHOST} shrink-0`}>
          <FolderOpen className="size-3.5" />
          {p.pickLabel}
        </button>
      </div>
    </div>
  );
}
