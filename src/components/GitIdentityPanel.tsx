import { useCallback, useEffect, useState } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import {
  AlertTriangle,
  Check,
  CheckCircle2,
  Copy,
  ExternalLink,
  KeyRound,
  Loader2,
  MonitorDown,
  Play,
  PlugZap,
  ShieldCheck,
  Sparkles,
  XCircle,
} from "lucide-react";
import { BTN_GHOST, BTN_PRIMARY, FIELD, INPUT } from "../lib/ui";
import { errorMessage, startDistro } from "../lib/wsl";
import {
  GITHUB_KEYS_URL,
  type GitIdentityStatus,
  type SshTestResult,
  emailError,
  fixSshPermissions,
  generateSshKey,
  getGitIdentity,
  importWindowsSshKeys,
  saveGitIdentity,
  testSshGithub,
} from "../lib/ssh";
import { InfoTip } from "./InfoTip";

/** Loads and refreshes the Git identity / SSH status of one distro. */
export function useGitIdentity(distro: string | null) {
  const [status, setStatus] = useState<GitIdentityStatus | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);

  const reload = useCallback(async () => {
    if (!distro) return;
    setLoading(true);
    try {
      setStatus(await getGitIdentity(distro));
      setError(null);
    } catch (err) {
      setStatus(null);
      setError(errorMessage(err));
    } finally {
      setLoading(false);
    }
  }, [distro]);

  useEffect(() => {
    setStatus(null);
    setError(null);
    void reload();
  }, [reload]);

  return { status, error, loading, reload, setStatus };
}

type Identity = ReturnType<typeof useGitIdentity>;

/** The distro must be on to read ~/.gitconfig and ~/.ssh (the app never boots it on its own). */
function StoppedNotice({ distro, error, onStarted }: { distro: string; error: string; onStarted: () => void }) {
  const [starting, setStarting] = useState(false);
  const stopped = /não está em execução/.test(error);
  if (!stopped) {
    return (
      <div className="flex items-start gap-2 rounded-lg border border-rose-500/20 bg-rose-500/[0.04] px-4 py-3 text-xs text-rose-300">
        <AlertTriangle className="mt-px size-3.5 shrink-0" />
        <span className="break-words">{error}</span>
      </div>
    );
  }
  return (
    <div className="flex items-center gap-3 rounded-lg border border-dashed border-zinc-800 px-4 py-3">
      <p className="flex-1 text-xs leading-relaxed text-zinc-400">
        A distro <span className="font-medium text-zinc-100">{distro}</span> está desligada. Ligue para ver e configurar o Git dela.
      </p>
      <button
        type="button"
        disabled={starting}
        onClick={async () => {
          setStarting(true);
          try {
            await startDistro(distro);
          } finally {
            setStarting(false);
            onStarted();
          }
        }}
        className={`${BTN_PRIMARY} shrink-0`}
      >
        {starting ? <Loader2 className="size-3.5 animate-spin" /> : <Play className="size-3.5" />}
        Ligar a distro
      </button>
    </div>
  );
}

