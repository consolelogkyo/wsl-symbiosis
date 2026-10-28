import { useEffect, useMemo, useRef, useState } from "react";
import { AlertTriangle, CloudDownload, FolderGit2, FolderOpen, Loader2, X } from "lucide-react";
import { CloneDialog } from "./CloneDialog";
import { useActivityContext } from "../../hooks/useActivity";
import { useExplorer } from "../../hooks/useExplorer";
import { useTools } from "../../hooks/useTools";
import { isActive } from "../../lib/projects";
import { errorMessage } from "../../lib/wsl";
import {
  DirEntry,
  ToolId,
  createDirectory,
  createEnvFromExample,
  createFile,
  deleteEntry,
  isNavigable,
  joinPath,
  openInTool,
  parentPath,
  renameEntry,
  validateEntryName,
} from "../../lib/explorer";
import { BANNER_ERROR, BTN_GHOST, BTN_PRIMARY } from "../../lib/ui";
import { useGitQueueContext } from "../../hooks/useGitQueue";
import { ExplorerToolbar } from "./ExplorerToolbar";
import { EntryRow, NewEntryRow } from "./EntryRow";
import { OpenInButtons } from "./OpenInButtons";
import { isProject } from "./ProjectBadges";
import { FileEditor } from "./FileEditor";

interface Props {
  distro: string;
  running: boolean;
  /** Folder the tab opens at; null = the user's home. */
  initialPath: string | null;
  visible: boolean;
  /** Reports the folder shown, for the tab label and "Nova aba" (opens next to it). */
  onPath: (path: string) => void;
  /** Mouse gestures that open a folder in a new tab (Settings → Atalhos). */
  newTabClick: string;
  onOpenInNewTab: (path: string) => void;
  onTerminalHere: (path: string) => void;
  /** IDE configured as default in the settings (the labeled "Abrir em" button). */
  defaultIde: ToolId | null;
}

/** A git repository on a branch (detached HEADs can't be pulled). */
const isRepo = (e: DirEntry) => isNavigable(e) && e.gitBranch !== null && !e.gitDetached;

/** localStorage is a per-machine convenience here: failures just mean nothing is remembered. */
function readJson(key: string | null): string[] | null {
  try {
    return key ? JSON.parse(localStorage.getItem(key) ?? "null") : null;
  } catch {
    return null;
  }
}

function writeJson(key: string | null, value: string[]) {
  try {
    if (key) localStorage.setItem(key, JSON.stringify(value));
  } catch {
    // ignore
  }
}

