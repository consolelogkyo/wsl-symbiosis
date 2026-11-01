import { useEffect, useMemo, useState } from "react";
import { CheckCircle2, Circle, Database, Eye, EyeOff, KeyRound, Loader2, Plus, Table2 } from "lucide-react";
import { BrandIcon } from "../BrandIcon";
import { ProgressBar } from "../ProgressBar";
import { InfoTip } from "../InfoTip";
import {
  type CreatedDatabase,
  type DatabaseProgress,
  type DbKind,
  type DbPreset,
  DB_PRESETS,
  containerNameError,
  createDatabaseContainer,
  dbIdentifierError,
  generatePassword,
  getDatabaseDefaults,
  onDatabaseProgress,
  passwordError,
} from "../../lib/docker";
import { formatDuration } from "../../lib/backup";
import { errorMessage } from "../../lib/wsl";
import { BTN_GHOST, BTN_ICON, BTN_PRIMARY, FIELD, INPUT } from "../../lib/ui";
import { CopyButton, DialogShell, ErrorBox } from "./common";

interface Props {
  distro: string;
  /** Names already used by containers (the backend checks again). */
  taken: string[];
  initialKind?: DbKind;
  onClose: () => void;
  onCreated: () => void;
  onManage: (id: string) => void;
}

type Phase =
  | { step: "choose" }
  | { step: "form" }
  | { step: "running"; startedAt: number }
  | { step: "done"; result: CreatedDatabase }
  | { step: "error"; message: string };

const STAGES: { id: DatabaseProgress["stage"]; label: string }[] = [
  { id: "pull", label: "Baixando o banco (só na primeira vez)" },
  { id: "create", label: "Criando o container" },
  { id: "wait", label: "Esperando o banco ficar pronto" },
];

