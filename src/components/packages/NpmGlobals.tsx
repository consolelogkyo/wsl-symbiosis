import { useMemo, useState } from "react";
import { CheckCircle2, Download, Loader2, PackageX, Search, Trash2, X } from "lucide-react";
import {
  GlobalPackage,
  NodeGlobalPackage,
  installNpmGlobal,
  packageSpecError,
  shortenHome,
  uninstallNpmGlobal,
} from "../../lib/packages";
import { errorMessage } from "../../lib/wsl";
import { TOOL_BRANDS } from "../../lib/brands";
import { BrandIcon } from "../BrandIcon";
import { BTN_DANGER_SOLID, BTN_PRIMARY, FIELD, INPUT, LABEL } from "../../lib/ui";

interface Props {
  distro: string;
  prefix: string | null;
  /** Globals of the active Node (npm list -g). */
  packages: GlobalPackage[];
  /** Globals of every nvm Node version (read from disk). */
  nvmGlobals: NodeGlobalPackage[];
  /** Active Node version, e.g. `v22.13.0` (null without nvm). */
  activeNode: string | null;
  onChanged: () => Promise<void>;
}

type Log = { ok: boolean; title: string; output: string };

interface Row {
  name: string;
  /** Version in the active Node, when installed there. */
  version: string | null;
  bundled: boolean;
  /** Every nvm Node version that has it (newest first). */
  nodes: NodeGlobalPackage[];
}

const BUNDLED = ["npm", "corepack"];

