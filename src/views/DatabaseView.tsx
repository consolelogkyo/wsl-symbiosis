import { useState } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { AlertTriangle, Container, Database, Download, Loader2, Play, Plus, Power, RefreshCw, RotateCw, ScrollText, Square, Trash2, X } from "lucide-react";
import { BrandIcon } from "../components/BrandIcon";
import { InfoTip } from "../components/InfoTip";
import { DatabaseCard, type ContainerAction } from "../components/database/DatabaseCard";
import { CreateDatabaseDialog } from "../components/database/CreateDatabaseDialog";
import { ContainerLogsDialog, RemoveContainerDialog } from "../components/database/ContainerDialogs";
import { StateBadge } from "../components/database/common";
import { TableExplorer } from "../components/database/TableExplorer";
import { useDocker } from "../hooks/useDocker";
import { useActivityContext } from "../hooks/useActivity";
import { useProgressContext } from "../hooks/useProgress";
import { type ContainerInfo, type DbKind, DB_PRESETS, restartContainer, startContainer, stopContainer } from "../lib/docker";
import { TOOL_BRANDS } from "../lib/brands";
import { formatBytes } from "../lib/format";
import { BANNER_ERROR, BTN_DANGER, BTN_ICON, BTN_PRIMARY, CARD, LABEL } from "../lib/ui";
import { type Distro, errorMessage } from "../lib/wsl";

const DOCKER_DOWNLOAD = "https://www.docker.com/products/docker-desktop/";

interface Props {
  distro: Distro | null;
  /** Visible screen and window: polls only then. */
  active: boolean;
}

type Dialog =
  | { kind: "create"; initial?: DbKind }
  | { kind: "logs"; container: ContainerInfo }
  | { kind: "remove"; container: ContainerInfo }
  | null;