/** "Quem é você nos commits": git config --global user.name / user.email. */
export function GitIdentityCard({ distro, identity }: { distro: string; identity: Identity }) {
  const { status, error, reload } = identity;
  const [name, setName] = useState("");
  const [email, setEmail] = useState("");
  const [saving, setSaving] = useState(false);
  const [saved, setSaved] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);

  useEffect(() => {
    setName(status?.name ?? "");
    setEmail(status?.email ?? "");
  }, [status?.name, status?.email]);

  if (error) return <StoppedNotice distro={distro} error={error} onStarted={reload} />;
  if (!status) return <div className="h-28 animate-pulse rounded-lg border border-zinc-800 bg-zinc-900" />;

  const invalidEmail = emailError(email);
  const changed = name.trim() !== (status.name ?? "") || email.trim() !== (status.email ?? "");
  const canSave = changed && name.trim() !== "" && email.trim() !== "" && !invalidEmail && status.gitInstalled;

  const save = async () => {
    setSaving(true);
    setSaveError(null);
    try {
      await saveGitIdentity(distro, name, email);
      identity.setStatus({ ...status, name: name.trim(), email: email.trim() });
      setSaved(true);
      window.setTimeout(() => setSaved(false), 2500);
    } catch (err) {
      setSaveError(errorMessage(err));
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="rounded-lg border border-zinc-800 bg-zinc-900 px-4 py-3.5">
      <p className="text-xs leading-relaxed text-zinc-500">
        Cada commit (cada versão do código que você salva no Git) leva um nome e um e-mail. É assim que o GitHub mostra quem fez
        cada alteração.{" "}
        <InfoTip>
          Grava <span className="font-mono">git config --global user.name</span> e <span className="font-mono">user.email</span> na
          distro {distro}. Dica: use o mesmo e-mail da sua conta do GitHub para os commits aparecerem no seu perfil.
        </InfoTip>
      </p>
      {!status.gitInstalled ? (
        <p className="mt-3 text-xs text-amber-300">O Git ainda não está instalado nesta distro. O Setup instala para você (Configurações → Configuração inicial).</p>
      ) : (
        <>
          <div className="mt-3 grid grid-cols-2 gap-2.5">
            <label className="block">
              <span className="mb-1 block text-[11px] text-zinc-400">Nome</span>
              <span className={FIELD}>
                <input value={name} onChange={(e) => setName(e.target.value)} placeholder="Seu nome" className={`${INPUT} py-0.5 text-sm`} />
              </span>
            </label>
            <label className="block">
              <span className="mb-1 block text-[11px] text-zinc-400">E-mail</span>
              <span className={`${FIELD} ${invalidEmail ? "border-rose-500/50" : ""}`}>
                <input
                  value={email}
                  onChange={(e) => setEmail(e.target.value)}
                  placeholder="voce@exemplo.com"
                  className={`${INPUT} py-0.5 text-sm`}
                />
              </span>
            </label>
          </div>
          <div className="mt-2.5 flex items-center gap-3">
            <span className="min-w-0 flex-1 truncate text-[11px]">
              {saveError ? (
                <span className="text-rose-300">{saveError}</span>
              ) : invalidEmail ? (
                <span className="text-rose-300">{invalidEmail}</span>
              ) : saved ? (
                <span className="flex items-center gap-1.5 text-zinc-300">
                  <CheckCircle2 className="size-3.5 text-emerald-400" /> Salvo: os próximos commits já usam esses dados
                </span>
              ) : !status.name || !status.email ? (
                <span className="text-amber-300">Ainda não configurado: o Git recusa commits sem nome e e-mail.</span>
              ) : null}
            </span>
            <button type="button" onClick={save} disabled={!canSave || saving} className={`${BTN_PRIMARY} shrink-0`}>
              {saving ? <Loader2 className="size-3.5 animate-spin" /> : <Check className="size-3.5" />}
              Salvar
            </button>
          </div>
        </>
      )}
    </div>
  );
}

