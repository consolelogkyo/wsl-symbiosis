import type { GitAuth } from "../lib/settings";

const OPTIONS: { id: GitAuth; title: string; detail: string; tag?: string }[] = [
  {
    id: "auto",
    title: "Automático",
    tag: "recomendado",
    detail:
      "Usa o remoto do repositório. Se um remoto HTTPS pedir usuário e senha, tenta de novo pelo SSH com a sua chave (~/.ssh).",
  },
  {
    id: "ssh",
    title: "Sempre via SSH",
    detail: "Remotos HTTPS são acessados direto pelo SSH (https://github.com/org/repo → git@github.com:org/repo).",
  },
  {
    id: "remote",
    title: "Como está no repositório",
    detail: "Nunca troca o protocolo. Remotos HTTPS precisam de um credential helper com a senha/token salvos.",
  },
];

interface Props {
  gitAuth: GitAuth;
  onChange: (value: GitAuth) => void;
}

/** Settings → Git: how pull/fetch reach HTTPS remotes (the app has no terminal to type a password). */
export function GitSettings({ gitAuth, onChange }: Props) {
  return (
    <section>
      <div className="mb-2.5 flex items-baseline gap-2.5">
        <h3 className="text-[11px] font-semibold tracking-wider text-zinc-300 uppercase">git pull e Buscar (fetch)</h3>
        <span className="ml-auto text-[11px] text-zinc-600">Remotos HTTPS sem senha salva</span>
      </div>
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
        A troca para SSH vale só para o comando do app: o remoto salvo no repositório nunca é alterado. O pull é sempre
        "só avança" (--ff-only): nunca cria merge nem conflito.
      </p>
    </section>
  );
}