/** Bancos & Containers: 1-click databases, containers and the table explorer. */
export function DatabaseView({ distro, active }: Props) {
  const distroName = distro?.name ?? null;
  const docker = useDocker(distroName, active);
  const { notify } = useActivityContext();
  const { track } = useProgressContext();
  const [dialog, setDialog] = useState<Dialog>(null);
  const [busy, setBusy] = useState<Record<string, ContainerAction>>({});
  // The explorer follows the container by id (its card data refreshes with the list).
  const [exploring, setExploring] = useState<string | null>(null);
  const target = distroName ?? "";
  const { status, containers } = docker;

  const act = async (c: ContainerInfo, action: ContainerAction) => {
    setBusy((b) => ({ ...b, [c.id]: action }));
    try {
      const verb = action === "start" ? "Ligando" : action === "stop" ? "Parando" : "Reiniciando";
      await track(`${verb} ${c.name}`, () => (action === "start" ? startContainer : action === "stop" ? stopContainer : restartContainer)(target, c.id), {
        detail: action === "stop" ? "O container tem até 10 s para fechar sozinho" : "Docker",
      });
      await docker.refresh();
    } catch (err) {
      notify({ tone: "error", title: `Não foi possível ${action === "start" ? "ligar" : action === "stop" ? "parar" : "reiniciar"} ${c.name}`, detail: errorMessage(err) });
    } finally {
      setBusy(({ [c.id]: _, ...rest }) => rest);
    }
  };

  const explored = exploring ? containers?.find((c) => c.id === exploring) : null;
  if (exploring && explored?.database && status?.state === "ready") {
    return (
      <TableExplorer
        key={explored.id}
        distro={target}
        container={explored}
        onBack={() => {
          setExploring(null);
          void docker.refresh();
        }}
      />
    );
  }

  const databases = containers?.filter((c) => c.database) ?? [];
  const others = containers?.filter((c) => !c.database) ?? [];

  return (
    <div className="min-h-full space-y-7 p-6">
      {/* Header */}
      <div className="flex flex-wrap items-start gap-4">
        <div className="min-w-0 flex-1">
          <h1 className="text-lg font-semibold tracking-tight text-zinc-100">Bancos & Containers</h1>
          <p className="mt-1 max-w-2xl text-xs leading-relaxed text-zinc-500">
            Crie bancos de dados locais em 1 clique e veja ou edite as tabelas sem precisar de Docker ou SQL.{" "}
            <InfoTip>
              Cada banco roda num <b className="font-medium">container</b>: uma “caixinha” isolada do Docker com o banco já instalado e
              configurado. Nada é instalado no seu Windows nem no seu Linux.
            </InfoTip>
          </p>
        </div>
        {status?.state === "ready" && (
          <div className="flex items-center gap-2">
            <EnginePill source={status.source} version={status.version} />
            <button type="button" onClick={() => void docker.refresh()} className={BTN_ICON} title="Atualizar a lista">
              <RefreshCw className="size-3.5" />
            </button>
            <button type="button" onClick={() => setDialog({ kind: "create" })} className={`${BTN_PRIMARY} h-8 px-4 whitespace-nowrap`}>
              <Plus className="size-3.5" />
              Novo banco de dados
            </button>
          </div>
        )}
      </div>

      {docker.error && (
        <div className={`${BANNER_ERROR} rounded-lg border`}>
          <AlertTriangle className="mt-px size-3.5 shrink-0" />
          <span className="flex-1 break-words whitespace-pre-wrap">{docker.error}</span>
          <button type="button" onClick={() => docker.setError(null)} className="text-rose-400/70 hover:text-rose-200">
            <X className="size-3.5" />
          </button>
        </div>
      )}

      {!status ? (
        <div className="flex items-center gap-2 text-xs text-zinc-500">
          <Loader2 className="size-4 animate-spin" />
          Procurando o Docker…
        </div>
      ) : status.state !== "ready" ? (
        <EngineOff status={status} distro={distroName} starting={docker.starting} onStart={docker.startEngine} />
      ) : !containers ? (
        <div className="flex items-center gap-2 text-xs text-zinc-500">
          <Loader2 className="size-4 animate-spin" />
          Lendo os containers…
        </div>
      ) : (
        <>
          <section>
            <div className="mb-3 flex items-center gap-2">
              <Database className="size-3.5 text-zinc-500" />
              <h2 className={LABEL}>Seus bancos de dados</h2>
              <span className="font-mono text-[11px] text-zinc-600">{databases.length}</span>
            </div>
            {databases.length === 0 ? (
              <EmptyDatabases onCreate={(kind) => setDialog({ kind: "create", initial: kind })} />
            ) : (
              <div className="grid grid-cols-[repeat(auto-fill,minmax(min(100%,420px),1fr))] gap-4">
                {databases.map((c) => (
                  <DatabaseCard
                    key={c.id}
                    container={c}
                    busy={busy[c.id] ?? null}
                    onAction={(a) => void act(c, a)}
                    onManage={() => setExploring(c.id)}
                    onLogs={() => setDialog({ kind: "logs", container: c })}
                    onRemove={() => setDialog({ kind: "remove", container: c })}
                  />
                ))}
              </div>
            )}
          </section>

          {others.length > 0 && (
            <section>
              <div className="mb-3 flex items-center gap-2">
                <Container className="size-3.5 text-zinc-500" />
                <h2 className={LABEL}>Outros containers</h2>
                <span className="font-mono text-[11px] text-zinc-600">{others.length}</span>
                <InfoTip>
                  Tudo o mais que roda no Docker: APIs, filas, ferramentas e containers de outros projetos (como os do docker compose).
                </InfoTip>
              </div>
              <OtherContainers
                containers={others}
                busy={busy}
                onAction={(c, a) => void act(c, a)}
                onLogs={(c) => setDialog({ kind: "logs", container: c })}
                onRemove={(c) => setDialog({ kind: "remove", container: c })}
              />
            </section>
          )}
        </>
      )}

      {dialog?.kind === "create" && (
        <CreateDatabaseDialog
          distro={target}
          taken={containers?.map((c) => c.name) ?? []}
          initialKind={dialog.initial}
          onClose={() => setDialog(null)}
          onCreated={() => void docker.refresh()}
          onManage={async (id) => {
            setDialog(null);
            await docker.refresh();
            setExploring(id);
          }}
        />
      )}
      {dialog?.kind === "logs" && <ContainerLogsDialog distro={target} container={dialog.container} onClose={() => setDialog(null)} />}
      {dialog?.kind === "remove" && (
        <RemoveContainerDialog
          distro={target}
          container={dialog.container}
          onClose={() => setDialog(null)}
          onRemoved={(message) => {
            notify({ tone: "success", title: `${dialog.container.name} removido`, detail: message });
            setDialog(null);
            void docker.refresh();
          }}
        />
      )}
    </div>
  );
}

