import { useEffect, useState } from "react";
import { CheckCircle2, Loader2 } from "lucide-react";
import { TOOL_BRANDS } from "../../lib/brands";
import { BrandIcon } from "../BrandIcon";
import { NvmInfo, shortenHome, switchNvmDefault } from "../../lib/packages";
import { errorMessage } from "../../lib/wsl";
import { CARD } from "../../lib/ui";

interface Props {
  distro: string;
  nvm: NvmInfo;
  onChanged: () => void;
}

export function NvmCard({ distro, nvm, onChanged }: Props) {
  const [busy, setBusy] = useState<string | null>(null);
  // The new default as soon as nvm confirms it: the full re-scan that follows (onChanged)
  // takes a few seconds in the login shell, and waiting for it made the switch feel laggy.
  const [confirmed, setConfirmed] = useState<string | null>(null);
  const [message, setMessage] = useState<{ ok: boolean; text: string } | null>(null);

  // A fresh scan arrived: it is the source of truth again.
  useEffect(() => setConfirmed(null), [nvm]);

  const makeDefault = async (version: string) => {
    if (busy) return;
    setBusy(version);
    setMessage(null);
    try {
      const out = await switchNvmDefault(distro, version);
      setConfirmed(version);
      setMessage({ ok: true, text: out || `default → ${version}` });
      onChanged();
    } catch (err) {
      setMessage({ ok: false, text: errorMessage(err) });
    } finally {
      setBusy(null);
    }
  };

  return (
    <section className={`${CARD} p-4`}>
      <div className="flex items-center gap-2.5">
        <div className="flex size-8 items-center justify-center rounded-md border border-zinc-700 bg-zinc-950">
          <BrandIcon icon={TOOL_BRANDS.node} className="size-4" />
        </div>
        <div>
          <h3 className="text-sm font-semibold text-zinc-100">Gerenciador Node (NVM)</h3>
          <p className="font-mono text-[11px] text-zinc-500">
            {shortenHome(nvm.dir)} · alias default: {confirmed ?? nvm.defaultAlias ?? "—"}
          </p>
        </div>
        <span className="ml-auto text-[11px] text-zinc-500">A troca vale para novos terminais</span>
      </div>

      <div className="mt-3 flex flex-wrap gap-2">
        {nvm.versions.map((v) => {
          // New shells start on the default, so the confirmed default is also the active one.
          const isDefault = confirmed ? v.version === confirmed : v.isDefault;
          const active = confirmed ? v.version === confirmed : v.active;
          const switching = busy === v.version;

          if (isDefault) {
            return (
              <div
                key={v.version}
                className="flex items-center gap-2 rounded-full border border-zinc-100 bg-zinc-100 py-1 pr-1.5 pl-3 text-void"
              >
                <span className="font-mono text-sm font-medium">{v.version}</span>
                {active && <ActiveTag />}
                <span className="rounded-full bg-void px-2 py-px text-[10px] font-semibold tracking-wider text-zinc-100 uppercase">
                  default
                </span>
              </div>
            );
          }

          return (
            <button
              key={v.version}
              type="button"
              onClick={() => makeDefault(v.version)}
              disabled={busy !== null}
              title={`Tornar ${v.version} a versão padrão do Node (nvm alias default)`}
              className={`group flex items-center gap-2 rounded-full border py-1 pr-1.5 pl-3 text-zinc-100 transition-colors disabled:cursor-default ${
                switching
                  ? "border-zinc-100 bg-zinc-900"
                  : "border-zinc-800 bg-zinc-950 hover:border-zinc-500 hover:bg-zinc-900 disabled:opacity-40 disabled:hover:border-zinc-800 disabled:hover:bg-zinc-950"
              }`}
            >
              <span className="font-mono text-sm font-medium">{v.version}</span>
              {active && <ActiveTag />}
              <span
                className={`flex items-center gap-1 rounded-full border px-2 py-px text-[10px] transition-colors ${
                  switching
                    ? "border-zinc-100 text-zinc-100"
                    : "border-zinc-700 text-zinc-400 group-hover:border-zinc-300 group-hover:text-zinc-100"
                }`}
              >
                {switching && <Loader2 className="size-3 animate-spin" />}
                {switching ? "Trocando…" : "Tornar padrão"}
              </span>
            </button>
          );
        })}
      </div>

      {message && (
        <div className={`mt-3 flex items-center gap-1.5 font-mono text-xs ${message.ok ? "text-zinc-200" : "text-rose-300"}`}>
          {message.ok && <CheckCircle2 className="size-3.5 text-emerald-400" />}
          {message.text}
        </div>
      )}
    </section>
  );
}

function ActiveTag() {
  return (
    <span className="flex items-center gap-1 text-[10px] tracking-wider uppercase opacity-70">
      <span className="size-1.5 rounded-full bg-emerald-400" />
      ativa
    </span>
  );
}