/** "Chave de acesso SSH": show/copy the key, import from Windows, generate, fix permissions, test. */
export function SshKeyCard({ distro, identity }: { distro: string; identity: Identity }) {
  const { status, error, reload } = identity;
  const [busy, setBusy] = useState<"import" | "generate" | "fix" | "test" | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [test, setTest] = useState<SshTestResult | null>(null);
  const [copied, setCopied] = useState(false);

  useEffect(() => {
    setTest(null);
    setNotice(null);
    setActionError(null);
  }, [distro]);

  if (error) return <StoppedNotice distro={distro} error={error} onStarted={reload} />;
  if (!status) return <div className="h-36 animate-pulse rounded-lg border border-zinc-800 bg-zinc-900" />;

  const act = async (kind: NonNullable<typeof busy>, task: () => Promise<string | null>) => {
    setBusy(kind);
    setActionError(null);
    setNotice(null);
    try {
      setNotice(await task());
      if (kind !== "test") await reload();
    } catch (err) {
      setActionError(errorMessage(err));
    } finally {
      setBusy(null);
    }
  };

  const key = status.keys[0] ?? null;
  const hasEd25519 = status.keys.some((k) => k.name === "id_ed25519");
  const importable = status.windowsKeys.filter((k) => !k.existsInLinux);
  const generateEmail = status.email ?? "";

  const generateButton = !hasEd25519 && (
    <button
      type="button"
      onClick={() =>
        act("generate", async () => {
          await generateSshKey(distro, generateEmail);
          return "Chave criada. Agora copie a chave pública e cadastre no GitHub.";
        })
      }
      disabled={busy !== null || !generateEmail || !status.keygenInstalled}
      title={!generateEmail ? "Salve o seu e-mail acima primeiro: ele vai junto na chave, para você reconhecê-la no GitHub" : undefined}
      className={`${key || importable.length ? BTN_GHOST : BTN_PRIMARY} shrink-0`}
    >
      {busy === "generate" ? <Loader2 className="size-3.5 animate-spin" /> : <Sparkles className="size-3.5" />}
      Gerar nova chave SSH em 1 clique
    </button>
  );

  return (
    <div className="space-y-2.5">
      <div className="rounded-lg border border-zinc-800 bg-zinc-900 px-4 py-3.5">
        <p className="text-xs leading-relaxed text-zinc-500">
          A chave SSH funciona como um crachá: o GitHub e o GitLab reconhecem este computador e deixam você baixar e enviar código sem
          digitar senha.{" "}
          <InfoTip>
            Fica em <span className="font-mono">~/.ssh</span> da distro {distro}. A parte <b className="font-semibold text-zinc-100">pública</b>{" "}
            (que termina em .pub) pode ser compartilhada e é a que você cola no GitHub. A parte privada nunca sai do seu computador.
          </InfoTip>
        </p>

        {key ? (
          <div className="mt-3 space-y-3">
            <div className="flex items-center gap-2.5">
              <KeyRound className="size-4 shrink-0 text-zinc-400" />
              <span className="text-sm font-medium text-zinc-100">Chave {key.kind ?? ""} encontrada</span>
              <span className="font-mono text-[11px] text-zinc-500">~/.ssh/{key.name}</span>
              {key.fingerprint && (
                <span className="ml-auto truncate font-mono text-[10px] text-zinc-600" title={key.fingerprint}>
                  {key.fingerprint}
                </span>
              )}
            </div>
            {key.publicKey ? (
              <div className="flex items-center gap-2 rounded-md border border-zinc-800 bg-zinc-950 px-3 py-2">
                <code className="min-w-0 flex-1 truncate font-mono text-[11px] text-zinc-400" title={key.publicKey}>
                  {key.publicKey}
                </code>
                <button
                  type="button"
                  onClick={async () => {
                    await navigator.clipboard.writeText(key.publicKey!);
                    setCopied(true);
                    window.setTimeout(() => setCopied(false), 2000);
                  }}
                  className={`${BTN_PRIMARY} shrink-0`}
                >
                  {copied ? <Check className="size-3.5" /> : <Copy className="size-3.5" />}
                  {copied ? "Copiada" : "Copiar chave pública"}
                </button>
              </div>
            ) : (
              <p className="text-xs text-amber-300">A parte pública (.pub) desta chave não está na pasta ~/.ssh.</p>
            )}
            <ol className="space-y-1 text-[11px] leading-relaxed text-zinc-500">
              <li>
                <span className="text-zinc-300">1.</span> Copie a chave pública.
              </li>
              <li>
                <span className="text-zinc-300">2.</span> No GitHub, abra <b className="font-medium text-zinc-300">Chaves SSH → New SSH key</b>, cole e salve
                (basta fazer uma vez por computador).
              </li>
              <li>
                <span className="text-zinc-300">3.</span> Teste a conexão.
              </li>
            </ol>
            <div className="flex flex-wrap items-center gap-2">
              <button type="button" onClick={() => openUrl(GITHUB_KEYS_URL).catch(() => {})} className={BTN_GHOST}>
                <ExternalLink className="size-3.5" />
                Abrir GitHub → Chaves SSH
              </button>
              <button
                type="button"
                onClick={() =>
                  act("test", async () => {
                    setTest(await testSshGithub(distro));
                    return null;
                  })
                }
                disabled={busy !== null}
                className={BTN_GHOST}
              >
                {busy === "test" ? <Loader2 className="size-3.5 animate-spin" /> : <PlugZap className="size-3.5" />}
                Testar conexão com o GitHub
              </button>
              {test && (
                <span className={`flex min-w-0 items-center gap-1.5 text-xs ${test.ok ? "text-zinc-100" : "text-rose-300"}`} title={test.detail}>
                  {test.ok ? <CheckCircle2 className="size-3.5 shrink-0 text-emerald-400" /> : <XCircle className="size-3.5 shrink-0" />}
                  <span className="truncate">{test.message}</span>
                </span>
              )}
            </div>
          </div>
        ) : importable.length > 0 ? (
          <div className="mt-3 rounded-lg border border-zinc-700 bg-zinc-950 px-4 py-3">
            <p className="flex items-start gap-2 text-xs leading-relaxed text-zinc-300">
              <MonitorDown className="mt-px size-4 shrink-0 text-zinc-400" />
              <span>
                Encontramos {importable.length === 1 ? "uma chave SSH" : `${importable.length} chaves SSH`} no Windows (
                <span className="font-mono text-zinc-400">{importable.map((k) => k.name).join(", ")}</span>). Se você já usa essa chave no
                GitHub, é só trazer para o Linux: o app copia e ajusta as permissões que o SSH exige.
              </span>
            </p>
            <div className="mt-3 flex flex-wrap items-center gap-2">
              <button
                type="button"
                onClick={() =>
                  act("import", async () => {
                    const r = await importWindowsSshKeys(distro);
                    return `Importada: ${r.imported.join(", ")}${r.skipped.length ? ` · já existia no Linux: ${r.skipped.join(", ")}` : ""}`;
                  })
                }
                disabled={busy !== null}
                className={BTN_PRIMARY}
              >
                {busy === "import" ? <Loader2 className="size-3.5 animate-spin" /> : <MonitorDown className="size-3.5" />}
                Importar minha chave do Windows e ajustar permissões
              </button>
              {generateButton}
            </div>
          </div>
        ) : (
          <div className="mt-3 flex flex-wrap items-center gap-3">
            <p className="min-w-0 flex-1 text-xs text-zinc-400">
              Nenhuma chave SSH nesta distro ainda.
              {!generateEmail && " Salve o seu e-mail acima para gerar uma."}
            </p>
            {generateButton}
          </div>
        )}

        {key && generateButton && <div className="mt-3 border-t border-zinc-800 pt-3">{generateButton}</div>}
      </div>

      {!status.permissionsOk && (
        <div className="flex items-center gap-3 rounded-lg border border-amber-500/30 bg-amber-500/[0.05] px-4 py-3">
          <AlertTriangle className="size-4 shrink-0 text-amber-400" />
          <p className="min-w-0 flex-1 text-xs leading-relaxed text-amber-200">
            As permissões da chave estão abertas demais (comum quando a chave é copiada pelo Explorer do Windows). Assim o SSH se recusa a
            usá-la.
          </p>
          <button
            type="button"
            onClick={() =>
              act("fix", async () => {
                await fixSshPermissions(distro);
                return "Permissões corrigidas: a pasta ~/.ssh e a chave agora só são acessíveis por você.";
              })
            }
            disabled={busy !== null}
            className={`${BTN_PRIMARY} shrink-0`}
          >
            {busy === "fix" ? <Loader2 className="size-3.5 animate-spin" /> : <ShieldCheck className="size-3.5" />}
            Corrigir permissões da chave
          </button>
        </div>
      )}

      {(notice || actionError) && (
        <p className={`flex items-start gap-1.5 text-xs ${actionError ? "text-rose-300" : "text-zinc-300"}`}>
          {actionError ? <AlertTriangle className="mt-px size-3.5 shrink-0" /> : <CheckCircle2 className="mt-px size-3.5 shrink-0 text-emerald-400" />}
          <span className="break-words">{actionError ?? notice}</span>
        </p>
      )}
    </div>
  );
}