function EnginePill({ source, version }: { source: string | null; version: string | null }) {
  return (
    <span className="flex items-center gap-1.5 rounded-full border border-zinc-800 px-2.5 py-1 text-[11px] whitespace-nowrap text-zinc-400">
      <span className="size-1.5 rounded-full bg-emerald-400" />
      <BrandIcon icon={TOOL_BRANDS.docker} className="size-3.5" />
      Docker {version}
      <InfoTip align="right">
        {source === "wsl_native"
          ? "Usando o Docker instalado dentro da sua distro Linux (Docker Engine)."
          : "Usando o Docker Desktop do Windows. Os bancos ficam disponíveis em localhost tanto para projetos no Windows quanto no Linux."}
      </InfoTip>
    </span>
  );
}

function EngineOff(p: { status: NonNullable<ReturnType<typeof useDocker>["status"]>; distro: string | null; starting: boolean; onStart: () => void }) {
  const { status } = p;
  const installed = status.state === "stopped";
  return (
    <div className={`${CARD} mx-auto flex max-w-xl flex-col items-center gap-3 px-8 py-10 text-center`}>
      <div className="flex size-12 items-center justify-center rounded-xl border border-zinc-800 bg-zinc-950">
        <BrandIcon icon={TOOL_BRANDS.docker} className="size-6" />
      </div>
      {installed ? (
        <>
          <h2 className="text-base font-semibold text-zinc-100">{p.starting ? "Ligando o Docker…" : "O Docker está desligado"}</h2>
          <p className="text-xs leading-relaxed text-zinc-500">
            {p.starting
              ? "Na primeira vez do dia isso leva até um minuto. Pode continuar usando o app enquanto isso."
              : "O Docker é o programa que roda os bancos de dados. Ligue-o para ver, criar e gerenciar seus bancos."}
          </p>
          <button type="button" onClick={p.onStart} disabled={p.starting} className={`${BTN_PRIMARY} mt-1 h-8 px-4`}>
            {p.starting ? <Loader2 className="size-3.5 animate-spin" /> : <Power className="size-3.5" />}
            {p.starting ? "Ligando…" : "Ligar o Docker"}
          </button>
          <p className="text-[11px] text-zinc-600">
            {status.source === "wsl_native" ? `Docker da distro ${p.distro}` : "Docker Desktop (Windows)"}
            {status.source === "docker_desktop" && (
              <>
                {" "}
                <InfoTip>O app abre o Docker Desktop em segundo plano. Se ele mostrar uma janela (termos de uso, login ou atualização), conclua por lá.</InfoTip>
              </>
            )}
          </p>
        </>
      ) : (
        <>
          <h2 className="text-base font-semibold text-zinc-100">O Docker não está instalado</h2>
          <p className="text-xs leading-relaxed text-zinc-500">
            O Docker é o programa gratuito que roda os bancos de dados em “caixinhas” isoladas. Instale o Docker Desktop, abra-o uma vez e volte
            aqui: o app encontra sozinho.
          </p>
          <button type="button" onClick={() => void openUrl(DOCKER_DOWNLOAD).catch(() => {})} className={`${BTN_PRIMARY} mt-1 h-8 px-4`}>
            <Download className="size-3.5" />
            Baixar o Docker Desktop
          </button>
          {!status.distroChecked && p.distro && (
            <p className="text-[11px] leading-relaxed text-zinc-600">
              Se o Docker está instalado dentro da distro {p.distro}, ligue a distro para o app encontrá-lo.
            </p>
          )}
        </>
      )}
    </div>
  );
}

function EmptyDatabases({ onCreate }: { onCreate: (kind: DbKind) => void }) {
  return (
    <div className={`${CARD} px-6 py-8`}>
      <p className="text-sm font-medium text-zinc-100">Nenhum banco de dados ainda</p>
      <p className="mt-1 text-xs leading-relaxed text-zinc-500">Escolha um para criar agora. Em menos de um minuto ele está pronto para conectar no seu projeto.</p>
      <div className="mt-4 grid grid-cols-[repeat(auto-fill,minmax(180px,1fr))] gap-2.5">
        {DB_PRESETS.map((p) => (
          <button
            key={p.kind}
            type="button"
            onClick={() => onCreate(p.kind)}
            className="flex items-center gap-2.5 rounded-lg border border-zinc-800 bg-zinc-950 px-3 py-2.5 text-left transition-colors hover:border-zinc-600"
          >
            <BrandIcon icon={p.icon} className="size-5" />
            <span className="min-w-0">
              <span className="block text-xs font-medium text-zinc-100">{p.title}</span>
              <span className="block truncate text-[11px] text-zinc-500">{p.tagline}</span>
            </span>
          </button>
        ))}
      </div>
    </div>
  );
}

