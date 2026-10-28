import { useEffect, useState, type ReactNode } from "react";
import { useUpdaterContext } from "../hooks/useUpdater";
import { AlertTriangle, ArrowUpCircle, Check, GitBranch, Keyboard, Loader2, MonitorCog, RefreshCw, Star, Wand2, X } from "lucide-react";
import { GitSettings } from "./GitSettings";
import type { Distro, WslInfo } from "../lib/wsl";
import { errorMessage, getWslInfo } from "../lib/wsl";
import type { ToolId } from "../lib/explorer";
import { type GitAuth, SettingsSnapshot, saveSettings } from "../lib/settings";
import { reloadTools, useTools } from "../hooks/useTools";
import { useToolPaths } from "../hooks/useToolPaths";
import { BTN_GHOST, BTN_PRIMARY } from "../lib/ui";
import { LogoMark } from "./Logo";
import { StatusDot } from "./StatusDot";
import { IdeGrid, ToolPathList } from "./ToolSettings";
import { ShortcutsSettings } from "./ShortcutsSettings";
import { SHORTCUT_DEFS, type ShortcutId, type Shortcuts, resolveShortcuts } from "../lib/shortcuts";

interface Props {
  /** First run: explains itself and finishing marks the onboarding as done. */
  onboarding: boolean;
  settings: SettingsSnapshot;
  distros: Distro[];
  info: WslInfo | null;
  onSaved: (settings: SettingsSnapshot) => void;
  onClose: () => void;
  /** Reopens the full-screen first-run setup. */
  onOpenSetup: () => void;
}

type Tab = "system" | "git" | "shortcuts";

const TABS: { id: Tab; label: string; icon: typeof Keyboard }[] = [
  { id: "system", label: "Sistema & IDEs", icon: MonitorCog },
  { id: "git", label: "Git", icon: GitBranch },
  { id: "shortcuts", label: "Atalhos", icon: Keyboard },
];

/** Key bindings shared by more than one action (mouse gestures act on different targets). */
function findConflicts(shortcuts: Shortcuts): Set<ShortcutId> {
  const byBinding = new Map<string, ShortcutId[]>();
  for (const def of SHORTCUT_DEFS) {
    const binding = shortcuts[def.id];
    if (def.kind !== "key" || !binding) continue;
    byBinding.set(binding, [...(byBinding.get(binding) ?? []), def.id]);
  }
  return new Set([...byBinding.values()].filter((ids) => ids.length > 1).flat());
}