export function NpmGlobals({ distro, prefix, packages, nvmGlobals, activeNode, onChanged }: Props) {
  const [spec, setSpec] = useState("");
  const [installing, setInstalling] = useState(false);
  const [query, setQuery] = useState("");
  const [confirming, setConfirming] = useState<string | null>(null);
  const [removing, setRemoving] = useState<string | null>(null);
  const [log, setLog] = useState<Log | null>(null);

  const specError = packageSpecError(spec.trim());
  // Active Node's globals + those that only exist in other nvm versions (e.g. eas-cli in v20).
  const all = useMemo<Row[]>(() => {
    const byName = new Map<string, Row>();
    for (const p of packages) byName.set(p.name, { name: p.name, version: p.version, bundled: p.bundled, nodes: [] });
    for (const g of nvmGlobals) {
      const row = byName.get(g.name) ?? { name: g.name, version: null, bundled: BUNDLED.includes(g.name), nodes: [] };
      row.nodes.push(g);
      byName.set(g.name, row);
    }
    return [...byName.values()].sort(
      (a, b) => Number(a.bundled) - Number(b.bundled) || Number(a.version === null) - Number(b.version === null) || a.name.localeCompare(b.name),
    );
  }, [packages, nvmGlobals]);
  const rows = useMemo(() => {
    const q = query.trim().toLowerCase();
    return q ? all.filter((p) => p.name.toLowerCase().includes(q)) : all;
  }, [all, query]);

  if (prefix === null) {
    return (
      <div className="rounded-lg border border-dashed border-zinc-800 p-10 text-center">
        <PackageX className="mx-auto mb-2 size-5 text-zinc-600" />
        <p className="text-sm text-zinc-300">npm não encontrado nesta distro</p>
        <p className="mt-1 text-xs text-zinc-500">
          O npm do Windows (via <span className="font-mono">/mnt/c</span>) é ignorado de propósito: ele instalaria pacotes
          no Windows.
        </p>
      </div>
    );
  }

  const install = async () => {
    const value = spec.trim();
    if (!value || specError) return;
    setInstalling(true);
    setLog(null);
    try {
      const out = await installNpmGlobal(distro, value);
      setLog({ ok: true, title: `${value} instalado`, output: out });
      setSpec("");
      await onChanged();
    } catch (err) {
      setLog({ ok: false, title: `Falha ao instalar ${value}`, output: errorMessage(err) });
    } finally {
      setInstalling(false);
    }
  };

  const uninstall = async (name: string) => {
    setRemoving(name);
    setLog(null);
    try {
      const out = await uninstallNpmGlobal(distro, name);
      setLog({ ok: true, title: `${name} removido`, output: out });
      await onChanged();
    } catch (err) {
      setLog({ ok: false, title: `Falha ao remover ${name}`, output: errorMessage(err) });
    } finally {
      setRemoving(null);
      setConfirming(null);
    }
  };

  return (
    <div className="space-y-4">
      <form
        onSubmit={(e) => {
          e.preventDefault();
          install();
        }}
        className="flex items-start gap-2"
      >
        <div className="flex-1">
          <label className="flex items-center gap-2 rounded-lg border border-zinc-800 bg-zinc-900 px-3 py-2 transition-colors focus-within:border-zinc-500">
            <Download className="size-4 text-zinc-500" />
            <input
              value={spec}
              onChange={(e) => setSpec(e.target.value)}
              disabled={installing}
              placeholder="+ Instalar pacote global: typescript, vercel, @nestjs/cli, tsx@4"
              className="w-full bg-transparent font-mono text-sm text-zinc-100 placeholder:font-sans placeholder:text-zinc-600 focus:outline-none"
            />
          </label>
          <p className="mt-1.5 text-[11px] text-zinc-500">
            {specError ? (
              <span className="text-rose-300">{specError}</span>
            ) : (
              <>
                npm install -g · prefixo <span className="font-mono">{shortenHome(prefix)}</span>
              </>
            )}
          </p>
        </div>
        <button
          type="submit"
          disabled={!spec.trim() || !!specError || installing}
          className={`${BTN_PRIMARY} h-[38px] px-4 text-sm`}
        >
          {installing ? <Loader2 className="size-4 animate-spin" /> : <Download className="size-4" />}
          {installing ? "Instalando…" : "Instalar"}
        </button>
      </form>

      {log && (
        <div
          className={`rounded-lg border px-3 py-2 ${
            log.ok ? "border-zinc-800 bg-zinc-900" : "border-rose-500/25 bg-zinc-900"
          }`}
        >
          <div className={`flex items-center gap-1.5 text-xs font-medium ${log.ok ? "text-zinc-100" : "text-rose-300"}`}>
            {log.ok && <CheckCircle2 className="size-3.5 text-emerald-400" />}
            {log.title}
            <button type="button" onClick={() => setLog(null)} className="ml-auto text-zinc-500 hover:text-zinc-200">
              <X className="size-3.5" />
            </button>
          </div>
          {log.output && (
            <pre className="mt-1.5 max-h-40 overflow-auto font-mono text-[11px] whitespace-pre-wrap text-zinc-400">
              {log.output}
            </pre>
          )}
        </div>
      )}

      <section className="overflow-hidden rounded-lg border border-zinc-800 bg-zinc-900">
        <div className="flex items-center gap-3 border-b border-zinc-800 px-4 py-2">
          <span className={LABEL}>Pacotes globais</span>
          <span className="text-[11px] text-zinc-600">{all.length}</span>
          {activeNode && (
            <span className="flex items-center gap-1.5 text-[11px] text-zinc-500">
              · Node ativo
              <span className="rounded-full bg-zinc-100 px-1.5 font-mono text-[10px] font-semibold text-void">{activeNode}</span>
            </span>
          )}
          <label className={`${FIELD} ml-auto w-56`}>
            <Search className="size-3.5 text-zinc-500" />
            <input
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              placeholder="Buscar pacote"
              className={INPUT}
            />
          </label>
        </div>
        {rows.map((p) => (
          <div key={p.name} className="group flex items-center gap-3 border-b border-zinc-800/50 px-4 py-2 last:border-0 hover:bg-zinc-800/30">
            <span className={`font-mono text-sm ${p.version !== null ? "text-zinc-100" : "text-zinc-400"}`}>{p.name}</span>
            {p.version !== null && <span className="font-mono text-xs text-zinc-500">{p.version}</span>}
            {p.bundled && (
              <span className="rounded border border-zinc-700 px-1.5 text-[10px] text-zinc-400" title="Vem junto com o Node">
                embutido
              </span>
            )}
            <NodeChips row={p} activeNode={activeNode} />
            <div className="ml-auto flex items-center gap-1.5">
              {removing === p.name ? (
                <Loader2 className="size-3.5 animate-spin text-zinc-500" />
              ) : confirming === p.name ? (
                <>
                  <span className="text-xs text-rose-300">Desinstalar {p.name}?</span>
                  <button
                    type="button"
                    onClick={() => uninstall(p.name)}
                    className={BTN_DANGER_SOLID}
                  >
                    Desinstalar
                  </button>
                  <button type="button" onClick={() => setConfirming(null)} aria-label="Cancelar" className="p-0.5 text-zinc-500 hover:text-zinc-200">
                    <X className="size-3.5" />
                  </button>
                </>
              ) : p.version === null ? (
                <span
                  className="text-[11px] text-zinc-600"
                  title={`Instalado só em outra versão do Node. Para remover: nvm use ${p.nodes[0]?.nodeVersion ?? "<versão>"} && npm uninstall -g ${p.name}`}
                >
                  fora do Node ativo
                </span>
              ) : (
                !p.bundled && (
                  <button
                    type="button"
                    onClick={() => setConfirming(p.name)}
                    disabled={removing !== null}
                    className="flex items-center gap-1 rounded-full border border-transparent px-2 py-0.5 text-[11px] text-zinc-500 opacity-0 transition-colors group-hover:opacity-100 hover:border-rose-500/40 hover:text-rose-300 focus:opacity-100"
                  >
                    <Trash2 className="size-3" />
                    Desinstalar
                  </button>
                )
              )}
            </div>
          </div>
        ))}
        {rows.length === 0 && (
          <div className="px-4 py-6 text-center text-xs text-zinc-500">
            {query ? "Nenhum pacote encontrado." : "Nenhum pacote global instalado."}
          </div>
        )}
      </section>
    </div>
  );
}

/** Which nvm Node versions have this package (active one inverted in white). */
function NodeChips({ row, activeNode }: { row: Row; activeNode: string | null }) {
  if (row.nodes.length === 0) return null;
  // npm/corepack ship with every Node: a count says enough.
  if (row.bundled) {
    return <span className="text-[11px] text-zinc-600">em {row.nodes.length} versões do Node</span>;
  }
  return (
    <span className="flex flex-wrap items-center gap-1">
      {row.nodes.map((n) => {
        const active = n.nodeVersion === activeNode;
        return (
          <span
            key={n.nodeVersion}
            title={`${row.name} ${n.version} no Node ${n.nodeVersion}${active ? " (ativo)" : ""}`}
            className={`flex items-center gap-1 rounded-full border px-1.5 py-px font-mono text-[10px] ${
              active ? "border-zinc-100 bg-zinc-100 font-semibold text-void" : "border-zinc-800 bg-zinc-950 text-zinc-500"
            }`}
          >
            <BrandIcon icon={TOOL_BRANDS.node} className="size-2.5" color="currentColor" />
            {n.nodeVersion}
            {n.version && <span className="text-zinc-600">· {n.version}</span>}
          </span>
        );
      })}
    </span>
  );
}