function OtherContainers(p: {
  containers: ContainerInfo[];
  busy: Record<string, ContainerAction>;
  onAction: (c: ContainerInfo, a: ContainerAction) => void;
  onLogs: (c: ContainerInfo) => void;
  onRemove: (c: ContainerInfo) => void;
}) {
  return (
    <div className={`${CARD} @container overflow-hidden`}>
      <table className="w-full text-xs">
        <thead className="border-b border-zinc-800 text-left text-[11px] tracking-wider text-zinc-500 uppercase">
          <tr>
            <th className="px-4 py-2 font-medium">Container</th>
            <th className="px-3 py-2 font-medium">Estado</th>
            <th className="px-3 py-2 font-medium @max-3xl:hidden">Portas</th>
            <th className="px-3 py-2 font-medium @max-2xl:hidden">Uso</th>
            <th className="w-36 px-4 py-2" />
          </tr>
        </thead>
        <tbody>
          {p.containers.map((c) => {
            const running = c.state === "running";
            const busy = p.busy[c.id] ?? null;
            return (
              <tr key={c.id} className="border-b border-zinc-800/70 last:border-0 hover:bg-zinc-950/40">
                <td className="max-w-0 px-4 py-2">
                  <div className="truncate font-medium text-zinc-100" title={c.name}>
                    {c.name}
                    {c.composeProject && <span className="ml-2 text-[10px] font-normal text-zinc-600">compose · {c.composeProject}</span>}
                  </div>
                  <div className="truncate font-mono text-[11px] text-zinc-500" title={c.image}>
                    {c.image}
                  </div>
                </td>
                <td className="px-3 py-2">
                  <StateBadge state={c.state} title={c.status} />
                </td>
                <td className="px-3 py-2 font-mono text-[11px] text-zinc-400 @max-3xl:hidden">
                  {c.ports.length === 0 ? "—" : c.ports.map((pm) => `${pm.hostPort}→${pm.containerPort}`).join(", ")}
                </td>
                <td className="px-3 py-2 font-mono text-[11px] whitespace-nowrap text-zinc-400 tabular-nums @max-2xl:hidden">
                  {running && c.stats ? `${formatBytes(c.stats.memoryUsed)} · ${c.stats.cpuPercent.toFixed(1)}%` : "—"}
                </td>
                <td className="px-4 py-2">
                  <div className="flex items-center justify-end gap-1">
                    {running ? (
                      <>
                        <button type="button" onClick={() => p.onAction(c, "stop")} disabled={busy !== null} className={BTN_ICON} title="Parar">
                          {busy === "stop" ? <Loader2 className="size-3.5 animate-spin" /> : <Square className="size-3.5" />}
                        </button>
                        <button type="button" onClick={() => p.onAction(c, "restart")} disabled={busy !== null} className={BTN_ICON} title="Reiniciar">
                          {busy === "restart" ? <Loader2 className="size-3.5 animate-spin" /> : <RotateCw className="size-3.5" />}
                        </button>
                      </>
                    ) : (
                      <button type="button" onClick={() => p.onAction(c, "start")} disabled={busy !== null} className={BTN_ICON} title="Ligar">
                        {busy === "start" ? <Loader2 className="size-3.5 animate-spin" /> : <Play className="size-3.5" />}
                      </button>
                    )}
                    <button type="button" onClick={() => p.onLogs(c)} className={BTN_ICON} title="Ver logs">
                      <ScrollText className="size-3.5" />
                    </button>
                    <button type="button" onClick={() => p.onRemove(c)} disabled={busy !== null} className={`${BTN_DANGER} px-2`} title="Remover">
                      <Trash2 className="size-3.5" />
                    </button>
                  </div>
                </td>
              </tr>
            );
          })}
        </tbody>
      </table>
    </div>
  );
}