/** "+ Novo banco de dados": pick an engine, confirm smart defaults, done. */
export function CreateDatabaseDialog({ distro, taken, initialKind, onClose, onCreated, onManage }: Props) {
  const [preset, setPreset] = useState<DbPreset | null>(DB_PRESETS.find((p) => p.kind === initialKind) ?? null);
  const [phase, setPhase] = useState<Phase>(preset ? { step: "form" } : { step: "choose" });
  const [loadingDefaults, setLoadingDefaults] = useState(false);
  const [name, setName] = useState("");
  const [database, setDatabase] = useState("");
  const [user, setUser] = useState("");
  const [password, setPassword] = useState("");
  const [showPassword, setShowPassword] = useState(false);
  const [port, setPort] = useState("");
  const [image, setImage] = useState("");
  const [progress, setProgress] = useState<DatabaseProgress | null>(null);
  const [now, setNow] = useState(Date.now());
  const running = phase.step === "running";

  useEffect(() => {
    if (!preset) return;
    setLoadingDefaults(true);
    getDatabaseDefaults(distro, preset.kind)
      .then((d) => {
        setName(d.name);
        setDatabase(d.database ?? "");
        setUser(d.user ?? "");
        setPassword(d.password ?? "");
        setPort(String(d.port));
        setImage(d.image);
      })
      .catch((err) => setPhase({ step: "error", message: errorMessage(err) }))
      .finally(() => setLoadingDefaults(false));
  }, [distro, preset]);

  // Subscribed from the start: the first event can arrive right after the click.
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let gone = false;
    onDatabaseProgress(setProgress).then((u) => (gone ? u() : (unlisten = u)));
    return () => {
      gone = true;
      unlisten?.();
    };
  }, []);

  useEffect(() => {
    if (!running) return;
    const id = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(id);
  }, [running]);

  const portNumber = Number(port);
  const errors = useMemo(() => {
    if (!preset) return {};
    return {
      name: containerNameError(name, taken),
      database: preset.fields.database ? dbIdentifierError(database, "Nome do banco") : null,
      user: preset.fields.user ? dbIdentifierError(user, "Usuário") : null,
      password: passwordError(password, preset.fields.password === "required"),
      port: !/^\d+$/.test(port) || portNumber < 1024 || portNumber > 65535 ? "Use um número entre 1024 e 65535" : null,
    };
  }, [preset, name, database, user, password, port, portNumber, taken]);
  const valid = !Object.values(errors).some(Boolean);

  const create = async () => {
    if (!preset) return;
    setProgress(null);
    setPhase({ step: "running", startedAt: Date.now() });
    try {
      const result = await createDatabaseContainer(distro, {
        kind: preset.kind,
        name,
        database: preset.fields.database ? database : null,
        user: preset.fields.user ? user : null,
        password: password || null,
        port: portNumber,
      });
      onCreated();
      setPhase({ step: "done", result });
    } catch (err) {
      onCreated();
      setPhase({ step: "error", message: errorMessage(err) });
    }
  };

  const stageIndex = progress ? STAGES.findIndex((s) => s.id === progress.stage) : -1;
  const urlPreview = preset ? previewUrl(preset.kind, user, password, portNumber, database) : "";

  return (
    <DialogShell
      icon={preset ? <BrandIcon icon={preset.icon} className="size-5" /> : <Database className="size-5 text-zinc-100" />}
      eyebrow="Novo banco de dados · 1 clique"
      title={preset ? `Criar ${preset.title}` : "Qual banco você quer?"}
      description={
        preset
          ? `${preset.tagline} ${preset.goodFor}`
          : "O app baixa, configura e liga o banco para você, já pronto para conectar no seu projeto. Você não precisa saber Docker."
      }
      busy={running}
      width="w-[min(640px,100%)]"
      onClose={onClose}
      footer={
        phase.step === "choose" ? (
          <button type="button" onClick={onClose} className={`${BTN_GHOST} h-8`}>
            Cancelar
          </button>
        ) : phase.step === "form" ? (
          <>
            <button
              type="button"
              onClick={() => {
                setPreset(null);
                setPhase({ step: "choose" });
              }}
              className={`${BTN_GHOST} h-8`}
            >
              Voltar
            </button>
            <button type="button" onClick={create} disabled={!valid || loadingDefaults} className={`${BTN_PRIMARY} h-8 px-4`}>
              <Plus className="size-3.5" />
              Criar banco
            </button>
          </>
        ) : phase.step === "done" ? (
          <>
            {phase.result.database?.explorer && (
              <button type="button" onClick={() => onManage(phase.result.id)} className={`${BTN_GHOST} h-8`}>
                <Table2 className="size-3.5" />
                Gerenciar tabelas & dados
              </button>
            )}
            <button type="button" onClick={onClose} className={`${BTN_PRIMARY} h-8 px-4`}>
              Concluir
            </button>
          </>
        ) : phase.step === "error" ? (
          <>
            <button type="button" onClick={() => setPhase(preset ? { step: "form" } : { step: "choose" })} className={`${BTN_GHOST} h-8`}>
              Voltar
            </button>
            <button type="button" onClick={onClose} className={`${BTN_PRIMARY} h-8 px-4`}>
              Fechar
            </button>
          </>
        ) : null
      }
    >
      <div className="space-y-4 px-6 py-5">
        {phase.step === "choose" && (
          <div className="grid grid-cols-2 gap-3">
            {DB_PRESETS.map((p) => (
              <button
                key={p.kind}
                type="button"
                onClick={() => {
                  setPreset(p);
                  setPhase({ step: "form" });
                }}
                className="group flex flex-col items-start gap-2 rounded-lg border border-zinc-800 bg-zinc-900 p-4 text-left transition-colors hover:border-zinc-600"
              >
                <div className="flex items-center gap-2.5">
                  <BrandIcon icon={p.icon} className="size-6" />
                  <span className="text-sm font-semibold text-zinc-100">{p.title}</span>
                </div>
                <span className="text-xs leading-relaxed text-zinc-300">{p.tagline}</span>
                <span className="text-[11px] leading-relaxed text-zinc-500">{p.goodFor}</span>
              </button>
            ))}
          </div>
        )}

        {phase.step === "form" && preset && (
          <>
            {loadingDefaults ? (
              <div className="flex items-center gap-2 py-6 text-xs text-zinc-500">
                <Loader2 className="size-4 animate-spin" />
                Procurando um nome e uma porta livres…
              </div>
            ) : (
              <>
                <div className="grid grid-cols-2 gap-x-4 gap-y-3">
                  <Field
                    label="Nome do banco no Docker"
                    info="É o nome da “caixinha” (container) onde o banco roda. Aparece aqui e no Docker Desktop."
                    error={errors.name}
                    className="col-span-2"
                  >
                    <input value={name} onChange={(e) => setName(e.target.value.trim())} className={`${INPUT} py-0.5 font-mono text-sm`} spellCheck={false} />
                  </Field>
                  {preset.fields.database && (
                    <Field label="Nome do banco de dados" info="O banco que seu projeto vai usar, dentro do servidor. Ex.: app_db, loja." error={errors.database}>
                      <input value={database} onChange={(e) => setDatabase(e.target.value.trim())} className={`${INPUT} py-0.5 font-mono text-sm`} spellCheck={false} />
                    </Field>
                  )}
                  {preset.fields.user && (
                    <Field label="Usuário" info="O login que seu projeto usa para entrar no banco." error={errors.user}>
                      <input value={user} onChange={(e) => setUser(e.target.value.trim())} className={`${INPUT} py-0.5 font-mono text-sm`} spellCheck={false} />
                    </Field>
                  )}
                  <Field
                    label={preset.fields.password === "optional" ? "Senha (opcional)" : "Senha"}
                    info={
                      preset.fields.password === "optional"
                        ? "Sem senha, qualquer programa deste computador pode usar o Redis. Ele só aceita conexões do próprio PC."
                        : "Para um banco de desenvolvimento uma senha simples serve: ele só aceita conexões deste computador."
                    }
                    error={errors.password}
                  >
                    <input
                      type={showPassword ? "text" : "password"}
                      value={password}
                      onChange={(e) => setPassword(e.target.value)}
                      className={`${INPUT} py-0.5 font-mono text-sm`}
                      spellCheck={false}
                    />
                    <button type="button" onClick={() => setShowPassword((s) => !s)} className={BTN_ICON} title={showPassword ? "Esconder" : "Mostrar"}>
                      {showPassword ? <EyeOff className="size-3.5" /> : <Eye className="size-3.5" />}
                    </button>
                    <button
                      type="button"
                      onClick={() => {
                        setPassword(generatePassword());
                        setShowPassword(true);
                      }}
                      className={BTN_ICON}
                      title="Gerar uma senha forte"
                    >
                      <KeyRound className="size-3.5" />
                    </button>
                  </Field>
                  <Field
                    label="Porta"
                    info="O “número da porta” por onde seu projeto conversa com o banco. Já sugerimos uma que está livre; se a padrão estiver ocupada, usamos a próxima."
                    error={errors.port}
                  >
                    <input value={port} onChange={(e) => setPort(e.target.value.replace(/\D/g, ""))} inputMode="numeric" className={`${INPUT} py-0.5 font-mono text-sm`} />
                  </Field>
                </div>

                <div className="rounded-lg border border-zinc-800 bg-zinc-900 px-4 py-3">
                  <div className="text-[11px] text-zinc-500">Seu projeto vai se conectar com</div>
                  <div className="mt-1 truncate font-mono text-xs text-zinc-200" title={urlPreview}>
                    {urlPreview}
                  </div>
                </div>

                <ul className="space-y-1.5 text-xs leading-relaxed text-zinc-500">
                  <li className="flex items-start gap-1.5">
                    <span>•</span>
                    <span>
                      Seus dados ficam guardados mesmo se você parar o banco ou desligar o computador.{" "}
                      <InfoTip>
                        O app cria um volume do Docker (<span className="font-mono">symbiosis_{name || "nome"}_data</span>) só para os arquivos
                        do banco. Ele só é apagado se você pedir ao remover o banco.
                      </InfoTip>
                    </span>
                  </li>
                  <li className="flex items-start gap-1.5">
                    <span>•</span>
                    <span>
                      O banco liga sozinho junto com o Docker, até você pará-lo.{" "}
                      <InfoTip>
                        Política de reinício <span className="font-mono">unless-stopped</span> e porta aberta só em{" "}
                        <span className="font-mono">127.0.0.1</span>: ninguém da sua rede alcança o banco. Imagem:{" "}
                        <span className="font-mono">{image}</span>.
                      </InfoTip>
                    </span>
                  </li>
                </ul>
              </>
            )}
          </>
        )}

        {phase.step === "running" && (
          <div className="space-y-4 py-2">
            <ol className="space-y-2.5">
              {STAGES.map((s, i) => {
                const done = stageIndex > i;
                const current = stageIndex === i || (stageIndex === -1 && i === 0);
                return (
                  <li key={s.id} className="flex items-center gap-3">
                    {done ? (
                      <CheckCircle2 className="size-4 shrink-0 text-emerald-400" />
                    ) : current ? (
                      <Loader2 className="size-4 shrink-0 animate-spin text-zinc-200" />
                    ) : (
                      <Circle className="size-4 shrink-0 text-zinc-700" />
                    )}
                    <span className={`text-sm ${done ? "text-zinc-400" : current ? "font-medium text-zinc-100" : "text-zinc-600"}`}>{s.label}</span>
                  </li>
                );
              })}
            </ol>
            {/* Downloading: real share of the image layers. Creating / waiting: no measure, moving bar. */}
            <div className="space-y-1.5">
              <div className="flex items-center justify-between text-[11px] text-zinc-500">
                <span>{progress?.stage === "pull" ? "Download da imagem" : progress?.stage === "wait" ? "O banco está se preparando" : "Preparando"}</span>
                {progress?.stage === "pull" && progress.percent !== null && (
                  <span className="font-mono text-zinc-300 tabular-nums">{Math.round(progress.percent)}%</span>
                )}
              </div>
              <ProgressBar percent={progress?.stage === "pull" ? progress.percent : null} className="h-2" />
            </div>
            {progress?.stage === "pull" && progress.detail && (
              <p className="truncate rounded-md border border-zinc-800 bg-zinc-900 px-3 py-2 font-mono text-[11px] text-zinc-500" title={progress.detail}>
                {progress.detail}
              </p>
            )}
            <p className="font-mono text-xs text-zinc-500 tabular-nums">{formatDuration((now - phase.startedAt) / 1000)}</p>
            <p className="text-xs leading-relaxed text-zinc-500">
              O download acontece só na primeira vez de cada tipo de banco e pode levar alguns minutos, conforme a internet.
            </p>
          </div>
        )}

        {phase.step === "done" && (
          <div className="space-y-4">
            <p className="flex items-start gap-2 text-sm leading-relaxed text-zinc-300">
              <CheckCircle2 className="mt-0.5 size-4 shrink-0 text-emerald-400" />
              <span>
                <span className="font-medium text-zinc-100">{phase.result.name}</span>{" "}
                {phase.result.ready
                  ? "está pronto para usar."
                  : "foi criado e ainda está terminando de iniciar. Em alguns segundos ele aceita conexões."}
              </span>
            </p>
            {phase.result.database?.connectionUrl && (
              <div className="rounded-lg border border-zinc-800 bg-zinc-900 p-4">
                <div className="text-[11px] text-zinc-500">Cole no .env do seu projeto</div>
                <div className="mt-1 truncate font-mono text-xs text-zinc-200" title={phase.result.database.connectionUrl}>
                  {phase.result.database.connectionUrl}
                </div>
                <div className="mt-3 flex flex-wrap gap-2">
                  <CopyButton value={phase.result.database.connectionUrl} label={`Copiar ${preset?.envName ?? "DATABASE_URL"}`} />
                  {phase.result.database.envSnippet && <CopyButton value={phase.result.database.envSnippet} label="Copiar formato .env" />}
                </div>
              </div>
            )}
          </div>
        )}

        {phase.step === "error" && <ErrorBox>{phase.message}</ErrorBox>}
      </div>
    </DialogShell>
  );
}

