import { useState } from "react";
import { Eye, EyeOff, Loader2, Play, RotateCw, ScrollText, Square, Table2, Trash2 } from "lucide-react";
import { BrandIcon } from "../BrandIcon";
import { InfoTip } from "../InfoTip";
import { type ContainerInfo, DB_ICONS, DB_PRESETS, DB_TITLES, databasePort } from "../../lib/docker";
import { formatBytes } from "../../lib/format";
import { BTN_DANGER, BTN_ICON, BTN_PRIMARY, CARD, LABEL } from "../../lib/ui";
import { CopyButton, StateBadge, maskUrl } from "./common";

export type ContainerAction = "start" | "stop" | "restart";

interface Props {
  container: ContainerInfo;
  /** Action running on this container. */
  busy: ContainerAction | null;
  onAction: (action: ContainerAction) => void;
  onManage: () => void;
  onLogs: () => void;
  onRemove: () => void;
}

/** One database container: state, usage and "Conectar no seu projeto". */
export function DatabaseCard({ container: c, busy, onAction, onManage, onLogs, onRemove }: Props) {
  const db = c.database!;
  const running = c.state === "running";
  const [reveal, setReveal] = useState(false);
  const port = databasePort(c);
  const envName = DB_PRESETS.find((p) => p.kind === db.kind)?.envName ?? "DATABASE_URL";

  return (
    <div className={`${CARD} flex flex-col`}>
      <div className="flex items-start gap-3 p-4">
        <div className="flex size-10 shrink-0 items-center justify-center rounded-lg border border-zinc-800 bg-zinc-950">
          <BrandIcon icon={DB_ICONS[db.kind]} className="size-5" />
        </div>
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2">
            <span className="truncate text-sm font-semibold text-zinc-100" title={c.name}>
              {c.name}
            </span>
            {c.managed && (
              <span className="shrink-0 rounded-full border border-zinc-800 px-1.5 py-px text-[10px] text-zinc-500" title="Criado pelo WSL Symbiosis">
                criado aqui
              </span>
            )}
            <span className="flex-1" />
            <StateBadge state={c.state} title={c.status} />
          </div>
          <div className="mt-1 flex flex-wrap items-center gap-x-2 gap-y-0.5 text-xs text-zinc-500">
            <span className="text-zinc-300">{DB_TITLES[db.kind]}</span>
            <span className="truncate font-mono text-[11px]" title={c.image}>
              {c.image}
            </span>
            {port !== null && (
              <span className="font-mono text-[11px] whitespace-nowrap">
                porta <span className="text-zinc-300">{port}</span>
              </span>
            )}
          </div>
          {running && c.stats && (
            <div className="mt-1 font-mono text-[11px] whitespace-nowrap text-zinc-500 tabular-nums">
              RAM <span className="text-zinc-300">{formatBytes(c.stats.memoryUsed)}</span> · CPU{" "}
              <span className="text-zinc-300">{c.stats.cpuPercent.toFixed(1)}%</span>
            </div>
          )}
        </div>
      </div>

      <div className="mx-4 border-t border-zinc-800 pt-3 pb-4">
        <div className="mb-2 flex items-center gap-1.5">
          <span className={LABEL}>Conectar no seu projeto</span>
          <InfoTip>
            Cole o link no arquivo <span className="font-mono">.env</span> do seu projeto (Prisma, Drizzle, Next.js, NestJS…). Ele diz ao
            projeto onde o banco está (<span className="font-mono">localhost</span>, nesta porta), com qual usuário e senha. Funciona tanto
            para projetos no Windows quanto no Linux.
          </InfoTip>
        </div>
        {db.connectionUrl ? (
          <>
            <div className="flex items-center gap-1 rounded-md border border-zinc-800 bg-zinc-950 py-1 pr-1 pl-2.5">
              <span className="min-w-0 flex-1 truncate font-mono text-[11px] text-zinc-300" title={reveal ? db.connectionUrl : undefined}>
                {reveal ? db.connectionUrl : maskUrl(db.connectionUrl, db.password)}
              </span>
              {db.password && (
                <button type="button" onClick={() => setReveal((r) => !r)} className={BTN_ICON} title={reveal ? "Esconder a senha" : "Mostrar a senha"}>
                  {reveal ? <EyeOff className="size-3.5" /> : <Eye className="size-3.5" />}
                </button>
              )}
            </div>
            <div className="mt-2 flex flex-wrap gap-2">
              <CopyButton value={db.connectionUrl} label={`Copiar ${envName}`} />
              {db.envSnippet && <CopyButton value={db.envSnippet} label="Copiar formato .env" title={db.envSnippet} />}
            </div>
          </>
        ) : (
          <p className="text-xs leading-relaxed text-zinc-500">
            Este banco não está aberto em nenhuma porta do Windows: só outros containers conseguem falar com ele.
          </p>
        )}
      </div>

      <div className="mt-auto flex items-center gap-1.5 border-t border-zinc-800 px-4 py-3">
        {running ? (
          db.explorer ? (
            <button type="button" onClick={onManage} className={`${BTN_PRIMARY} whitespace-nowrap`}>
              <Table2 className="size-3.5" />
              Gerenciar tabelas & dados
            </button>
          ) : (
            <span className="flex items-center gap-1.5 text-[11px] text-zinc-500">
              Ver dados: MongoDB Compass
              <InfoTip>
                O explorador de tabelas do app ainda não abre MongoDB. Instale o MongoDB Compass (gratuito) e cole o link acima em “New
                connection”.
              </InfoTip>
            </span>
          )
        ) : (
          <button type="button" onClick={() => onAction("start")} disabled={busy !== null} className={`${BTN_PRIMARY} whitespace-nowrap`}>
            {busy === "start" ? <Loader2 className="size-3.5 animate-spin" /> : <Play className="size-3.5" />}
            Ligar
          </button>
        )}
        <span className="flex-1" />
        {running && (
          <>
            <button type="button" onClick={() => onAction("stop")} disabled={busy !== null} className={BTN_ICON} title="Parar (os dados ficam guardados)">
              {busy === "stop" ? <Loader2 className="size-3.5 animate-spin" /> : <Square className="size-3.5" />}
            </button>
            <button type="button" onClick={() => onAction("restart")} disabled={busy !== null} className={BTN_ICON} title="Reiniciar">
              {busy === "restart" ? <Loader2 className="size-3.5 animate-spin" /> : <RotateCw className="size-3.5" />}
            </button>
          </>
        )}
        <button type="button" onClick={onLogs} className={BTN_ICON} title="Ver o que o banco escreveu (logs)">
          <ScrollText className="size-3.5" />
        </button>
        <button type="button" onClick={onRemove} disabled={busy !== null} className={`${BTN_DANGER} px-2`} title="Remover o banco">
          <Trash2 className="size-3.5" />
        </button>
      </div>
    </div>
  );
}