/** One Explorer tab: its own folder, history, filter and hidden-files toggle. */
export function ExplorerPane({
  distro,
  running,
  initialPath,
  visible,
  onPath,
  newTabClick,
  onOpenInNewTab,
  onTerminalHere,
  defaultIde,
}: Props) {
  const ex = useExplorer(distro, running, initialPath);
  const tools = useTools();
  const [filter, setFilter] = useState("");
  const [creating, setCreating] = useState<"dir" | "file" | null>(null);
  const [editing, setEditing] = useState<string | null>(null);

  const path = ex.listing?.path ?? null;
  const onPathRef = useRef(onPath);
  onPathRef.current = onPath;
  useEffect(() => {
    setFilter("");
    setCreating(null);
    if (path) onPathRef.current(path);
  }, [path]);

  // An install finishing in a project listed here creates node_modules: refresh the rows.
  const { tasks } = useActivityContext();
  const finishedInstalls = tasks
    .filter((t) => t.kind === "install" && !isActive(t) && path !== null && parentPath(t.path) === path)
    .map((t) => t.id)
    .join(",");
  const refreshRef = useRef(ex.refresh);
  refreshRef.current = ex.refresh;
  useEffect(() => {
    if (finishedInstalls) refreshRef.current();
  }, [finishedInstalls]);

  const entries = useMemo(() => {
    const all = ex.listing?.entries ?? [];
    const q = filter.trim().toLowerCase();
    return q ? all.filter((e) => e.name.toLowerCase().includes(q)) : all;
  }, [ex.listing, filter]);

  /** Runs an operation; on success refreshes, on failure shows the banner. */
  const act = async (task: () => Promise<void>): Promise<boolean> => {
    try {
      await task();
      ex.setError(null);
      await ex.refresh();
      return true;
    } catch (err) {
      ex.setError(errorMessage(err));
      return false;
    }
  };

  const open = async (target: string, tool: ToolId) => {
    try {
      await openInTool(distro, target, tool);
    } catch (err) {
      ex.setError(errorMessage(err));
    }
  };

  const activate = (entry: DirEntry) => {
    if (isNavigable(entry)) ex.navigate(entry.path);
    else setEditing(entry.path);
  };

  const rename = (entry: DirEntry, newName: string) => {
    if (newName === entry.name) return Promise.resolve(true);
    const invalid = validateEntryName(newName);
    if (invalid) {
      ex.setError(invalid);
      return Promise.resolve(false);
    }
    const dir = parentPath(entry.path) ?? "/";
    return act(() => renameEntry(distro, entry.path, joinPath(dir, newName)));
  };

  const create = (kind: "dir" | "file", entryName: string) => {
    const invalid = validateEntryName(entryName);
    if (invalid || !path) {
      ex.setError(invalid);
      return Promise.resolve(false);
    }
    const target = joinPath(path, entryName);
    return act(() => (kind === "dir" ? createDirectory(distro, target) : createFile(distro, target)));
  };

  /** "+ Criar .env": copy the template, refresh the row, and open the new file to fill in. */
  const createEnv = async (entry: DirEntry) => {
    try {
      const file = await createEnvFromExample(distro, entry.path);
      ex.setError(null);
      await ex.refresh();
      setEditing(file);
    } catch (err) {
      ex.setError(errorMessage(err));
    }
  };

  const projectsHere = (ex.listing?.entries ?? []).filter((e) => isProject(e.projectMarkers)).length;
  const reposHere = (ex.listing?.entries ?? []).filter(isRepo);

  // Batch git pull: pick repositories in this folder, then queue them (useGitQueue).
  const queue = useGitQueueContext();
  const [selected, setSelected] = useState<Set<string> | null>(null);
  const [cloning, setCloning] = useState(false);
  const selectionKey = path ? `gitPullSelection:${distro}:${path}` : null;
  useEffect(() => setSelected(null), [path]);

  const startSelecting = () => {
    // Pre-check what was pulled from this folder last time (weekly routine, same repos).
    const remembered = new Set<string>(readJson(selectionKey) ?? []);
    setSelected(new Set(reposHere.filter((e) => remembered.has(e.name)).map((e) => e.path)));
  };
  const toggle = (p: string) =>
    setSelected((s) => {
      const next = new Set(s);
      if (next.has(p)) next.delete(p);
      else next.add(p);
      return next;
    });
  const pullSelected = () => {
    if (!selected) return;
    const repos = reposHere.filter((e) => selected.has(e.path));
    writeJson(selectionKey, repos.map((e) => e.name));
    queue.enqueue(repos.map((e) => ({ distro, path: e.path, name: e.name })));
    setSelected(null);
  };

  // A pull finishing in a repository listed here may bring new files: refresh the rows.
  const finishedPulls = queue.items
    .filter((i) => i.status === "updated" && i.distro === distro && path !== null && parentPath(i.path) === path)
    .map((i) => i.id)
    .join(",");
  useEffect(() => {
    if (finishedPulls) refreshRef.current();
  }, [finishedPulls]);

  return (
    <div className={visible ? "flex min-h-0 flex-1 flex-col" : "hidden"}>
      {ex.listing && path ? (
        <ExplorerToolbar
          distro={distro}
          path={path}
          windowsPath={ex.listing.windowsPath}
          home={ex.home}
          loading={ex.loading}
          canGoBack={ex.canGoBack}
          showHidden={ex.showHidden}
          filter={filter}
          onFilter={setFilter}
          onNavigate={ex.navigate}
          onBack={ex.back}
          onUp={() => ex.listing?.parent && ex.navigate(ex.listing.parent)}
          onRefresh={() => ex.refresh()}
          onToggleHidden={ex.toggleHidden}
          onNew={setCreating}
        />
      ) : null}

      {ex.error && (
        <div className={BANNER_ERROR}>
          <AlertTriangle className="mt-px size-3.5 shrink-0" />
          <span className="flex-1 font-mono break-all whitespace-pre-wrap">{ex.error}</span>
          <button type="button" onClick={() => ex.setError(null)} className="text-rose-400/70 hover:text-rose-200">
            <X className="size-3.5" />
          </button>
        </div>
      )}

      {path && selected && (
        <div className="@container flex items-center gap-3 border-b border-zinc-800 bg-zinc-900 px-4 py-2">
          <CloudDownload className="size-4 shrink-0 text-zinc-500" />
          <span className="text-[11px] font-medium tracking-wider whitespace-nowrap text-zinc-500 uppercase">git pull em lote</span>
          <span className="font-mono text-[11px] whitespace-nowrap text-zinc-300">
            {selected.size} de {reposHere.length}
          </span>
          <button
            type="button"
            onClick={() => setSelected(new Set(selected.size === reposHere.length ? [] : reposHere.map((e) => e.path)))}
            className="rounded-full px-2.5 py-1 text-xs whitespace-nowrap text-zinc-400 transition-colors hover:bg-zinc-800 hover:text-zinc-100"
          >
            {selected.size === reposHere.length ? "Limpar seleção" : "Selecionar todos"}
          </button>
          <span className="ml-auto flex items-center gap-2">
            <button type="button" onClick={() => setSelected(null)} className={`${BTN_GHOST} whitespace-nowrap`}>
              Cancelar
            </button>
            <button type="button" onClick={pullSelected} disabled={selected.size === 0} className={`${BTN_PRIMARY} whitespace-nowrap`}>
              <CloudDownload className="size-3.5" />
              Atualizar {selected.size || ""} {selected.size === 1 ? "repositório" : "repositórios"}
            </button>
          </span>
        </div>
      )}

      {path && !selected && (
        <div className="@container flex items-center gap-3 border-b border-zinc-800 bg-zinc-900 px-4 py-2">
          <FolderOpen className="size-4 shrink-0 text-zinc-500" />
          <span className="text-[11px] font-medium tracking-wider whitespace-nowrap text-zinc-500 uppercase">Abrir pasta em</span>
          <OpenInButtons
            tools={tools}
            defaultIde={defaultIde}
            variant="full"
            onOpen={(tool) => open(path, tool)}
            onTerminalHere={() => onTerminalHere(path)}
          />
          <span className="ml-auto text-[11px] whitespace-nowrap text-zinc-600">
            {ex.listing?.entries.length ?? 0} itens
            {projectsHere > 0 && ` · ${projectsHere} projeto${projectsHere > 1 ? "s" : ""}`}
          </span>
          <button
            type="button"
            onClick={() => setCloning(true)}
            title="Clonar um repositório (link HTTPS ou SSH) para dentro desta pasta"
            className={`${BTN_GHOST} shrink-0 whitespace-nowrap`}
          >
            <FolderGit2 className="size-3.5" />
            <span className="@max-[60rem]:hidden">git clone</span>
          </button>
          {reposHere.length > 0 && (
            <button
              type="button"
              onClick={startSelecting}
              title="Escolher repositórios desta pasta e fazer git pull em fila, um de cada vez"
              className={`${BTN_GHOST} shrink-0 whitespace-nowrap`}
            >
              <CloudDownload className="size-3.5" />
              <span className="@max-[60rem]:hidden">git pull em lote</span>
            </button>
          )}
        </div>
      )}

      <div className="min-h-0 flex-1 overflow-auto" role="grid" aria-busy={ex.loading}>
        {creating && (
          <NewEntryRow kind={creating} onSubmit={(n) => create(creating, n)} onCancel={() => setCreating(null)} />
        )}
        {!ex.listing && !ex.error && (
          <div className="flex h-40 items-center justify-center">
            <Loader2 className="size-5 animate-spin text-zinc-600" />
          </div>
        )}
        {entries.map((entry) => (
          <EntryRow
            key={entry.path}
            distro={distro}
            entry={entry}
            tools={tools}
            defaultIde={defaultIde}
            onActivate={activate}
            newTabClick={newTabClick}
            onOpenInNewTab={onOpenInNewTab}
            onRename={rename}
            onDelete={(target) => act(() => deleteEntry(distro, target))}
            onOpen={open}
            onTerminalHere={onTerminalHere}
            onChanged={() => ex.refresh()}
            onCreateEnv={createEnv}
            selection={
              selected
                ? { selectable: isRepo(entry), selected: selected.has(entry.path), onToggle: () => toggle(entry.path) }
                : undefined
            }
          />
        ))}
        {ex.listing && entries.length === 0 && (
          <div className="px-4 py-10 text-center text-xs text-zinc-500">
            {filter ? `Nada corresponde a "${filter}".` : "Pasta vazia."}
          </div>
        )}
      </div>

      {cloning && path && (
        <CloneDialog
          distro={distro}
          parent={path}
          existing={(ex.listing?.entries ?? []).map((e) => e.name)}
          onClose={() => setCloning(false)}
          onCloned={() => refreshRef.current()}
        />
      )}

      {editing && (
        <FileEditor distro={distro} path={editing} onClose={() => setEditing(null)} onSaved={() => ex.refresh()} />
      )}
    </div>
  );
}