function Field(p: { label: string; info?: string; error?: string | null; className?: string; children: React.ReactNode }) {
  return (
    <label className={`block ${p.className ?? ""}`}>
      <span className="mb-1 flex items-center gap-1.5 text-[11px] text-zinc-400">
        {p.label}
        {p.info && <InfoTip>{p.info}</InfoTip>}
      </span>
      <span className={`${FIELD} ${p.error ? "border-rose-500/50" : ""}`}>{p.children}</span>
      {p.error && <span className="mt-1 block text-[11px] text-rose-300">{p.error}</span>}
    </label>
  );
}

/** Same shape as `database_info` (docker.rs), password masked. */
function previewUrl(kind: DbKind, user: string, password: string, port: number, database: string): string {
  const auth = (u: string) => (password ? `${encodeURIComponent(u)}:••••••@` : u ? `${encodeURIComponent(u)}@` : "");
  const p = Number.isFinite(port) && port > 0 ? port : "…";
  switch (kind) {
    case "postgres":
      return `postgresql://${auth(user || "postgres")}localhost:${p}/${database || "app_db"}`;
    case "mysql":
    case "mariadb":
      return `mysql://${auth(user || "root")}localhost:${p}/${database || "app_db"}`;
    case "redis":
      return `redis://${password ? ":••••••@" : ""}localhost:${p}`;
    case "mongo":
      return `mongodb://${auth(user || "admin")}localhost:${p}/${database || "app_db"}?authSource=admin`;
  }
}
