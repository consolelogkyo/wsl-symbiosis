import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import {
  AlertTriangle,
  ArrowLeft,
  ArrowRight,
  Check,
  ChevronDown,
  Download,
  ExternalLink,
  Eye,
  EyeOff,
  Loader2,
  Play,
  Power,
  RefreshCw,
  ShieldCheck,
  SkipForward,
  TerminalSquare,
} from "lucide-react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { LogoMark } from "../components/Logo";
import { SetupIntro } from "../components/SetupIntro";
import { WindowControls } from "../components/Titlebar";
import { IdeGrid, ToolPathList } from "../components/ToolSettings";
import { reloadTools, useTools } from "../hooks/useTools";
import { useToolPaths } from "../hooks/useToolPaths";
import type { ToolId, ToolInfo } from "../lib/explorer";
import { type SettingsSnapshot, saveSettings } from "../lib/settings";
import {
  MODULE_DEFS,
  type ModuleId,
  type ModuleState,
  type ProvisionProgress,
  type SetupStateInspection,
  type StepStatus,
  FALLBACK_DISTROS,
  IDE_DOWNLOADS,
  type OnlineDistro,
  assessModule,
  enableWslEngine,
  inspectSetupState,
  installUbuntuDistro,
  linuxUsernameError,
  listOnlineDistros,
  onProvisionProgress,
  runProvisioningTasks,
} from "../lib/setup";
import { BTN_GHOST, BTN_PRIMARY, FIELD, INPUT } from "../lib/ui";
import { errorMessage, isSystemDistro, startDistro } from "../lib/wsl";

interface Props {
  /** First run: the app opens here until the setup is finished or skipped. */
  firstRun: boolean;
  settings: SettingsSnapshot;
  /** Distro selected in the Titlebar (preferred by the inspection). */
  initialDistro: string | null;
  /** A distro was installed/started: the app refreshes its list. */
  onDistrosChanged: () => void;
  onFinish: (saved: SettingsSnapshot) => void;
}

const STEPS = [
  { title: "Diagnóstico", hint: "O que já existe aqui" },
  { title: "Sistema & Distro", hint: "WSL2 e Linux" },
  { title: "Ambiente", hint: "Terminal, zsh, Node" },
  { title: "IDEs", hint: "Editor favorito" },
  { title: "Aplicar", hint: "Progresso ao vivo" },
];

const MAX_LOG = 2000;

type StepRun = { status: StepStatus; message?: string };

