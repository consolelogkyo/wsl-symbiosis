import { Suspense, lazy, useEffect, useMemo, useState } from "react";
import { resolveShortcuts } from "./lib/shortcuts";
import type { TerminalRequest } from "./views/TerminalView";
import { AlertTriangle, X } from "lucide-react";
import { Titlebar } from "./components/Titlebar";
import { Sidebar } from "./components/Sidebar";
import { SettingsDialog } from "./components/SettingsDialog";
import { useWsl } from "./hooks/useWsl";
import { useMetrics } from "./hooks/useMetrics";
import { ActivityContext, useActivity } from "./hooks/useActivity";
import { Toaster } from "./components/Toaster";
import { GitQueuePanel } from "./components/GitQueuePanel";
import { GitQueueContext, useGitQueue } from "./hooks/useGitQueue";
import { TaskLogDrawer } from "./components/TaskLogDrawer";
import { UpdateDialog } from "./components/UpdateDialog";
import { UpdaterContext, useUpdater } from "./hooks/useUpdater";
import { SettingsSnapshot, getSettings } from "./lib/settings";
import { BANNER_ERROR } from "./lib/ui";
import type { ToolId } from "./lib/explorer";
import { NAV_ITEMS, type ViewId } from "./views";
import { DashboardView } from "./views/DashboardView";
import { ExplorerView } from "./views/ExplorerView";
import { PackagesView } from "./views/PackagesView";

// xterm is heavy; load it only when the Terminal screen is opened.
const TerminalView = lazy(() => import("./views/TerminalView").then((m) => ({ default: m.TerminalView })));

export default function App() {
  const [view, setView] = useState<ViewId>("dashboard");
  // "Terminal aqui" in the Explorer: opens (or reuses) a terminal tab at that folder.
  const [terminalRequest, setTerminalRequest] = useState<TerminalRequest | null>(null);
  // Once opened, the Terminal stays mounted (hidden) so PTY sessions and scrollback survive
  // switching screens.
  const [terminalMounted, setTerminalMounted] = useState(false);
  // Same for the Explorer: its tabs (folders, history) survive switching screens.
  const [explorerMounted, setExplorerMounted] = useState(false);
  const [settings, setSettings] = useState<SettingsSnapshot | null>(null);
  const [settingsOpen, setSettingsOpen] = useState(false);

  // First run: the diagnostic opens by itself until it is completed once.
  useEffect(() => {
    getSettings()
      .then((s) => {
        setSettings(s);
        if (!s.onboardingCompleted) setSettingsOpen(true);
      })
      .catch(() =>
        setSettings({
          onboardingCompleted: true,
          customToolPaths: {},
          defaultDistro: null,
          defaultIde: null,
          shortcuts: {},
          gitAuth: "auto",
          file: null,
        }),
      );
  }, []);

  const wsl = useWsl(settings ? settings.defaultDistro : undefined);
  // The sidebar's live panel needs telemetry on every screen; faster while on the Dashboard.
  const telemetry = useMetrics(wsl.selected?.name ?? null, view === "dashboard" ? 3000 : 5000);
  // Projects started from the Explorer + everything listening in the distro (sidebar, rows, toasts).
  const activity = useActivity(wsl.selected?.name ?? null, wsl.selected?.state === "Running");
  // git pull queue (Explorer): one repository at a time, keeps going across screens.
  const gitQueue = useGitQueue(activity.notify);
  // Auto-update (GitHub Releases): silent check after start-up, pill in the Titlebar.
  const updater = useUpdater();
  const current = NAV_ITEMS.find((n) => n.id === view)!;
  const shortcuts = useMemo(() => resolveShortcuts(settings?.shortcuts), [settings]);

  const navigate = (next: ViewId) => {
    if (next === "terminal") setTerminalMounted(true);
    if (next === "explorer") setExplorerMounted(true);
    setView(next);
  };

  return (
    <ActivityContext.Provider value={activity}>
    <GitQueueContext.Provider value={gitQueue}>
    <UpdaterContext.Provider value={updater}>
    <div className="flex h-full flex-col overflow-hidden">
      <Titlebar
        section={current.label}
        distros={wsl.distros}
        selected={wsl.selected}
        loading={wsl.loading}
        pending={wsl.pending}
        onSelectDistro={wsl.select}
        onStart={wsl.start}
        onShutdown={wsl.shutdown}
        onRefresh={wsl.refresh}
        onOpenSettings={() => setSettingsOpen(true)}
      />

      <div className="flex min-h-0 flex-1">
        <Sidebar
          view={view}
          onNavigate={navigate}
          selected={wsl.selected}
          metrics={telemetry.metrics}
          onOpenSettings={() => setSettingsOpen(true)}
        />

        <div className="flex min-w-0 flex-1 flex-col bg-zinc-950">
          {wsl.error && (
            <div className={BANNER_ERROR}>
              <AlertTriangle className="mt-px size-3.5 shrink-0" />
              <span className="flex-1 font-mono break-all whitespace-pre-wrap">{wsl.error}</span>
              <button type="button" onClick={wsl.dismissError} className="text-rose-400/70 hover:text-rose-200">
                <X className="size-3.5" />
              </button>
            </div>
          )}

          <main className="min-h-0 flex-1 overflow-auto">
            {view === "dashboard" && (
              <DashboardView
                distros={wsl.distros}
                selected={wsl.selected}
                info={wsl.info}
                pending={wsl.pending}
                telemetry={telemetry}
                onSelect={wsl.select}
                onStart={wsl.start}
              />
            )}
            {explorerMounted && (
              <div className={view === "explorer" ? "h-full" : "hidden"}>
                <ExplorerView
                  distro={wsl.selected}
                  distros={wsl.distros}
                  active={view === "explorer"}
                  pending={wsl.pending}
                  onStart={wsl.start}
                  defaultIde={(settings?.defaultIde as ToolId | null | undefined) ?? null}
                  shortcuts={shortcuts}
                  onTerminalHere={(path) => {
                    if (wsl.selected) setTerminalRequest({ distro: wsl.selected.name, path, nonce: Date.now() });
                    navigate("terminal");
                  }}
                />
              </div>
            )}
            {view === "packages" && (
              <PackagesView distro={wsl.selected} pending={wsl.pending} onStart={wsl.start} />
            )}
            {terminalMounted && (
              <div className={view === "terminal" ? "h-full" : "hidden"}>
                <Suspense fallback={null}>
                  <TerminalView distro={wsl.selected} request={terminalRequest} active={view === "terminal"} />
                </Suspense>
              </div>
            )}
          </main>
        </div>
      </div>

      {settingsOpen && settings && (
        <SettingsDialog
          onboarding={!settings.onboardingCompleted}
          settings={settings}
          distros={wsl.distros}
          info={wsl.info}
          onSaved={(saved) => {
            setSettings(saved);
            setSettingsOpen(false);
            if (saved.defaultDistro && wsl.distros.some((d) => d.name === saved.defaultDistro)) {
              wsl.select(saved.defaultDistro);
            }
          }}
          onClose={() => setSettingsOpen(false)}
        />
      )}

      {updater.dialogOpen && <UpdateDialog />}

      <TaskLogDrawer />
      <div className="pointer-events-none fixed right-4 bottom-4 z-50 flex w-96 flex-col gap-2">
        <Toaster />
        <GitQueuePanel />
      </div>
    </div>
    </UpdaterContext.Provider>
    </GitQueueContext.Provider>
    </ActivityContext.Provider>
  );
}
