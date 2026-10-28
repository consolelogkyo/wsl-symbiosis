import { useEffect, useRef, useState } from "react";
import { Check, File, Folder, FolderGit2, Link2, Loader2, Pencil, Plus, Trash2, X } from "lucide-react";
import { GitBranchChip } from "./GitBranchChip";
import { DirEntry, ToolId, ToolInfo, canCreateEnv, isNavigable, isProtectedPath, joinPath } from "../../lib/explorer";
import { formatBytes } from "../../lib/format";
import { ProjectBadges, mainStack } from "./ProjectBadges";
import { OpenInButtons } from "./OpenInButtons";
import { ACTIONS_SLOT, ProjectActions } from "./ProjectActions";
import { BTN_DANGER_SOLID } from "../../lib/ui";
import { matchesClick } from "../../lib/shortcuts";

interface Props {
  distro: string;
  entry: DirEntry;
  tools: ToolInfo[];
  defaultIde: ToolId | null;
  onActivate: (entry: DirEntry) => void;
  /** Gestures (default Ctrl+click, middle click) that open a folder in a new tab, like a link. */
  newTabClick: string;
  onOpenInNewTab: (path: string) => void;
  onRename: (entry: DirEntry, newName: string) => Promise<boolean>;
  onDelete: (path: string) => Promise<boolean>;
  onOpen: (path: string, tool: ToolId) => Promise<void>;
  onTerminalHere: (path: string) => void;
  /** Something changed on disk (branch switched): reload the listing. */
  onChanged: () => void;
  /** "+ Criar .env": copies the project's .env.example and opens the new file in the editor. */
  onCreateEnv?: (entry: DirEntry) => Promise<void>;
  /** Batch git pull selection mode: a checkbox per repository; clicks toggle it. */
  selection?: { selectable: boolean; selected: boolean; onToggle: () => void };
}

const dateFormat = new Intl.DateTimeFormat("pt-BR", { dateStyle: "short", timeStyle: "short" });

/** What is pending confirmation: the entry itself or one of its heavy subfolders. */
type Confirm = { path: string; label: string } | null;