export function SettingsDialog({ onboarding, settings, distros, info: initialInfo, onSaved, onClose, onOpenSetup }: Props) {
  const [info, setInfo] = useState(initialInfo);
  const [checkingWsl, setCheckingWsl] = useState(false);
  const allTools = useTools();
  const [detected, setDetected] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const toolPaths = useToolPaths(settings.customToolPaths, setError);
  const [defaultDistro, setDefaultDistro] = useState<string | null>(settings.defaultDistro);
  const [defaultIde, setDefaultIde] = useState<ToolId | null>((settings.defaultIde as ToolId | null) ?? null);
  const [shortcuts, setShortcuts] = useState<Shortcuts>(() => resolveShortcuts(settings.shortcuts));
  const [gitAuth, setGitAuth] = useState<GitAuth>(settings.gitAuth ?? "auto");
  const [tab, setTab] = useState<Tab>("system");
  const [saving, setSaving] = useState(false);

  // Fresh detection every time the screen opens (an IDE may have been installed meanwhile).
  useEffect(() => {
    reloadTools().then(() => setDetected(true));
  }, []);
  const tools = detected ? allTools : null;

  useEffect(() => {
    // The update dialog may open on top of this one and handles its own Escape first.
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && !e.defaultPrevented && onClose();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  const recheckWsl = async () => {
    setCheckingWsl(true);
    try {
      setInfo(await getWslInfo());
    } catch (err) {
      setError(errorMessage(err));
    } finally {
      setCheckingWsl(false);
    }
  };

  const conflicts = findConflicts(shortcuts);
  const invalid = toolPaths.invalid || conflicts.size > 0;

  const save = async () => {
    setSaving(true);
    setError(null);
    try {
      const customToolPaths = toolPaths.toSave();
      // Only what differs from the defaults, so future default changes still reach the user.
      const changedShortcuts = Object.fromEntries(
        SHORTCUT_DEFS.filter((d) => shortcuts[d.id] !== d.default).map((d) => [d.id, shortcuts[d.id]]),
      );
      const saved = await saveSettings({
        onboardingCompleted: true,
        customToolPaths,
        defaultDistro,
        defaultIde,
        shortcuts: changedShortcuts,
        gitAuth,
      });
      await reloadTools();
      onSaved(saved);
    } catch (err) {
      setError(errorMessage(err));
    } finally {
      setSaving(false);
    }
  };

  const wslOk = !!info?.wslVersion;
  const wslDefault = distros.find((d) => d.isDefault);

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/70 p-6" onMouseDown={onClose}>
      <div
        role="dialog"
        aria-modal
        aria-label="Diagnóstico do Sistema & Configurações"
        onMouseDown={(e) => e.stopPropagation()}
        className="flex h-[min(780px,100%)] w-[min(820px,100%)] flex-col overflow-hidden rounded-xl border border-zinc-800 bg-zinc-950 shadow-2xl shadow-black"
      >
        <header className="flex items-start gap-3.5 border-b border-zinc-800 bg-void px-6 py-5">
          <div className="flex size-10 shrink-0 items-center justify-center rounded-lg border border-zinc-800 bg-zinc-950">
            <LogoMark className="size-6 text-zinc-100" />
          </div>
          <div className="min-w-0 flex-1">
            <div className="text-[11px] font-medium tracking-[0.2em] text-zinc-500 uppercase">
              {onboarding ? "Primeiro uso" : "Sistema"}
            </div>
            <h2 className="mt-0.5 text-lg font-semibold tracking-tight text-zinc-100">
              Diagnóstico do Sistema & Configurações
            </h2>
            <p className="mt-1 text-xs text-zinc-500">
              {onboarding
                ? "Confira o que foi detectado nesta máquina. IDEs instaladas em outro disco (ex.: D:\\) podem ser apontadas manualmente."
                : "Caminhos personalizados têm prioridade sobre a detecção automática."}
            </p>
          </div>
          <button type="button" onClick={onClose} aria-label="Fechar" className="rounded-md p-1.5 text-zinc-500 hover:bg-zinc-900 hover:text-zinc-100">
            <X className="size-4" />
          </button>
        </header>

        <div className="flex items-center gap-1 border-b border-zinc-800 bg-void px-4" role="tablist">
          {TABS.map(({ id, label, icon: Icon }) => {
            const active = tab === id;
            return (
              <button
                key={id}
                type="button"
                role="tab"
                aria-selected={active}
                onClick={() => setTab(id)}
                className={`relative flex items-center gap-2 px-3 pt-2.5 pb-2.5 text-sm whitespace-nowrap transition-colors ${
                  active ? "font-medium text-zinc-100" : "text-zinc-500 hover:text-zinc-200"
                }`}
              >
                <Icon className="size-4" />
                {label}
                {id === "shortcuts" && conflicts.size > 0 && <span className="size-1.5 rounded-full bg-amber-400" />}
                {active && <span className="absolute inset-x-2 -bottom-px h-0.5 bg-zinc-100" />}
              </button>
            );
          })}
        </div>

        <div className="min-h-0 flex-1 space-y-6 overflow-y-auto px-6 py-5">
          {tab === "git" ? (
            <GitSettings gitAuth={gitAuth} onChange={setGitAuth} />
          ) : tab === "shortcuts" ? (
            <ShortcutsSettings
              shortcuts={shortcuts}
              conflicts={conflicts}
              onChange={(id, binding) => setShortcuts((s) => ({ ...s, [id]: binding }))}
              onResetAll={() => setShortcuts(resolveShortcuts({}))}
            />
          ) : (
          <>
          <Section index="01" title="Motor WSL2 & Kernel">
            <div className="grid grid-cols-3 divide-x divide-zinc-800 rounded-lg border border-zinc-800 bg-zinc-900">
              <Cell label="Status">
                <span className="flex items-center gap-2">
                  <span className={`size-1.5 rounded-full ${wslOk ? "bg-emerald-400" : "bg-amber-400"}`} />
                  {wslOk ? "Respondendo" : info ? "Sem versão" : "Verificando…"}
                </span>
              </Cell>
              <Cell label="Versão do WSL" mono>
                {info?.wslVersion ?? "—"}
              </Cell>
              <Cell label="Kernel" mono>
                {info?.kernelVersion ?? "—"}
              </Cell>
            </div>
            <button type="button" onClick={recheckWsl} disabled={checkingWsl} className={`${BTN_GHOST} mt-2`}>
              {checkingWsl ? <Loader2 className="size-3.5 animate-spin" /> : <RefreshCw className="size-3.5" />}
              Verificar novamente
            </button>
          </Section>

          <Section index="02" title="Distros encontradas" hint="Distro selecionada ao abrir o app">
            <div className="overflow-hidden rounded-lg border border-zinc-800">
              <DistroOption
                selected={defaultDistro === null}
                onSelect={() => setDefaultDistro(null)}
                title="Seguir o padrão do WSL"
                detail={wslDefault ? `hoje: ${wslDefault.name}` : undefined}
              />
              {distros.map((d) => (
                <DistroOption
                  key={d.name}
                  selected={defaultDistro === d.name}
                  onSelect={() => setDefaultDistro(d.name)}
                  title={d.name}
                  state={d}
                />
              ))}
              {distros.length === 0 && (
                <div className="px-4 py-4 text-xs text-zinc-500">Nenhuma distro instalada (wsl --install).</div>
              )}
            </div>
          </Section>

          <Section index="03" title="IDE padrão" hint="Botão em destaque no Explorador">
            <IdeGrid tools={tools} allTools={allTools} value={defaultIde} onChange={setDefaultIde} />
          </Section>

          <Section index="04" title="Mapeamento de IDEs e ferramentas" hint="Caminho personalizado > detecção automática">
            <ToolPathList
              tools={tools}
              paths={toolPaths.paths}
              checks={toolPaths.checks}
              onChange={toolPaths.setPath}
              onBrowse={toolPaths.browse}
            />
          </Section>

          <UpdatesSection />

          <Section index="06" title="Configuração inicial" hint="Setup Zero-to-Hero">
            <div className="flex items-center gap-3 rounded-lg border border-zinc-800 bg-zinc-900 px-4 py-3">
              <p className="flex-1 text-xs leading-relaxed text-zinc-500">
                Refaz o diagnóstico da máquina e permite instalar ou reparar WSL, Ubuntu, Windows Terminal, zsh e Node.
                Alterações não salvas aqui serão descartadas.
              </p>
              <button type="button" onClick={onOpenSetup} className={`${BTN_GHOST} shrink-0`}>
                <Wand2 className="size-3.5" />
                Abrir o setup
              </button>
            </div>
          </Section>
          </>
          )}
        </div>

        <footer className="flex items-center gap-3 border-t border-zinc-800 bg-void px-6 py-3.5">
          {conflicts.size > 0 && !error ? (
            <span className="flex min-w-0 flex-1 items-center gap-1.5 text-xs text-amber-300">
              <AlertTriangle className="size-3.5 shrink-0" />
              Dois atalhos usam a mesma combinação
            </span>
          ) : error ? (
            <span className="flex min-w-0 flex-1 items-center gap-1.5 text-xs text-rose-300">
              <AlertTriangle className="size-3.5 shrink-0" />
              <span className="truncate" title={error}>
                {error}
              </span>
            </span>
          ) : (
            <span className="min-w-0 flex-1 truncate font-mono text-[10px] text-zinc-600" title={settings.file ?? undefined}>
              {settings.file ?? "configurações só em memória"}
            </span>
          )}
          <button type="button" onClick={onClose} className={`${BTN_GHOST} h-8`}>
            {onboarding ? "Depois" : "Cancelar"}
          </button>
          <button type="button" onClick={save} disabled={saving || invalid} className={`${BTN_PRIMARY} h-8 px-4`}>
            {saving ? <Loader2 className="size-3.5 animate-spin" /> : <Check className="size-3.5" />}
            {onboarding ? "Concluir configuração" : "Salvar"}
          </button>
        </footer>
      </div>
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

function UpdatesSection() {
  const { state, currentVersion, lastChecked, checkNow, openDialog } = useUpdaterContext();
  const update = "update" in state ? state.update : undefined;
  const status =
    state.status === "checking"
      ? { label: "Verificando…", dot: "bg-zinc-500" }
      : state.status === "available"
        ? { label: `v${state.update.version} disponível`, dot: "bg-amber-400" }
        : state.status === "downloading" || state.status === "ready"
          ? { label: "Instalando atualização", dot: "bg-amber-400" }
          : state.status === "up-to-date"
            ? { label: "Atualizado", dot: "bg-emerald-400" }
            : state.status === "error"
              ? { label: "Falha na verificação", dot: "bg-rose-400" }
              : { label: "Ainda não verificado", dot: "bg-zinc-600" };

  return (
    <Section index="05" title="Atualizações do aplicativo" hint="GitHub Releases · verificação automática ao abrir">
      <div className="grid grid-cols-3 divide-x divide-zinc-800 rounded-lg border border-zinc-800 bg-zinc-900">
        <Cell label="Versão instalada" mono>
          {currentVersion ? `v${currentVersion}` : "—"}
        </Cell>
        <Cell label="Status">
          <span className="flex items-center gap-2">
            <span className={`size-1.5 shrink-0 rounded-full ${status.dot}`} />
            <span className="truncate">{status.label}</span>
          </span>
        </Cell>
        <Cell label="Última verificação" mono>
          {lastChecked
            ? new Date(lastChecked).toLocaleString("pt-BR", { dateStyle: "short", timeStyle: "short" })
            : "—"}
        </Cell>
      </div>
      {state.status === "error" && <p className="mt-1.5 text-[11px] text-rose-300">{state.message}</p>}
      <div className="mt-2 flex items-center gap-2">
        <button
          type="button"
          onClick={checkNow}
          disabled={state.status === "checking" || state.status === "downloading" || state.status === "ready"}
          className={BTN_GHOST}
        >
          {state.status === "checking" ? <Loader2 className="size-3.5 animate-spin" /> : <RefreshCw className="size-3.5" />}
          Verificar atualizações
        </button>
        {update && (
          <button type="button" onClick={openDialog} className={BTN_PRIMARY}>
            <ArrowUpCircle className="size-3.5" />
            Ver novidades e instalar v{update.version}
          </button>
        )}
      </div>
    </Section>
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

function DistroOption({
  selected,
  onSelect,
  title,
  detail,
  state,
}: {
  selected: boolean;
  onSelect: () => void;
  title: string;
  detail?: string;
  state?: Distro;
}) {
  return (
    <button
      type="button"
      role="radio"
      aria-checked={selected}
      onClick={onSelect}
      className={`flex w-full items-center gap-3 border-b border-zinc-800 px-4 py-2.5 text-left text-sm transition-colors last:border-0 ${
        selected ? "bg-zinc-900 text-zinc-100" : "text-zinc-400 hover:bg-zinc-900/60 hover:text-zinc-200"
      }`}
    >
      <span
        className={`flex size-3.5 shrink-0 items-center justify-center rounded-full border ${
          selected ? "border-zinc-100" : "border-zinc-600"
        }`}
      >
        {selected && <span className="size-1.5 rounded-full bg-zinc-100" />}
      </span>
      {state && <StatusDot state={state.state} />}
      <span className="font-medium">{title}</span>
      {state?.isDefault && <Star className="size-3 fill-zinc-500 text-zinc-500" aria-label="Padrão do WSL" />}
      <span className="ml-auto font-mono text-[11px] text-zinc-500">
        {state ? `${state.stateLabel} · WSL ${state.version}` : detail}
      </span>
    </button>
  );
}
