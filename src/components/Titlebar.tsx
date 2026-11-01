import { useEffect, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { ArrowUp, Loader2, Play, Power, RefreshCw, Settings2 } from "lucide-react";
import type { Distro } from "../lib/wsl";
import { type PendingAction, isDockerDistro } from "../hooks/useWsl";
import { useUpdaterContext } from "../hooks/useUpdater";
import { percentOf } from "../lib/updater";
import { BTN_DANGER, BTN_GHOST, BTN_PRIMARY } from "../lib/ui";
import { DistroSelect } from "./DistroSelect";
import { LogoMark } from "./Logo";

interface Props {
  section: string;
  distros: Distro[];
  selected: Distro | null;
  loading: boolean;
  pending: PendingAction;
  onSelectDistro: (name: string) => void;
  onStart: () => void;
  onShutdown: () => void;
  onRefresh: () => void;
  onOpenSettings: () => void;
}

/** Frameless window chrome: drag region, quick WSL controls and the window buttons. */
export function Titlebar(p: Props) {
  const running = p.selected?.state === "Running";
  const busy = p.pending !== null;

  return (
    <header
      data-tauri-drag-region
      className="relative z-30 flex h-11 shrink-0 items-center border-b border-[#18181e] bg-void pl-3.5"
    >
      <div data-tauri-drag-region className="flex items-center gap-2.5">
        <LogoMark className="pointer-events-none size-[18px] text-zinc-100" />
        <span data-tauri-drag-region className="text-[13px] font-extrabold tracking-[0.28em] text-zinc-100">
          SYMBIOSIS
        </span>
      </div>
      <span data-tauri-drag-region className="mx-3.5 h-4 w-px bg-zinc-800" />
      <span data-tauri-drag-region className="truncate text-[11px] font-medium tracking-wider text-zinc-500 uppercase">
        {p.section}
      </span>

      <div data-tauri-drag-region className="h-full min-w-6 flex-1" />

      <div className="flex items-center gap-2 pr-2">
        <UpdatePill />

        <DistroSelect distros={p.distros} selected={p.selected} loading={p.loading} onSelect={p.onSelectDistro} />

        <PowerButton {...p} running={running} busy={busy} />

        <span className="mx-0.5 h-4 w-px bg-zinc-800" />
        <button
          type="button"
          onClick={p.onRefresh}
          title="Atualizar distros"
          className="rounded-md p-1.5 text-zinc-500 transition-colors hover:bg-zinc-900 hover:text-zinc-100"
        >
          <RefreshCw className="size-4" />
        </button>
        <button
          type="button"
          onClick={p.onOpenSettings}
          title="Diagnóstico & Configurações"
          className="rounded-md p-1.5 text-zinc-500 transition-colors hover:bg-zinc-900 hover:text-zinc-100"
        >
          <Settings2 className="size-4" />
        </button>
      </div>

      <WindowControls />
    </header>
  );
}

/** One button that turns things on or off, never both at once. For the docker-desktop distro it
 *  talks about Docker (it starts/stops Docker Desktop itself), not about WSL. */
function PowerButton(p: Props & { running: boolean; busy: boolean }) {
  const docker = isDockerDistro(p.selected?.name);
  // No distro selected (or none installed): nothing to start, but WSL can still be shut down.
  const on = p.running || !p.selected;
  const label =
    p.pending === "start"
      ? docker ? "Ligando o Docker…" : "Iniciando…"
      : p.pending === "shutdown"
        ? docker ? "Desligando o Docker…" : "Desligando…"
        : on
          ? docker ? "Desligar o Docker" : "Desligar WSL"
          : docker ? "Ligar o Docker" : "Iniciar";
  const title = on
    ? docker
      ? "Fecha o Docker Desktop de forma segura (docker desktop stop)"
      : "Desliga todas as distros e o Docker Desktop, se estiver ligado (wsl --shutdown)"
    : docker
      ? "Abre o Docker Desktop e espera ele ficar pronto"
      : `Liga a distro ${p.selected?.name}`;
  return (
    <button
      type="button"
      onClick={on ? p.onShutdown : p.onStart}
      disabled={p.busy}
      title={title}
      className={`${on ? BTN_DANGER : BTN_GHOST} h-7 min-w-[8.5rem] justify-center whitespace-nowrap`}
    >
      {p.busy ? <Loader2 className="size-3.5 animate-spin" /> : on ? <Power className="size-3.5" /> : <Play className="size-3.5" />}
      {label}
    </button>
  );
}

/** Shows up only when GitHub has a newer version (or while it downloads). */
function UpdatePill() {
  const { state, openDialog } = useUpdaterContext();
  if (state.status === "downloading" || state.status === "ready") {
    const pct = state.status === "ready" ? 100 : percentOf(state.downloaded, state.total);
    return (
      <button type="button" onClick={openDialog} className={`${BTN_GHOST} h-7 font-mono tabular-nums`}>
        <Loader2 className="size-3.5 animate-spin" />
        {state.status === "ready" ? "Instalando…" : `Baixando${pct !== null ? ` ${pct.toFixed(0)}%` : "…"}`}
      </button>
    );
  }
  const update = state.status === "available" || state.status === "error" ? state.update : undefined;
  if (!update) return null;
  return (
    <button
      type="button"
      onClick={openDialog}
      title="Ver novidades e instalar"
      className={`${BTN_PRIMARY} relative h-7 whitespace-nowrap`}
    >
      <span className="pointer-events-none absolute -inset-[3px] animate-pulse rounded-full border border-zinc-100/40" />
      <ArrowUp className="size-3.5" />
      Atualizar para v{update.version}
    </button>
  );
}

/** Minimize / maximize / close (also used by the full-screen first-run setup). */
export function WindowControls() {
  const [maximized, setMaximized] = useState(false);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let alive = true;
    try {
      const win = getCurrentWindow();
      const sync = () => win.isMaximized().then((m) => alive && setMaximized(m)).catch(() => {});
      sync();
      win
        .onResized(sync)
        .then((fn) => (alive ? (unlisten = fn) : fn()))
        .catch(() => {});
    } catch {
      /* not running inside Tauri (browser preview) */
    }
    return () => {
      alive = false;
      unlisten?.();
    };
  }, []);

  const act = (fn: (w: ReturnType<typeof getCurrentWindow>) => Promise<void>) => () => {
    try {
      fn(getCurrentWindow()).catch(() => {});
    } catch {
      /* browser preview */
    }
  };

  const base = "flex h-11 w-[46px] items-center justify-center text-zinc-400 transition-colors";
  return (
    <div className="flex self-start border-l border-[#18181e]">
      <button type="button" aria-label="Minimizar" onClick={act((w) => w.minimize())} className={`${base} hover:bg-zinc-900 hover:text-zinc-100`}>
        <svg viewBox="0 0 10 10" className="size-2.5" stroke="currentColor" strokeWidth={1}>
          <path d="M0 5.5h10" />
        </svg>
      </button>
      <button
        type="button"
        aria-label={maximized ? "Restaurar" : "Maximizar"}
        onClick={act((w) => w.toggleMaximize())}
        className={`${base} hover:bg-zinc-900 hover:text-zinc-100`}
      >
        {maximized ? (
          <svg viewBox="0 0 10 10" className="size-2.5" fill="none" stroke="currentColor" strokeWidth={1}>
            <path d="M2.5 2.5V.5h7v7h-2" />
            <rect x=".5" y="2.5" width="7" height="7" />
          </svg>
        ) : (
          <svg viewBox="0 0 10 10" className="size-2.5" fill="none" stroke="currentColor" strokeWidth={1}>
            <rect x=".5" y=".5" width="9" height="9" />
          </svg>
        )}
      </button>
      <button type="button" aria-label="Fechar" onClick={act((w) => w.close())} className={`${base} hover:bg-rose-600/85 hover:text-white`}>
        <svg viewBox="0 0 10 10" className="size-2.5" stroke="currentColor" strokeWidth={1}>
          <path d="M0 0l10 10M10 0L0 10" />
        </svg>
      </button>
    </div>
  );
}