export function EntryRow({
  distro,
  entry,
  tools,
  defaultIde,
  onActivate,
  newTabClick,
  onOpenInNewTab,
  onRename,
  onDelete,
  onOpen,
  onTerminalHere,
  onChanged,
  onCreateEnv,
  selection,
}: Props) {
  const [renaming, setRenaming] = useState(false);
  const [confirm, setConfirm] = useState<Confirm>(null);
  const [busy, setBusy] = useState(false);
  const [creatingEnv, setCreatingEnv] = useState(false);

  const navigable = isNavigable(entry);
  const stack = navigable ? mainStack(entry.projectMarkers) : null;
  const project = stack !== null;
  const Icon = entry.kind === "symlink" ? Link2 : !navigable ? File : entry.projectMarkers.includes(".git") ? FolderGit2 : Folder;

  const runDelete = async () => {
    if (!confirm) return;
    setBusy(true);
    await onDelete(confirm.path);
    setBusy(false);
    setConfirm(null);
  };

  return (
    <div
      role="row"
      onClick={(e) => {
        if (renaming || confirm) return;
        if (selection) selection.selectable && selection.onToggle();
        else if (navigable && matchesClick(e, newTabClick)) onOpenInNewTab(entry.path);
        else onActivate(entry);
      }}
      onMouseDown={(e) => navigable && e.button === 1 && e.preventDefault()} // no autoscroll cursor
      onAuxClick={(e) => navigable && !renaming && !confirm && matchesClick(e, newTabClick) && onOpenInNewTab(entry.path)}
      // @container: the row adapts to its own width (narrow rows hide badges, labels and
      // cleanup chips, see ProjectActions / OpenInButtons), not the window's.
      className={`@container group relative flex items-center gap-3 border-b border-zinc-800/60 px-4 py-1.5 transition-colors ${
        selection && !selection.selectable ? "cursor-default opacity-35" : "cursor-pointer hover:bg-zinc-900"
      } ${selection?.selected ? "bg-zinc-900" : ""}`}
    >
      {selection && (
        <span
          role="checkbox"
          aria-checked={selection.selected}
          aria-disabled={!selection.selectable}
          className={`flex size-4 shrink-0 items-center justify-center rounded border transition-colors ${
            selection.selected ? "border-zinc-100 bg-zinc-100 text-void" : "border-zinc-600"
          } ${selection.selectable ? "" : "invisible"}`}
        >
          {selection.selected && <Check className="size-3" strokeWidth={3} />}
        </span>
      )}
      <Icon
        className={`size-4 shrink-0 ${stack ? stack.folder : navigable ? "text-zinc-400" : "text-zinc-600"}`}
      />

      <div className="flex min-w-0 flex-1 items-center gap-2 overflow-hidden">
        {renaming ? (
          <RenameInput
            initial={entry.name}
            onCancel={() => setRenaming(false)}
            onSubmit={async (name) => {
              if (await onRename(entry, name)) setRenaming(false);
            }}
          />
        ) : (
          <span className={`truncate text-sm ${navigable ? "text-zinc-100" : "text-zinc-300"}`} title={entry.path}>
            {entry.name}
          </span>
        )}
        {/* The branch replaces the "GIT" badge and stays visible on narrow rows. */}
        {navigable && entry.gitBranch && <GitBranchChip distro={distro} entry={entry} onChanged={onChanged} />}
        {navigable && onCreateEnv && canCreateEnv(entry.projectMarkers) && (
          <button
            type="button"
            disabled={creatingEnv}
            onClick={async (e) => {
              e.stopPropagation();
              setCreatingEnv(true);
              await onCreateEnv(entry);
              setCreatingEnv(false);
            }}
            title="Cria o arquivo .env a partir do modelo de exemplo do projeto e abre para você preencher"
            className="flex shrink-0 items-center gap-1 rounded-full border border-dashed border-zinc-600 px-2 py-0.5 font-mono text-[10px] text-zinc-300 transition-colors hover:border-zinc-300 hover:bg-zinc-800 hover:text-zinc-100 disabled:opacity-50"
          >
            {creatingEnv ? <Loader2 className="size-3 animate-spin" /> : <Plus className="size-3" />}
            Criar .env
          </button>
        )}
        <span className="flex shrink-0 items-center gap-2 @max-5xl:hidden">
          <ProjectBadges markers={entry.gitBranch ? entry.projectMarkers.filter((m) => m !== ".git") : entry.projectMarkers} />
        </span>
      </div>

      {confirm ? (
        <div className="flex shrink-0 items-center gap-1.5" onClick={(e) => e.stopPropagation()}>
          <span className="text-xs text-rose-300">Excluir {confirm.label}?</span>
          <button
            type="button"
            onClick={runDelete}
            disabled={busy}
            className={BTN_DANGER_SOLID}
          >
            {busy && <Loader2 className="size-3 animate-spin" />}
            Excluir
          </button>
          <button
            type="button"
            onClick={() => setConfirm(null)}
            disabled={busy}
            aria-label="Cancelar"
            className="rounded p-0.5 text-zinc-500 hover:text-zinc-200"
          >
            <X className="size-3.5" />
          </button>
        </div>
      ) : (
        <>
          <div
            className={`shrink-0 transition-opacity ${navigable ? "" : "hidden"} ${
              project ? "" : "opacity-0 group-hover:opacity-100 focus-within:opacity-100"
            }`}
          >
            {navigable && (
              <OpenInButtons
                tools={tools}
                defaultIde={defaultIde}
                variant="compact"
                withWindowsTerminal={false}
                onOpen={(tool) => onOpen(entry.path, tool)}
                onTerminalHere={() => onTerminalHere(entry.path)}
              />
            )}
          </div>

          <div className="flex shrink-0 items-center opacity-0 transition-opacity group-hover:opacity-100 focus-within:opacity-100">
            <button
              type="button"
              onClick={(e) => {
                e.stopPropagation();
                setRenaming(true);
              }}
              title="Renomear"
              className="rounded p-1 text-zinc-500 hover:bg-zinc-800 hover:text-zinc-200"
            >
              <Pencil className="size-3.5" />
            </button>
            {!isProtectedPath(entry.path) && (
              <button
                type="button"
                onClick={(e) => {
                  e.stopPropagation();
                  setConfirm({ path: entry.path, label: entry.name });
                }}
                title="Excluir"
                className="rounded p-1 text-zinc-500 hover:bg-zinc-800 hover:text-rose-300"
              >
                <Trash2 className="size-3.5" />
              </button>
            )}
          </div>

          {/* Fixed slot on the right (run/stop, install, cleanup) so the "Abrir em" buttons
              line up across rows. */}
          {project ? (
            <ProjectActions
              distro={distro}
              entry={entry}
              onCleanup={(h) => setConfirm({ path: joinPath(entry.path, h), label: h })}
            />
          ) : (
            <div className={`${ACTIONS_SLOT} shrink-0`} />
          )}
        </>
      )}

      <span className="w-20 shrink-0 text-right font-mono text-xs text-zinc-500 tabular-nums">
        {navigable ? "—" : formatBytes(entry.size)}
      </span>
      <span className="hidden w-32 shrink-0 text-right text-xs text-zinc-500 tabular-nums @min-6xl:block">
        {entry.modified ? dateFormat.format(entry.modified * 1000) : ""}
      </span>
      <span className="hidden w-24 shrink-0 text-right font-mono text-[11px] text-zinc-600 @min-7xl:block">
        {entry.permissions}
      </span>
    </div>
  );
}

