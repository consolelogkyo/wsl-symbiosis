import { useEffect, useRef, useState, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { AlertTriangle, Check, ChevronDown, CloudDownload, GitBranch, Loader2, RefreshCw, Search } from "lucide-react";
import type { DirEntry } from "../../lib/explorer";
import { type GitBranches, gitBranches, gitFetch, gitSwitch, splitGitError } from "../../lib/git";
import { type MenuPlacement, closeOnOutside, placeMenu } from "../../lib/menu";
import { isActive } from "../../lib/projects";
import { STACK_BRANDS } from "../../lib/brands";
import { errorMessage } from "../../lib/wsl";
import { useActivityContext } from "../../hooks/useActivity";
import { useGitQueueContext } from "../../hooks/useGitQueue";
import { BrandIcon } from "../BrandIcon";

const MENU_WIDTH = 320;
/** Above this many branches the menu shows a filter field. */
const FILTER_FROM = 8;

type Busy = { kind: "switch"; branch: string } | { kind: "fetch" } | null;

interface Props {
  distro: string;
  entry: DirEntry;
  /** The branch changed: reload the listing (the chip reads .git/HEAD from it). */
  onChanged: () => void;
}

/**
 * The repository's branch, in Git's color (it replaces the "GIT" stack badge). Click: switch
 * branch (local or remote), fetch, or queue a `git pull` for this repository.
 */
export function GitBranchChip({ distro, entry, onChanged }: Props) {
  const [menu, setMenu] = useState<MenuPlacement | null>(null);
  const anchor = useRef<HTMLButtonElement>(null);
  const queue = useGitQueueContext();
  const pull = queue.itemFor(distro, entry.path);
  const pulling = pull?.status === "running" || pull?.status === "queued";

  return (
    <>
      <button
        ref={anchor}
        type="button"
        onClick={(e) => {
          e.stopPropagation();
          setMenu(menu ? null : placeMenu(anchor.current!.getBoundingClientRect(), MENU_WIDTH, "left"));
        }}
        title={entry.gitDetached ? `HEAD destacado em ${entry.gitBranch}` : `Branch ${entry.gitBranch}: clique para trocar ou atualizar`}
        className="flex max-w-48 min-w-0 shrink items-center gap-1 @max-5xl:max-w-28 rounded-sm border border-red-500/30 bg-red-500/10 px-1.5 py-px font-mono text-[10px] leading-4 text-red-300 transition-colors hover:border-red-400/60 hover:bg-red-500/20"
      >
        {pulling ? (
          <Loader2 className="size-2.5 shrink-0 animate-spin" />
        ) : (
          <BrandIcon icon={STACK_BRANDS.git} className="size-2.5 shrink-0" color="currentColor" />
        )}
        <span className="truncate">{entry.gitDetached ? `@${entry.gitBranch}` : entry.gitBranch}</span>
        {pull?.status === "failed" && <span className="size-1.5 shrink-0 rounded-full bg-rose-500" title={pull.error?.title} />}
        <ChevronDown className="size-2.5 shrink-0 opacity-70" />
      </button>
      {menu && (
        <BranchMenu
          distro={distro}
          entry={entry}
          placement={menu}
          onClose={() => setMenu(null)}
          onChanged={onChanged}
        />
      )}
    </>
  );
}

function BranchMenu({
  distro,
  entry,
  placement,
  onClose,
  onChanged,
}: {
  distro: string;
  entry: DirEntry;
  placement: MenuPlacement;
  onClose: () => void;
  onChanged: () => void;
}) {
  const [data, setData] = useState<GitBranches | null>(null);
  const [busy, setBusy] = useState<Busy>(null);
  const [error, setError] = useState<{ title: string; detail: string } | null>(null);
  const [filter, setFilter] = useState("");
  const panel = useRef<HTMLDivElement>(null);
  const { notify, taskFor } = useActivityContext();
  const queue = useGitQueueContext();
  const task = taskFor(entry.path);
  const running = task && isActive(task) && task.kind === "run";

  const load = () =>
    gitBranches(distro, entry.path)
      .then((b) => {
        setData(b);
        setError(null);
      })
      .catch((err) => setError(splitGitError(errorMessage(err))));

  const busyRef = useRef<Busy>(null);
  busyRef.current = busy;
  useEffect(() => {
    load();
    // While git works, a stray click must not close the menu and hide the result.
    return closeOnOutside(panel, () => !busyRef.current && onClose());
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const switchTo = async (branch: string, remote: boolean) => {
    setBusy({ kind: "switch", branch });
    setError(null);
    try {
      await gitSwitch(distro, entry.path, branch, remote);
      const local = remote ? branch.slice(branch.indexOf("/") + 1) : branch;
      notify({ tone: "success", title: `${entry.name} agora está em ${local}`, detail: running ? "O servidor de dev recarrega com a nova branch" : undefined });
      onChanged();
      onClose();
    } catch (err) {
      setError(splitGitError(errorMessage(err)));
    } finally {
      setBusy(null);
    }
  };

  const fetchRemotes = async () => {
    setBusy({ kind: "fetch" });
    setError(null);
    try {
      await gitFetch(distro, entry.path);
      await load();
    } catch (err) {
      setError(splitGitError(errorMessage(err)));
    } finally {
      setBusy(null);
    }
  };

  const q = filter.trim().toLowerCase();
  const local = (data?.local ?? []).filter((b) => b.name.toLowerCase().includes(q));
  const remote = (data?.remote ?? []).filter((b) => b.toLowerCase().includes(q));
  const total = (data?.local.length ?? 0) + (data?.remote.length ?? 0);
  const current = data?.local.find((b) => b.name === data.current);
  const pull = queue.itemFor(distro, entry.path);
  const pulling = pull?.status === "running" || pull?.status === "queued";
  const canPull = !!current?.upstream && !current.gone;

  return createPortal(
    <div
      ref={panel}
      onPointerDown={(e) => e.stopPropagation()}
      onClick={(e) => e.stopPropagation()}
      style={{ top: placement.top, bottom: placement.bottom, left: placement.left, width: MENU_WIDTH, maxHeight: placement.maxHeight }}
      className="fixed z-50 flex flex-col overflow-hidden rounded-lg border border-zinc-700 bg-zinc-900 shadow-2xl shadow-black/70"
    >
      <div className="flex shrink-0 items-center gap-2 border-b border-zinc-800 px-3 py-2">
        <GitBranch className="size-3.5 shrink-0 text-zinc-500" />
        <span className="min-w-0 flex-1 truncate text-[10px] font-medium tracking-wider text-zinc-500 uppercase">
          Branches · <span className="normal-case">{entry.name}</span>
        </span>
        <button
          type="button"
          onClick={fetchRemotes}
          disabled={busy !== null}
          title="Buscar branches e commits novos do remoto (git fetch)"
          className="flex items-center gap-1 rounded-full px-2 py-0.5 text-[11px] text-zinc-400 transition-colors hover:bg-zinc-800 hover:text-zinc-100 disabled:opacity-40"
        >
          <RefreshCw className={`size-3 ${busy?.kind === "fetch" ? "animate-spin" : ""}`} />
          Buscar
        </button>
      </div>

      {data && data.changes > 0 && (
        <div className="flex shrink-0 items-start gap-1.5 border-b border-zinc-800 px-3 py-1.5 text-[11px] text-amber-300">
          <AlertTriangle className="mt-px size-3 shrink-0" />
          {data.changes} arquivo{data.changes > 1 ? "s" : ""} alterado{data.changes > 1 ? "s" : ""} sem commit: o git pode recusar a troca.
        </div>
      )}
      {running && (
        <div className="shrink-0 border-b border-zinc-800 px-3 py-1.5 text-[11px] text-zinc-500">
          O projeto está rodando: o servidor de dev recarrega com a branch nova.
        </div>
      )}

      {error && (
        <div className="shrink-0 border-b border-rose-500/20 bg-rose-500/[0.04] px-3 py-2 text-[11px] text-rose-300">
          <div className="flex items-start gap-1.5">
            <AlertTriangle className="mt-px size-3 shrink-0" />
            <span>{error.title}</span>
          </div>
          {error.detail && (
            <pre className="mt-1.5 max-h-24 overflow-auto font-mono text-[10px] whitespace-pre-wrap text-rose-300/70">{error.detail}</pre>
          )}
        </div>
      )}

      {total > FILTER_FROM && (
        <label className="mx-2 mt-2 flex shrink-0 items-center gap-2 rounded-md border border-zinc-800 bg-zinc-950 px-2 py-1 focus-within:border-zinc-500">
          <Search className="size-3 text-zinc-500" />
          <input
            autoFocus
            value={filter}
            onChange={(e) => setFilter(e.target.value)}
            placeholder={`Filtrar ${total} branches`}
            spellCheck={false}
            className="w-full bg-transparent font-mono text-xs text-zinc-100 placeholder:text-zinc-600 focus:outline-none"
          />
        </label>
      )}

      <div className="min-h-0 flex-1 overflow-y-auto overscroll-contain p-1">
        {!data && !error && (
          <div className="flex justify-center py-4">
            <Loader2 className="size-4 animate-spin text-zinc-500" />
          </div>
        )}
        {data?.detached && (
          <div className="px-2.5 py-1.5 text-[11px] text-zinc-500">
            HEAD destacado em <span className="font-mono text-zinc-300">{data.detached}</span>
          </div>
        )}
        {local.length > 0 && <GroupLabel>Locais</GroupLabel>}
        {local.map((b) => {
          const isCurrent = b.name === data?.current;
          return (
            <BranchRow
              key={b.name}
              name={b.name}
              current={isCurrent}
              busy={busy?.kind === "switch" && busy.branch === b.name}
              disabled={busy !== null || isCurrent}
              onClick={() => switchTo(b.name, false)}
              meta={
                b.gone ? (
                  <span title="A branch remota foi apagada">remota apagada</span>
                ) : (
                  <>
                    {b.ahead > 0 && <span title={`${b.ahead} commits locais ainda não enviados`}>↑{b.ahead}</span>}
                    {b.behind > 0 && <span title={`${b.behind} commits novos no remoto`}>↓{b.behind}</span>}
                    {!b.upstream && <span title="Sem branch remota">local</span>}
                  </>
                )
              }
            />
          );
        })}
        {remote.length > 0 && <GroupLabel>Remotas</GroupLabel>}
        {remote.map((b) => (
          <BranchRow
            key={b}
            name={b}
            current={false}
            busy={busy?.kind === "switch" && busy.branch === b}
            disabled={busy !== null}
            onClick={() => switchTo(b, true)}
            meta={<span title="Cria a branch local acompanhando a remota">nova local</span>}
          />
        ))}
        {data && local.length + remote.length === 0 && (
          <div className="px-2.5 py-2 text-xs text-zinc-500">{q ? `Nenhuma branch com "${filter}".` : "Nenhuma branch."}</div>
        )}
      </div>

      <div className="flex shrink-0 items-center gap-2 border-t border-zinc-800 px-2 py-2">
        <button
          type="button"
          onClick={() => {
            queue.enqueue([{ distro, path: entry.path, name: entry.name }]);
            onClose();
          }}
          disabled={!canPull || pulling || busy !== null}
          title={
            !data
              ? undefined
              : data.detached
                ? "HEAD destacado: troque para uma branch para atualizar"
                : !canPull
                  ? "A branch atual não tem branch remota"
                  : `git pull --ff-only em ${data.current}`
          }
          className="flex flex-1 items-center justify-center gap-1.5 rounded-full bg-zinc-100 px-3 py-1 text-xs font-semibold text-void transition-colors hover:bg-white disabled:cursor-not-allowed disabled:bg-zinc-800 disabled:text-zinc-500"
        >
          {pulling ? <Loader2 className="size-3.5 animate-spin" /> : <CloudDownload className="size-3.5" />}
          {pulling ? "Atualizando…" : "Atualizar (git pull)"}
          {current && current.behind > 0 && !pulling && <span className="font-mono text-[10px] opacity-70">↓{current.behind}</span>}
        </button>
      </div>
    </div>,
    document.body,
  );
}

function GroupLabel({ children }: { children: string }) {
  return <div className="px-2.5 pt-2 pb-1 text-[10px] font-medium tracking-wider text-zinc-600 uppercase">{children}</div>;
}

function BranchRow({
  name,
  current,
  busy,
  disabled,
  onClick,
  meta,
}: {
  name: string;
  current: boolean;
  busy: boolean;
  disabled: boolean;
  onClick: () => void;
  meta: ReactNode;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={disabled}
      title={current ? "Branch atual" : `Trocar para ${name}`}
      className={`flex w-full items-center gap-2 rounded-md px-2.5 py-1.5 text-left transition-colors ${
        current ? "bg-zinc-800/60" : "hover:bg-zinc-800 disabled:hover:bg-transparent"
      } disabled:cursor-default`}
    >
      <span className="flex size-3.5 shrink-0 items-center justify-center">
        {busy ? (
          <Loader2 className="size-3.5 animate-spin text-zinc-300" />
        ) : current ? (
          <Check className="size-3.5 text-zinc-100" />
        ) : null}
      </span>
      <span className={`min-w-0 flex-1 truncate font-mono text-xs ${current ? "font-semibold text-zinc-100" : "text-zinc-300"}`}>{name}</span>
      <span className="flex shrink-0 items-center gap-1.5 font-mono text-[10px] text-zinc-500">{meta}</span>
    </button>
  );
}
