import { useState, type ReactNode } from "react";
import type { GitAuth } from "../lib/settings";
import { type Distro, isSystemDistro, preferredDistro } from "../lib/wsl";
import { GitIdentityCard, SshKeyCard, useGitIdentity } from "./GitIdentityPanel";
import { StatusDot } from "./StatusDot";

const OPTIONS: { id: GitAuth; title: string; detail: string; tag?: string }[] = [
  {
    id: "auto",
    title: "Automático",
    tag: "recomendado",
    detail:
      "Usa o endereço do repositório. Se um endereço HTTPS pedir usuário e senha, tenta de novo pela sua chave SSH.",
  },
  {
    id: "ssh",
    title: "Sempre pela chave SSH",
    detail: "Repositórios com endereço HTTPS são acessados direto pela chave (https://github.com/org/repo → git@github.com:org/repo).",
  },
  {
    id: "remote",
    title: "Como está no repositório",
    detail: "Nunca troca o protocolo. Endereços HTTPS precisam de uma senha ou token salvo no Git (credential helper).",
  },
];

interface Props {
  gitAuth: GitAuth;
  onChange: (value: GitAuth) => void;
  distros: Distro[];
  /** Settings' default distro: the one configured here first. */
  defaultDistro: string | null;
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

/** Settings → Git: identity for commits, the SSH key, and how pull/fetch reach HTTPS remotes. */
export function GitSettings({ gitAuth, onChange, distros, defaultDistro }: Props) {
  const real = distros.filter((d) => !isSystemDistro(d.name));
  const initial = real.find((d) => d.name === defaultDistro)?.name ?? preferredDistro(real)?.name ?? null;
  const [distro, setDistro] = useState<string | null>(initial);
  const current = distro && real.some((d) => d.name === distro) ? distro : initial;
  const identity = useGitIdentity(current);

  return (
    <div className="space-y-6">
      {real.length > 1 && (
        <div className="flex flex-wrap items-center gap-2">
          <span className="text-[11px] text-zinc-500">Configurar o Git da distro:</span>
          {real.map((d) => (
            <button
              key={d.name}
              type="button"
              onClick={() => setDistro(d.name)}
              className={`flex items-center gap-1.5 rounded-full border px-2.5 py-0.5 text-xs transition-colors ${
                d.name === current ? "border-zinc-100 bg-zinc-100 font-semibold text-void" : "border-zinc-800 text-zinc-400 hover:text-zinc-100"
              }`}
            >
              <StatusDot state={d.state} />
              {d.name}
            </button>
          ))}
        </div>
      )}

      {current ? (
        <>
          <Section index="01" title="Quem é você nos commits" hint="Identidade do Git">
            <GitIdentityCard distro={current} identity={identity} />
          </Section>
          <Section index="02" title="Chave de acesso SSH" hint="GitHub / GitLab sem pedir senha">
            <SshKeyCard distro={current} identity={identity} />
          </Section>
        </>
      ) : (
        <p className="rounded-lg border border-dashed border-zinc-800 px-4 py-3 text-xs text-zinc-500">
          Nenhuma distro Linux instalada ainda: o Setup (aba Sistema & IDEs → 07) instala o Ubuntu.
        </p>
      )}

      <Section index="03" title="Atualizar repositórios (git pull e fetch)" hint="Endereços HTTPS sem senha salva">
        <div className="overflow-hidden rounded-lg border border-zinc-800" role="radiogroup">
          {OPTIONS.map((o) => {
            const selected = gitAuth === o.id;
            return (
              <button
                key={o.id}
                type="button"
                role="radio"
                aria-checked={selected}
                onClick={() => onChange(o.id)}
                className={`flex w-full items-start gap-3 border-b border-zinc-800 px-4 py-3 text-left transition-colors last:border-0 ${
                  selected ? "bg-zinc-900" : "hover:bg-zinc-900/60"
                }`}
              >
                <span
                  className={`mt-0.5 flex size-3.5 shrink-0 items-center justify-center rounded-full border ${
                    selected ? "border-zinc-100" : "border-zinc-600"
                  }`}
                >
                  {selected && <span className="size-1.5 rounded-full bg-zinc-100" />}
                </span>
                <span className="min-w-0 flex-1">
                  <span className={`flex items-center gap-2 text-sm font-medium ${selected ? "text-zinc-100" : "text-zinc-300"}`}>
                    {o.title}
                    {o.tag && (
                      <span className="rounded-full border border-zinc-700 px-1.5 text-[9px] font-semibold tracking-wider text-zinc-400 uppercase">
                        {o.tag}
                      </span>
                    )}
                  </span>
                  <span className="mt-0.5 block text-xs text-zinc-500">{o.detail}</span>
                </span>
              </button>
            );
          })}
        </div>
        <p className="mt-3 text-[11px] text-zinc-600">
          A troca para a chave SSH vale só para o comando do app: o endereço salvo no repositório nunca é alterado. A atualização só
          avança o código (--ff-only): nunca cria merge nem conflito. Esta opção é salva com o botão Salvar abaixo.
        </p>
      </Section>
    </div>
  );
}