function RenameInput({
  initial,
  onSubmit,
  onCancel,
}: {
  initial: string;
  onSubmit: (name: string) => void;
  onCancel: () => void;
}) {
  const [value, setValue] = useState(initial);
  const ref = useRef<HTMLInputElement>(null);

  useEffect(() => {
    const input = ref.current;
    if (!input) return;
    input.focus();
    // Select the name without the extension, like file managers do.
    const dot = initial.lastIndexOf(".");
    input.setSelectionRange(0, dot > 0 ? dot : initial.length);
  }, [initial]);

  return (
    <input
      ref={ref}
      value={value}
      onClick={(e) => e.stopPropagation()}
      onChange={(e) => setValue(e.target.value)}
      onKeyDown={(e) => {
        if (e.key === "Enter") onSubmit(value.trim());
        if (e.key === "Escape") onCancel();
      }}
      onBlur={onCancel}
      className="w-full max-w-md rounded border border-zinc-400 bg-zinc-950 px-1.5 py-0.5 text-sm text-zinc-100 focus:outline-none"
    />
  );
}

export function NewEntryRow({
  kind,
  onSubmit,
  onCancel,
}: {
  kind: "dir" | "file";
  onSubmit: (name: string) => Promise<boolean>;
  onCancel: () => void;
}) {
  const [value, setValue] = useState("");
  const [busy, setBusy] = useState(false);
  const Icon = kind === "dir" ? Folder : File;

  const submit = async () => {
    if (!value.trim()) return onCancel();
    setBusy(true);
    const ok = await onSubmit(value.trim());
    setBusy(false);
    if (ok) onCancel();
  };

  return (
    <div className="flex items-center gap-3 border-b border-zinc-800 bg-zinc-900 px-4 py-1.5">
      <Icon className="size-4 shrink-0 text-zinc-100" />
      <input
        autoFocus
        value={value}
        disabled={busy}
        onChange={(e) => setValue(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter") submit();
          if (e.key === "Escape") onCancel();
        }}
        placeholder={kind === "dir" ? "Nome da nova pasta" : "Nome do novo arquivo"}
        className="w-full max-w-md rounded border border-zinc-400 bg-zinc-950 px-1.5 py-0.5 text-sm text-zinc-100 placeholder:text-zinc-600 focus:outline-none"
      />
      {busy && <Loader2 className="size-3.5 animate-spin text-zinc-500" />}
      <span className="text-[11px] text-zinc-500">Enter para criar · Esc para cancelar</span>
    </div>
  );
}