export function SetupWizardView({ firstRun, settings, initialDistro, onDistrosChanged, onFinish }: Props) {
  const [step, setStep] = useState(0);
  const [inspection, setInspection] = useState<SetupStateInspection | null>(null);
  const [inspecting, setInspecting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [distro, setDistro] = useState<string | null>(initialDistro);

  const [selected, setSelected] = useState<Set<ModuleId>>(new Set());
  // Once the user touches a checkbox, re-inspections no longer reset the choices.
  const touched = useRef(false);

  const allTools = useTools();
  const [detected, setDetected] = useState(false);
  const [defaultIde, setDefaultIde] = useState<ToolId | null>((settings.defaultIde as ToolId | null) ?? null);
  const toolPaths = useToolPaths(settings.customToolPaths, setError);

  const [runs, setRuns] = useState<Partial<Record<ModuleId, StepRun>>>({});
  const [phase, setPhase] = useState<"idle" | "running" | "finished">("idle");
  const [installRun, setInstallRun] = useState<StepRun | null>(null);
  const [log, setLog] = useState<string[]>([]);
  const [saving, setSaving] = useState(false);
  // First access opens with the animated intro; the machine is inspected underneath meanwhile.
  const [intro, setIntro] = useState(firstRun);
  const endIntro = useCallback(() => setIntro(false), []);

  useEffect(() => {
    reloadTools().then(() => setDetected(true));
  }, []);

  const inspect = useCallback(async (name: string | null) => {
    setInspecting(true);
    try {
      const result = await inspectSetupState(name);
      setInspection(result);
      setDistro(result.distro);
      if (!touched.current) {
        setSelected(new Set(MODULE_DEFS.filter((m) => assessModule(m.id, result).recommended).map((m) => m.id)));
      }
      setError(null);
    } catch (err) {
      setError(errorMessage(err));
    } finally {
      setInspecting(false);
    }
  }, []);

  useEffect(() => {
    void inspect(initialDistro);
  }, [inspect, initialDistro]);

  // Live progress from the backend (modules and the distro install).
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let alive = true;
    onProvisionProgress((p: ProvisionProgress) => {
      if (p.logLine) {
        const line = `[${p.stepId}] ${p.logLine}`;
        setLog((l) => (l.length >= MAX_LOG ? [...l.slice(-MAX_LOG + 1), line] : [...l, line]));
      }
      if (p.logLine && !p.message) return;
      if (p.stepId === "install_distro") {
        setInstallRun({ status: p.status, message: p.message ?? undefined });
      } else {
        setRuns((r) => ({ ...r, [p.stepId]: { status: p.status, message: p.message ?? r[p.stepId as ModuleId]?.message } }));
      }
    })
      .then((fn) => (alive ? (unlisten = fn) : fn()))
      .catch(() => {});
    return () => {
      alive = false;
      unlisten?.();
    };
  }, []);

  const toggle = (id: ModuleId) => {
    touched.current = true;
    setSelected((s) => {
      const next = new Set(s);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  };

  const chosen = MODULE_DEFS.filter((m) => selected.has(m.id));
  const blocked = !inspection?.wslInstalled || inspection.needsReboot || !distro;
  const canRun = chosen.length > 0 && !blocked;

  const apply = async (only?: ModuleId[]) => {
    if (!distro) return;
    const ids = only ?? chosen.map((m) => m.id);
    // The re-inspection afterwards must not swap the list being shown.
    touched.current = true;
    setPhase("running");
    setRuns((r) => ({ ...r, ...Object.fromEntries(ids.map((id) => [id, { status: "pending" as StepStatus }])) }));
    try {
      const results = await runProvisioningTasks(distro, ids);
      setRuns((r) => ({ ...r, ...Object.fromEntries(results.map((x) => [x.stepId, { status: x.status, message: x.message }])) }));
      onDistrosChanged();
      void inspect(distro);
    } catch (err) {
      setError(errorMessage(err));
    } finally {
      setPhase("finished");
    }
  };

  const finish = async () => {
    setSaving(true);
    try {
      const saved = await saveSettings({
        ...settings,
        onboardingCompleted: true,
        setupWizardSeen: true,
        defaultIde,
        customToolPaths: toolPaths.toSave(),
      });
      await reloadTools();
      onFinish(saved);
    } catch (err) {
      setError(errorMessage(err));
      setSaving(false);
    }
  };

  const failed = chosen.filter((m) => runs[m.id]?.status === "error").map((m) => m.id);
  const finishedCount = chosen.filter((m) => ["done", "warning", "error", "skipped"].includes(runs[m.id]?.status ?? "")).length;
  // Diagnostic, system, environment, IDEs, apply.
  const stepReady = [!!inspection && detected, !!inspection && !blocked, true, !toolPaths.invalid, true];
  const LAST = STEPS.length - 1;

  const rescan = () => {
    void inspect(distro);
    setDetected(false);
    reloadTools().then(() => setDetected(true));
  };

  return (
    <div className="h-full bg-void">
    {intro && <SetupIntro onDone={endIntro} />}
    <div className={`flex h-full min-h-0 flex-col bg-zinc-950 ${intro ? "opacity-0" : firstRun ? "setup-enter" : ""}`}>
      {/* Own window chrome: the setup replaces the whole app until it is done. */}
      <div data-tauri-drag-region className="flex h-11 shrink-0 items-center border-b border-[#18181e] bg-void pl-3.5">
        <LogoMark className="pointer-events-none size-[18px] text-zinc-100" />
        <span data-tauri-drag-region className="ml-2.5 text-[13px] font-extrabold tracking-[0.28em] text-zinc-100">
          SYMBIOSIS
        </span>
        <div data-tauri-drag-region className="h-full flex-1" />
        <WindowControls />
      </div>

      <header className="flex items-center gap-3.5 border-b border-zinc-800 bg-void px-8 py-5">
        <div className="min-w-0 flex-1">
          <div className="text-[11px] font-medium tracking-[0.2em] text-zinc-500 uppercase">
            {firstRun ? "Primeiro acesso" : "Configuração inicial"}
          </div>
          <h1 className="mt-0.5 text-lg font-semibold tracking-tight text-zinc-100">Setup Zero-to-Hero</h1>
        </div>
        <button
          type="button"
          onClick={rescan}
          disabled={inspecting || phase === "running"}
          className={BTN_GHOST}
          title="Analisar a máquina de novo"
        >
          {inspecting ? <Loader2 className="size-3.5 animate-spin" /> : <RefreshCw className="size-3.5" />}
          Analisar de novo
        </button>
      </header>

      <div className="flex min-h-0 flex-1">
        <nav className="w-60 shrink-0 space-y-px overflow-y-auto border-r border-zinc-800 bg-void px-3 py-5">
          {STEPS.map((s, i) => {
            const active = i === step;
            const done = i < step;
            return (
              <button
                key={s.title}
                type="button"
                onClick={() => phase !== "running" && setStep(i)}
                className={`relative flex w-full items-center gap-3 rounded-md px-3 py-2.5 text-left transition-colors ${
                  active ? "bg-zinc-900" : "hover:bg-zinc-900/60"
                }`}
              >
                {active && <span className="absolute inset-y-1.5 -left-3 w-0.5 bg-zinc-100" />}
                <span
                  className={`flex size-6 shrink-0 items-center justify-center rounded-full border font-mono text-[11px] ${
                    active ? "border-zinc-100 bg-zinc-100 text-void" : done ? "border-zinc-600 text-zinc-300" : "border-zinc-800 text-zinc-600"
                  }`}
                >
                  {done ? <Check className="size-3" /> : String(i + 1).padStart(2, "0")}
                </span>
                <span className="min-w-0">
                  <span className={`block truncate text-[13px] ${active ? "font-medium text-zinc-100" : "text-zinc-400"}`}>{s.title}</span>
                  <span className="block truncate text-[11px] text-zinc-600">{s.hint}</span>
                </span>
              </button>
            );
          })}
        </nav>

        <div className="min-h-0 flex-1 overflow-y-auto">
          <div className="mx-auto max-w-4xl space-y-6 px-8 py-6">
            {error && (
              <div className="flex items-start gap-2 rounded-lg border border-rose-500/25 bg-rose-500/[0.05] px-3.5 py-2.5 text-xs text-rose-300">
                <AlertTriangle className="mt-px size-3.5 shrink-0" />
                <span className="font-mono break-all whitespace-pre-wrap">{error}</span>
              </div>
            )}
            {step === 0 && (
              <DiagnosticStep inspection={inspection} tools={detected ? allTools : null} />
            )}
            {step === 1 && (
              <SystemStep
                inspection={inspection}
                inspecting={inspecting}
                distro={distro}
                installRun={installRun}
                log={log}
                onSelect={(name) => void inspect(name)}
                onError={setError}
                onChanged={(name) => {
                  onDistrosChanged();
                  void inspect(name);
                }}
              />
            )}
            {step === 2 && (
              <ModulesStep inspection={inspection} distro={distro} selected={selected} onToggle={toggle} />
            )}
            {step === 3 && (
              <>
                <Section index="01" title="IDE favorita" hint="Botão em destaque no Explorador">
                  <IdeGrid tools={detected ? allTools : null} allTools={allTools} value={defaultIde} onChange={setDefaultIde} />
                  <IdeDownloads tools={detected ? allTools : null} onRescan={rescan} />
                </Section>
                <Section index="02" title="Mapeamento de IDEs e ferramentas" hint="Caminho personalizado > detecção automática">
                  <ToolPathList
                    tools={detected ? allTools : null}
                    paths={toolPaths.paths}
                    checks={toolPaths.checks}
                    onChange={toolPaths.setPath}
                    onBrowse={toolPaths.browse}
                  />
                </Section>
              </>
            )}
            {step === 4 && (
              <ApplyStep
                chosen={chosen.map((m) => m.id)}
                runs={runs}
                phase={phase}
                finishedCount={finishedCount}
                log={log}
                blockedReason={
                  !inspection?.wslInstalled
                    ? "Instale o WSL2 na etapa 2 primeiro."
                    : inspection.needsReboot
                      ? "Reinicie o Windows para concluir a instalação do WSL."
                      : !distro
                        ? "Instale ou escolha uma distro Linux na etapa 2."
                        : null
                }
              />
            )}
          </div>
        </div>
      </div>

      <footer className="flex items-center gap-3 border-t border-zinc-800 bg-void px-8 py-3.5">
        <button
          type="button"
          onClick={() => void finish()}
          disabled={saving || phase === "running"}
          className="text-xs text-zinc-500 transition-colors hover:text-zinc-200 disabled:opacity-40"
        >
          {firstRun ? "Pular configuração" : "Sair do setup"}
        </button>
        <span className="flex-1" />
        {step > 0 && (
          <button type="button" onClick={() => setStep(step - 1)} disabled={phase === "running"} className={`${BTN_GHOST} h-8`}>
            <ArrowLeft className="size-3.5" />
            Voltar
          </button>
        )}
        {step < LAST ? (
          <button
            type="button"
            onClick={() => setStep(step + 1)}
            // Step 1 may go on without a distro (the IDEs still get saved).
            disabled={step !== 1 && !stepReady[step]}
            className={`${BTN_PRIMARY} h-8 px-4`}
          >
            {step === 0 && !stepReady[0] ? (
              <>
                <Loader2 className="size-3.5 animate-spin" /> Analisando…
              </>
            ) : (
              <>
                {step === 1 && !stepReady[1] ? "Continuar sem distro" : "Continuar"}
                <ArrowRight className="size-3.5" />
              </>
            )}
          </button>
        ) : phase === "finished" ? (
          <>
            {failed.length > 0 && (
              <button type="button" onClick={() => void apply(failed)} className={`${BTN_GHOST} h-8`}>
                <RefreshCw className="size-3.5" />
                Tentar de novo os que falharam
              </button>
            )}
            <button type="button" onClick={() => void finish()} disabled={saving} className={`${BTN_PRIMARY} h-8 px-4`}>
              {saving ? <Loader2 className="size-3.5 animate-spin" /> : <Check className="size-3.5" />}
              Entrar no Symbiosis
            </button>
          </>
        ) : canRun ? (
          <button type="button" onClick={() => void apply()} disabled={phase === "running"} className={`${BTN_PRIMARY} h-8 px-4`}>
            {phase === "running" ? <Loader2 className="size-3.5 animate-spin" /> : <Play className="size-3.5" />}
            {phase === "running" ? "Aplicando…" : "Aplicar selecionados e entrar no Symbiosis"}
          </button>
        ) : (
          <button type="button" onClick={() => void finish()} disabled={saving} className={`${BTN_PRIMARY} h-8 px-4`}>
            {saving ? <Loader2 className="size-3.5 animate-spin" /> : <SkipForward className="size-3.5" />}
            Pular e ir direto para o app
          </button>
        )}
      </footer>
    </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Step 0: diagnostic (what this machine already has)
// ---------------------------------------------------------------------------

type DiagState = "ok" | "missing" | "warn" | "pending";

function DiagnosticStep({ inspection, tools }: { inspection: SetupStateInspection | null; tools: ToolInfo[] | null }) {
  const tool = (id: ToolId) => tools?.find((t) => t.id === id);
  const toolRow = (id: ToolId, label: string): DiagRowProps => {
    const t = tool(id);
    return {
      label,
      state: !tools ? "pending" : t?.available ? "ok" : "missing",
      detail: !tools ? "detectando…" : t?.available ? (t.path ?? "instalado") : "não encontrado",
      mono: !!t?.available,
    };
  };
  const s = inspection;
  const linuxDistros = s?.distros.filter((d) => !isSystemDistro(d.name)) ?? [];
  const l = s?.linux;
  const ides = (["antigravity", "vscode", "cursor"] as ToolId[]).filter((id) => tool(id)?.available).length;

  const windows: DiagRowProps[] = [
    {
      label: "WSL2",
      state: !s ? "pending" : s.needsReboot ? "warn" : s.wslInstalled ? "ok" : "missing",
      detail: !s ? "verificando…" : s.needsReboot ? "aguardando reinício do Windows" : s.wslInstalled ? `versão ${s.wslVersion ?? "?"}` : "não instalado",
      mono: !!s?.wslVersion,
    },
    {
      label: "Distro Linux",
      state: !s ? "pending" : linuxDistros.length ? "ok" : "missing",
      detail: !s
        ? "verificando…"
        : linuxDistros.length
          ? linuxDistros.map((d) => d.name).join(", ")
          : s.distros.length
            ? `nenhuma de desenvolvimento (só ${s.distros.map((d) => d.name).join(", ")})`
            : "nenhuma instalada",
      mono: linuxDistros.length > 0,
    },
    {
      label: "Windows Terminal",
      state: !s ? "pending" : s.windowsTerminal.installed ? "ok" : "missing",
      detail: !s ? "verificando…" : s.windowsTerminal.installed ? "instalado" : "não encontrado",
    },
    toolRow("antigravity", "Antigravity"),
    toolRow("vscode", "VS Code"),
    toolRow("cursor", "Cursor"),
  ];

  const linux: DiagRowProps[] = !s
    ? []
    : !l
      ? [
          {
            label: s.distro ?? "Linux",
            state: "warn",
            detail: s.distro ? "distro parada: analisada na próxima etapa" : "sem distro para analisar",
          },
        ]
      : [
          {
            label: "Zsh + Oh-My-Zsh",
            state: l.zshInstalled && l.ohMyZshInstalled ? "ok" : "missing",
            detail: [l.zshInstalled ? "zsh" : "sem zsh", l.ohMyZshInstalled ? "Oh-My-Zsh" : "sem Oh-My-Zsh", l.zshIsDefaultShell ? "shell padrão" : null]
              .filter(Boolean)
              .join(" · "),
          },
          {
            label: "Tema do zsh",
            state: l.zshTheme ? "ok" : "missing",
            detail: l.zshTheme ?? "nenhum",
            mono: !!l.zshTheme,
          },
          {
            label: "Plugins zsh",
            state: l.zshPlugins.autosuggestions && l.zshPlugins.syntaxHighlighting && l.zshPlugins.completions ? "ok" : "missing",
            detail: [
              l.zshPlugins.autosuggestions && "autosuggestions",
              l.zshPlugins.syntaxHighlighting && "syntax-highlighting",
              l.zshPlugins.completions && "completions",
            ]
              .filter(Boolean)
              .join(" · ") || "nenhum",
          },
          {
            label: "NVM + Node.js",
            state: l.nvmInstalled && l.nodeLtsInstalled ? "ok" : "missing",
            detail: l.nvmInstalled ? (l.nodeLtsInstalled ? "nvm + Node padrão" : "nvm sem Node") : "não instalado",
          },
        ];

  const all = [...windows, ...linux];
  const done = !!s && !!tools;
  const ok = all.filter((r) => r.state === "ok").length;

  return (
    <>
      {/* Container query: the mark shrinks in narrow windows and disappears when there's no room. */}
      <div className="@container">
        <div className="flex items-center gap-5 @max-2xl:gap-4">
          <div className="flex size-16 shrink-0 items-center justify-center rounded-xl border border-zinc-800 bg-void @max-2xl:size-12 @max-2xl:rounded-lg @max-md:hidden">
            <LogoMark className="size-9 text-zinc-100 @max-2xl:size-7" />
          </div>
          <div className="min-w-0">
            <h2 className="text-xl font-semibold tracking-tight text-zinc-100 @max-md:text-lg">Bem-vindo ao WSL Symbiosis</h2>
            <p className="mt-1.5 max-w-2xl text-sm leading-relaxed text-zinc-500">
              Antes de tudo, um raio-x desta máquina: o que já está pronto e o que falta para ter o WSL2, o terminal e o
              seu editor funcionando juntos. Nada é alterado nesta etapa.
            </p>
          </div>
        </div>
      </div>

      <Section index="01" title="Windows" hint={ides > 0 ? `${ides} IDE${ides > 1 ? "s" : ""} encontrada${ides > 1 ? "s" : ""}` : undefined}>
        <DiagList rows={windows} />
      </Section>

      <Section index="02" title={`Ambiente Linux${s?.distro ? ` · ${s.distro}` : ""}`}>
        <DiagList rows={linux.length ? linux : [{ label: "Linux", state: "pending", detail: "verificando…" }]} />
      </Section>

      <div className="flex items-center gap-2 text-xs text-zinc-500">
        {done ? (
          <>
            <Check className="size-3.5 text-zinc-300" />
            <span>
              <span className="font-mono text-zinc-200">
                {ok}/{all.length}
              </span>{" "}
              itens prontos. Nas próximas etapas você escolhe o que instalar ou configurar.
            </span>
          </>
        ) : (
          <>
            <Loader2 className="size-3.5 animate-spin" /> Analisando a máquina…
          </>
        )}
      </div>
    </>
  );
}

interface DiagRowProps {
  label: string;
  state: DiagState;
  detail: string;
  mono?: boolean;
}

function DiagList({ rows }: { rows: DiagRowProps[] }) {
  return (
    <div className="overflow-hidden rounded-lg border border-zinc-800 bg-zinc-900">
      {rows.map((r) => (
        <div key={r.label} className="flex items-center gap-3 border-b border-zinc-800 px-4 py-2.5 last:border-0">
          {r.state === "pending" ? (
            <Loader2 className="size-3.5 shrink-0 animate-spin text-zinc-500" />
          ) : (
            <span
              className={`size-1.5 shrink-0 rounded-full ${
                r.state === "ok" ? "bg-emerald-400" : r.state === "warn" ? "bg-amber-400" : "bg-zinc-600"
              }`}
            />
          )}
          <span className="w-44 shrink-0 text-sm text-zinc-200">{r.label}</span>
          <span className={`min-w-0 flex-1 truncate text-right text-xs ${r.mono ? "font-mono" : ""} ${r.state === "ok" ? "text-zinc-400" : "text-zinc-500"}`} title={r.detail}>
            {r.detail}
          </span>
        </div>
      ))}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Step 3 (extra): IDE downloads
// ---------------------------------------------------------------------------

const IDE_LABELS: Record<keyof typeof IDE_DOWNLOADS, string> = { antigravity: "Antigravity", vscode: "VS Code", cursor: "Cursor" };

function IdeDownloads({ tools, onRescan }: { tools: ToolInfo[] | null; onRescan: () => void }) {
  if (!tools) return null;
  const missing = (Object.keys(IDE_DOWNLOADS) as (keyof typeof IDE_DOWNLOADS)[]).filter(
    (id) => !tools.find((t) => t.id === id)?.available,
  );
  const none = missing.length === 3;
  return (
    <div className="mt-3 space-y-2.5">
      <p className="text-[11px] leading-relaxed text-zinc-500">
        Você só precisa de <span className="text-zinc-300">uma</span> dessas IDEs: o Symbiosis abre seus projetos do WSL na
        que estiver instalada.
      </p>
      {none && (
        <Notice tone="amber">Nenhuma IDE encontrada nesta máquina. Instale uma das três abaixo e clique em "Detectar de novo".</Notice>
      )}
      {missing.length > 0 && (
        <div className="flex flex-wrap items-center gap-2">
          {missing.map((id) => (
            <button
              key={id}
              type="button"
              onClick={() => void openUrl(IDE_DOWNLOADS[id]).catch(() => {})}
              className={BTN_GHOST}
              title={IDE_DOWNLOADS[id]}
            >
              <ExternalLink className="size-3.5" />
              Baixar {IDE_LABELS[id]}
            </button>
          ))}
          <button type="button" onClick={onRescan} className="ml-auto flex items-center gap-1.5 text-xs text-zinc-500 hover:text-zinc-200">
            <RefreshCw className="size-3.5" /> Detectar de novo
          </button>
        </div>
      )}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Step 1: system & distro
// ---------------------------------------------------------------------------

function SystemStep({
  inspection,
  inspecting,
  distro,
  installRun,
  log,
  onSelect,
  onError,
  onChanged,
}: {
  inspection: SetupStateInspection | null;
  inspecting: boolean;
  distro: string | null;
  installRun: StepRun | null;
  log: string[];
  onSelect: (name: string) => void;
  onError: (message: string | null) => void;
  onChanged: (distro: string | null) => void;
}) {
  const [enabling, setEnabling] = useState(false);
  const [enableMessage, setEnableMessage] = useState<string | null>(null);
  const [starting, setStarting] = useState(false);
  const [showInstall, setShowInstall] = useState(false);

  if (!inspection) {
    return (
      <div className="flex h-48 items-center justify-center gap-2 text-xs text-zinc-500">
        <Loader2 className="size-4 animate-spin" /> Analisando a máquina…
      </div>
    );
  }

  const linuxDistros = inspection.distros.filter((d) => !isSystemDistro(d.name));
  const systemDistros = inspection.distros.filter((d) => isSystemDistro(d.name));
  const selectedDistro = linuxDistros.find((d) => d.name === distro);
  const installOpen = showInstall || (inspection.wslInstalled && !inspection.needsReboot && !inspection.hasLinuxDistro);

  const enable = async () => {
    setEnabling(true);
    onError(null);
    try {
      const r = await enableWslEngine();
      setEnableMessage(r.message);
      onChanged(null);
    } catch (err) {
      onError(errorMessage(err));
    } finally {
      setEnabling(false);
    }
  };

  const start = async () => {
    if (!distro) return;
    setStarting(true);
    try {
      await startDistro(distro);
      onChanged(distro);
    } catch (err) {
      onError(errorMessage(err));
    } finally {
      setStarting(false);
    }
  };

  return (
    <>
      <Section index="01" title="Motor WSL2" hint="Recursos do Windows">
        <div className="grid grid-cols-3 divide-x divide-zinc-800 rounded-lg border border-zinc-800 bg-zinc-900">
          <Cell label="Status">
            <span className="flex items-center gap-2">
              <span
                className={`size-1.5 rounded-full ${
                  inspection.needsReboot ? "bg-amber-400" : inspection.wslInstalled ? "bg-emerald-400" : "bg-rose-400"
                }`}
              />
              {inspection.needsReboot ? "Aguardando reinício" : inspection.wslInstalled ? "Instalado" : "Não instalado"}
            </span>
          </Cell>
          <Cell label="Versão" mono>
            {inspection.wslVersion ?? "—"}
          </Cell>
          <Cell label="Distros Linux" mono>
            {linuxDistros.length}
          </Cell>
        </div>

        {inspection.needsReboot && (
          <Notice tone="amber">
            Os recursos do WSL foram habilitados, falta <strong className="text-amber-200">reiniciar o Windows</strong>. Depois
            abra o WSL Symbiosis: o setup continua daqui.
          </Notice>
        )}
        {!inspection.wslInstalled && !inspection.needsReboot && (
          <div className="mt-2.5 rounded-lg border border-zinc-800 bg-zinc-900 p-4">
            <div className="flex items-start gap-3">
              <ShieldCheck className="mt-0.5 size-4 shrink-0 text-zinc-400" />
              <div className="min-w-0 flex-1">
                <div className="text-sm font-semibold text-zinc-100">Habilitar o WSL2 automaticamente</div>
                <p className="mt-1 text-xs leading-relaxed text-zinc-500">
                  Ativa "Subsistema do Windows para Linux" e "Plataforma de Máquina Virtual" (DISM) e define o WSL 2 como
                  padrão. O Windows vai pedir permissão de administrador uma vez e, em geral, um reinício.
                </p>
                {enableMessage && <p className="mt-2 text-xs text-zinc-300">{enableMessage}</p>}
              </div>
              <button type="button" onClick={() => void enable()} disabled={enabling} className={`${BTN_PRIMARY} h-8 shrink-0 px-4`}>
                {enabling ? <Loader2 className="size-3.5 animate-spin" /> : <Power className="size-3.5" />}
                {enabling ? "Habilitando…" : "Habilitar WSL2"}
              </button>
            </div>
          </div>
        )}
      </Section>

      <Section index="02" title="Distro Linux" hint="Onde o ambiente será configurado">
        {inspection.wslInstalled && linuxDistros.length === 0 && systemDistros.length > 0 && (
          <Notice tone="amber">
            Detectamos o WSL2 (com{" "}
            {systemDistros.some((d) => /^docker/i.test(d.name)) ? "Docker" : systemDistros.map((d) => d.name).join(", ")}), mas
            nenhuma distribuição Linux de desenvolvimento (como Ubuntu) está instalada. Instale o Ubuntu abaixo: ele é onde o
            seu ambiente vai rodar.
          </Notice>
        )}
        {linuxDistros.length > 0 && (
          <div className="overflow-hidden rounded-lg border border-zinc-800" role="radiogroup">
            {linuxDistros.map((d) => {
              const active = d.name === distro;
              return (
                <button
                  key={d.name}
                  type="button"
                  role="radio"
                  aria-checked={active}
                  onClick={() => onSelect(d.name)}
                  disabled={inspecting}
                  className={`flex w-full items-center gap-3 border-b border-zinc-800 px-4 py-3 text-left text-sm transition-colors last:border-0 ${
                    active ? "bg-zinc-900 text-zinc-100" : "text-zinc-400 hover:bg-zinc-900/60 hover:text-zinc-200"
                  }`}
                >
                  <span
                    className={`flex size-3.5 shrink-0 items-center justify-center rounded-full border ${
                      active ? "border-zinc-100" : "border-zinc-600"
                    }`}
                  >
                    {active && <span className="size-1.5 rounded-full bg-zinc-100" />}
                  </span>
                  <span className={`size-1.5 rounded-full ${d.state === "Running" ? "bg-emerald-400" : "bg-zinc-600"}`} />
                  <span className="font-medium">{d.name}</span>
                  <span className="font-mono text-[11px] text-zinc-500">
                    {d.stateLabel} · WSL {d.version}
                  </span>
                  {active && (
                    <span className="ml-auto inline-flex items-center gap-1 rounded-full bg-zinc-100 px-2 py-0.5 text-[11px] font-semibold text-void">
                      <Check className="size-3" /> Pronto
                    </span>
                  )}
                </button>
              );
            })}
          </div>
        )}

        {selectedDistro && !inspection.distroRunning && (
          <Notice tone="zinc">
            <span className="flex items-center gap-3">
              <span className="flex-1">
                {selectedDistro.name} está parada. Ligue-a para analisar o zsh, os plugins e o Node (a análise nunca liga a
                distro sozinha).
              </span>
              <button type="button" onClick={() => void start()} disabled={starting} className={`${BTN_GHOST} shrink-0`}>
                {starting ? <Loader2 className="size-3.5 animate-spin" /> : <Play className="size-3.5" />}
                Ligar e analisar
              </button>
            </span>
          </Notice>
        )}

        {inspection.hasLinuxDistro && !showInstall && (
          <button type="button" onClick={() => setShowInstall(true)} className="mt-2.5 flex items-center gap-1.5 text-xs text-zinc-500 hover:text-zinc-200">
            <ChevronDown className="size-3.5" /> Instalar outra distro (Ubuntu/Debian)
          </button>
        )}
        {(installOpen || !inspection.hasLinuxDistro) && (
          <InstallDistroForm
            disabled={!inspection.wslInstalled || inspection.needsReboot}
            installed={inspection.distros.map((d) => d.name)}
            run={installRun}
            log={log.filter((l) => l.startsWith("[install_distro]"))}
            onError={onError}
            onInstalled={(name) => onChanged(name)}
          />
        )}
      </Section>
    </>
  );
}

function InstallDistroForm({
  disabled,
  installed,
  run,
  log,
  onError,
  onInstalled,
}: {
  disabled: boolean;
  installed: string[];
  run: StepRun | null;
  log: string[];
  onError: (message: string | null) => void;
  onInstalled: (distro: string) => void;
}) {
  const [options, setOptions] = useState<OnlineDistro[] | null>(null);
  const [name, setName] = useState("Ubuntu");
  const [user, setUser] = useState("");

  useEffect(() => {
    if (disabled) return;
    let alive = true;
    listOnlineDistros()
      .catch(() => FALLBACK_DISTROS)
      .then((list) => {
        if (!alive) return;
        setOptions(list);
        setName(list.find((o) => !installed.includes(o.name))?.name ?? list[0].name);
      });
    return () => {
      alive = false;
    };
    // The catalog is read once per form (it is a network call).
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [disabled]);
  const [password, setPassword] = useState("");
  const [confirm, setConfirm] = useState("");
  const [busy, setBusy] = useState(false);

  const userError = linuxUsernameError(user);
  const mismatch = confirm.length > 0 && confirm !== password;
  const ready = !disabled && !busy && user.length > 0 && !userError && password.length > 0 && password === confirm;

  const install = async () => {
    setBusy(true);
    onError(null);
    try {
      await installUbuntuDistro(name, user, password);
      // Leave it running so the next step can inspect it right away (a stopped distro is never
      // booted by the inspection, and the modules would show up as "Não analisado").
      await startDistro(name).catch(() => {});
      setPassword("");
      setConfirm("");
      onInstalled(name);
    } catch (err) {
      onError(errorMessage(err));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className={`mt-2.5 rounded-lg border border-zinc-800 bg-zinc-900 p-4 ${disabled ? "opacity-50" : ""}`}>
      <div className="flex items-center gap-2">
        <Download className="size-4 text-zinc-400" />
        <span className="text-sm font-semibold text-zinc-100">Instalar Linux automaticamente</span>
      </div>

      <div className="mt-3 flex flex-wrap items-center gap-1.5" role="radiogroup">
        {!options && !disabled && (
          <span className="flex items-center gap-1.5 text-[11px] text-zinc-500">
            <Loader2 className="size-3 animate-spin" /> Lendo o catálogo do WSL…
          </span>
        )}
        {(options ?? FALLBACK_DISTROS).map((o) => {
          const active = o.name === name;
          const exists = installed.includes(o.name);
          return (
            <button
              key={o.name}
              type="button"
              role="radio"
              aria-checked={active}
              disabled={disabled || busy || exists || !options}
              onClick={() => setName(o.name)}
              className={`rounded-full px-3 py-1 text-xs transition-colors disabled:cursor-not-allowed ${
                active ? "bg-zinc-100 font-semibold text-void" : "border border-zinc-800 text-zinc-400 hover:border-zinc-600 disabled:opacity-40"
              }`}
              title={exists ? "Já instalada" : o.name}
            >
              {o.friendlyName}
            </button>
          );
        })}
      </div>

      <div className="mt-3 grid grid-cols-3 gap-2">
        <Field label="Usuário Linux" error={userError}>
          <input
            value={user}
            onChange={(e) => setUser(e.target.value.trim())}
            disabled={disabled || busy}
            spellCheck={false}
            autoComplete="off"
            placeholder="ex.: kyo"
            className={`${INPUT} font-mono`}
          />
        </Field>
        <Field label="Senha (sudo)">
          <PasswordInput value={password} onChange={setPassword} disabled={disabled || busy} />
        </Field>
        <Field label="Confirmar senha" error={mismatch ? "As senhas não conferem." : null}>
          <PasswordInput value={confirm} onChange={setConfirm} disabled={disabled || busy} />
        </Field>
      </div>

      <div className="mt-3 flex items-center gap-3">
        <p className="flex-1 text-[11px] leading-relaxed text-zinc-500">
          A senha vai direto para o <span className="font-mono">chpasswd</span> dentro da distro e não fica salva no app. O
          usuário vira o padrão da distro (<span className="font-mono">/etc/wsl.conf</span>).
        </p>
        <button type="button" onClick={() => void install()} disabled={!ready} className={`${BTN_PRIMARY} h-8 shrink-0 px-4`}>
          {busy ? <Loader2 className="size-3.5 animate-spin" /> : <Download className="size-3.5" />}
          {busy ? "Instalando…" : `Instalar ${name}`}
        </button>
      </div>

      {(busy || run) && (
        <div className="mt-3 space-y-2">
          {run?.message && (
            <div className="flex items-center gap-2 text-xs">
              <StatusDot status={run.status} />
              <span className="text-zinc-300">{run.message}</span>
            </div>
          )}
          <LogPanel lines={log} defaultOpen />
        </div>
      )}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Step 2: modules
// ---------------------------------------------------------------------------

const STATE_TAG: Record<ModuleState, { label: string; className: string; dot?: string }> = {
  configured: { label: "Já configurado", className: "border-zinc-800 text-zinc-400", dot: "bg-emerald-400" },
  missing: { label: "Recomendado", className: "border-zinc-600 text-zinc-100" },
  unknown: { label: "Não analisado", className: "border-zinc-800 text-zinc-500" },
  unavailable: { label: "Indisponível", className: "border-zinc-800 text-zinc-600" },
};
const OPTIONAL_TAG = { label: "Opcional", className: "border-zinc-800 text-zinc-400" } as { label: string; className: string; dot?: string };

function ModulesStep({
  inspection,
  distro,
  selected,
  onToggle,
}: {
  inspection: SetupStateInspection | null;
  distro: string | null;
  selected: Set<ModuleId>;
  onToggle: (id: ModuleId) => void;
}) {
  return (
    <Section index="01" title="Checklist do ambiente" hint={distro ? `Distro: ${distro}` : "Nenhuma distro selecionada"}>
      <div className="grid gap-2.5">
        {MODULE_DEFS.map((m) => {
          const a = assessModule(m.id, inspection);
          // Missing but not recommended (e.g. would replace another zsh theme): optional.
          const tag = a.state === "missing" && !a.recommended ? OPTIONAL_TAG : STATE_TAG[a.state];
          // A configured module can still be re-run (repair); unavailable ones can't run at all.
          const disabled = a.state === "unavailable";
          const checked = selected.has(m.id) && !disabled;
          return (
            <button
              key={m.id}
              type="button"
              role="checkbox"
              aria-checked={checked}
              disabled={disabled}
              onClick={() => onToggle(m.id)}
              className={`flex items-start gap-4 rounded-lg border px-4 py-3.5 text-left transition-colors disabled:cursor-not-allowed disabled:opacity-45 ${
                checked ? "border-zinc-500 bg-zinc-900" : "border-zinc-800 bg-zinc-900/50 hover:border-zinc-700"
              }`}
            >
              <span
                className={`mt-0.5 flex size-5 shrink-0 items-center justify-center rounded border transition-colors ${
                  checked ? "border-zinc-100 bg-zinc-100 text-void" : "border-zinc-600"
                }`}
              >
                {checked && <Check className="size-3.5" strokeWidth={3} />}
              </span>
              <span className="min-w-0 flex-1">
                <span className="flex items-center gap-2.5">
                  {m.linux ? (
                    <span className="font-mono text-[10px] tracking-wider text-zinc-600 uppercase">Linux</span>
                  ) : (
                    <TerminalSquare className="size-3.5 text-zinc-500" />
                  )}
                  <span className="text-sm font-semibold text-zinc-100">{m.title}</span>
                  <span className={`ml-auto inline-flex shrink-0 items-center gap-1.5 rounded-full border px-2 py-0.5 text-[11px] ${tag.className}`}>
                    {tag.dot && <span className={`size-1.5 rounded-full ${tag.dot}`} />}
                    {tag.label}
                  </span>
                </span>
                <span className="mt-1 block text-xs leading-relaxed text-zinc-500">{m.detail}</span>
                {a.note && (
                  <span className={`mt-1.5 flex items-center gap-1.5 text-[11px] ${a.state === "missing" ? "text-amber-300" : "text-zinc-500"}`}>
                    {a.state === "missing" && <AlertTriangle className="size-3" />}
                    {a.note}
                  </span>
                )}
              </span>
            </button>
          );
        })}
      </div>
      <p className="mt-3 text-[11px] leading-relaxed text-zinc-500">
        Tudo é idempotente: o que já existe é mantido, e cada arquivo alterado ganha um backup{" "}
        <span className="font-mono">.symbiosis.bak</span> na primeira vez (~/.zshrc e o settings.json do Windows Terminal).
      </p>
    </Section>
  );
}

// ---------------------------------------------------------------------------
// Step 4: apply
// ---------------------------------------------------------------------------

function ApplyStep({
  chosen,
  runs,
  phase,
  finishedCount,
  log,
  blockedReason,
}: {
  chosen: ModuleId[];
  runs: Partial<Record<ModuleId, StepRun>>;
  phase: "idle" | "running" | "finished";
  finishedCount: number;
  log: string[];
  blockedReason: string | null;
}) {
  const total = chosen.length;
  const pct = total > 0 ? (finishedCount / total) * 100 : 0;

  if (total === 0) {
    return (
      <Section index="01" title="Aplicar" hint="Nada marcado">
        <div className="rounded-lg border border-zinc-800 bg-zinc-900 px-4 py-6 text-center text-sm text-zinc-400">
          Nenhum módulo marcado. Suas escolhas de IDE são salvas ao entrar no app.
        </div>
      </Section>
    );
  }

  return (
    <Section index="01" title="Aplicar" hint={`${finishedCount} de ${total} concluídos`}>
      {blockedReason && phase === "idle" && <Notice tone="amber">{blockedReason}</Notice>}
      <div className="overflow-hidden rounded-lg border border-zinc-800 bg-zinc-900">
        {chosen.map((id) => {
          const def = MODULE_DEFS.find((m) => m.id === id)!;
          const run = runs[id];
          return (
            <div key={id} className="flex items-start gap-3 border-b border-zinc-800 px-4 py-3 last:border-0">
              <span className="mt-1.5">
                <StatusDot status={run?.status ?? "pending"} />
              </span>
              <span className="min-w-0 flex-1">
                <span className="block text-sm font-medium text-zinc-100">{def.title}</span>
                <span
                  className={`mt-0.5 block text-xs whitespace-pre-wrap ${
                    run?.status === "error" ? "text-rose-300" : run?.status === "warning" ? "text-amber-300" : "text-zinc-500"
                  }`}
                >
                  {run?.message ?? (phase === "idle" ? "Aguardando" : "Na fila")}
                </span>
              </span>
            </div>
          );
        })}
      </div>

      <div className="mt-4">
        <div className="mb-2 flex items-center text-[11px] tracking-wider text-zinc-500 uppercase">
          Progresso
          <span className="ml-auto font-mono tabular-nums">{pct.toFixed(0)}%</span>
        </div>
        <div className="meter-cells h-2 overflow-hidden bg-zinc-800" role="progressbar" aria-valuemin={0} aria-valuemax={100} aria-valuenow={pct}>
          <div className="h-full bg-zinc-100 transition-[width] duration-500" style={{ width: `${pct}%` }} />
        </div>
      </div>

      <div className="mt-4">
        <LogPanel lines={log} defaultOpen={phase !== "idle"} />
      </div>
    </Section>
  );
}

// ---------------------------------------------------------------------------
// Bits
// ---------------------------------------------------------------------------

function StatusDot({ status }: { status: StepStatus }) {
  if (status === "running") return <Loader2 className="size-3.5 animate-spin text-zinc-100" />;
  const color =
    status === "done"
      ? "bg-emerald-400"
      : status === "warning"
        ? "bg-amber-400"
        : status === "error"
          ? "bg-rose-400"
          : "bg-zinc-600";
  return <span className={`block size-1.5 rounded-full ${color}`} />;
}

function LogPanel({ lines, defaultOpen }: { lines: string[]; defaultOpen?: boolean }) {
  const [open, setOpen] = useState(!!defaultOpen);
  const ref = useRef<HTMLPreElement>(null);
  useEffect(() => {
    if (defaultOpen) setOpen(true);
  }, [defaultOpen]);
  // Follow the tail while running.
  useEffect(() => {
    const el = ref.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [lines, open]);
  const text = useMemo(() => lines.join("\n"), [lines]);

  return (
    <div className="rounded-lg border border-zinc-800 bg-void">
      <button
        type="button"
        onClick={() => setOpen((o) => !o)}
        className="flex w-full items-center gap-2 px-3 py-2 text-[11px] tracking-wider text-zinc-500 uppercase hover:text-zinc-300"
      >
        <ChevronDown className={`size-3.5 transition-transform ${open ? "" : "-rotate-90"}`} />
        Log técnico
        <span className="ml-auto font-mono normal-case">{lines.length} linhas</span>
      </button>
      {open && (
        <pre
          ref={ref}
          className="max-h-64 overflow-auto border-t border-zinc-800 px-3 py-2 font-mono text-[11px] leading-relaxed whitespace-pre-wrap text-zinc-400"
        >
          {text || "Sem saída ainda."}
        </pre>
      )}
    </div>
  );
}

function Section({ index, title, hint, children }: { index: string; title: string; hint?: string; children: ReactNode }) {
  return (
    <section>
      <div className="mb-2.5 flex items-baseline gap-2.5">
        <span className="font-mono text-[11px] text-zinc-600">{index}</span>
        <h3 className="text-[11px] font-semibold tracking-wider text-zinc-300 uppercase">{title}</h3>
        {hint && <span className="ml-auto text-[11px] text-zinc-600">{hint}</span>}
      </div>
      {children}
    </section>
  );
}

function Cell({ label, mono, children }: { label: string; mono?: boolean; children: ReactNode }) {
  return (
    <div className="min-w-0 px-4 py-3">
      <div className="text-[10px] tracking-wider text-zinc-500 uppercase">{label}</div>
      <div className={`mt-1 truncate text-sm text-zinc-100 ${mono ? "font-mono" : ""}`}>{children}</div>
    </div>
  );
}

/** Password with a show/hide toggle (eye). */
function PasswordInput({ value, onChange, disabled }: { value: string; onChange: (v: string) => void; disabled?: boolean }) {
  const [visible, setVisible] = useState(false);
  return (
    <>
      <input
        type={visible ? "text" : "password"}
        value={value}
        onChange={(e) => onChange(e.target.value)}
        disabled={disabled}
        autoComplete="new-password"
        spellCheck={false}
        className={`${INPUT} ${visible ? "font-mono" : ""}`}
      />
      <button
        type="button"
        onClick={() => setVisible((v) => !v)}
        disabled={disabled}
        aria-label={visible ? "Ocultar senha" : "Mostrar senha"}
        title={visible ? "Ocultar senha" : "Mostrar senha"}
        className="shrink-0 text-zinc-500 transition-colors hover:text-zinc-200 disabled:opacity-40"
      >
        {visible ? <EyeOff className="size-3.5" /> : <Eye className="size-3.5" />}
      </button>
    </>
  );
}

function Field({ label, error, children }: { label: string; error?: string | null; children: ReactNode }) {
  return (
    <label className="block min-w-0">
      <span className="mb-1 block text-[10px] tracking-wider text-zinc-500 uppercase">{label}</span>
      <span className={`${FIELD} ${error ? "border-rose-500/50" : ""}`}>{children}</span>
      {error && <span className="mt-1 block text-[11px] text-rose-300">{error}</span>}
    </label>
  );
}

function Notice({ tone, children }: { tone: "amber" | "zinc"; children: ReactNode }) {
  return (
    <div
      className={`mt-2.5 rounded-lg border px-3.5 py-2.5 text-xs leading-relaxed ${
        tone === "amber" ? "border-amber-500/25 bg-amber-500/[0.05] text-amber-300" : "border-zinc-800 bg-zinc-900 text-zinc-400"
      }`}
    >
      {children}
    </div>
  );
}
