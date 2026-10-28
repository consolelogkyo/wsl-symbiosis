import { useState } from "react";
import { Boxes, Cpu, Layers, Package, Wrench, type LucideIcon } from "lucide-react";
import type { DetectedTool, ToolCategory } from "../../lib/packages";
import { shortenHome } from "../../lib/packages";
import { TOOL_BRANDS } from "../../lib/brands";
import { BrandIcon } from "../BrandIcon";

const CATEGORIES: { id: ToolCategory; label: string; icon: LucideIcon }[] = [
  { id: "runtime", label: "Runtimes", icon: Cpu },
  { id: "package_manager", label: "Gerenciadores de pacotes", icon: Package },
  { id: "version_manager", label: "Gerenciadores de versão", icon: Layers },
  { id: "tool", label: "Ferramentas", icon: Wrench },
];

export function ToolGrid({ tools }: { tools: DetectedTool[] }) {
  const [showMissing, setShowMissing] = useState(false);
  const installed = tools.filter((t) => t.installed).length;

  return (
    <div className="space-y-5">
      <div className="flex items-center gap-3">
        <span className="text-xs text-zinc-500">
          <span className="font-mono text-zinc-300">{installed}</span> de {tools.length} detectadas · ferramentas do
          Windows (<span className="font-mono">/mnt/c</span>) são ignoradas
        </span>
        <label className="ml-auto flex cursor-pointer items-center gap-2 text-xs text-zinc-400">
          <input
            type="checkbox"
            checked={showMissing}
            onChange={(e) => setShowMissing(e.target.checked)}
            className="size-3.5 accent-zinc-100"
          />
          Mostrar não instaladas
        </label>
      </div>

      {CATEGORIES.map(({ id, label, icon: Icon }) => {
        const items = tools.filter((t) => t.category === id && (showMissing || t.installed));
        if (items.length === 0) return null;
        return (
          <section key={id}>
            <h3 className="mb-2 flex items-center gap-2 text-[11px] font-medium tracking-wider text-zinc-500 uppercase">
              <Icon className="size-3.5" />
              {label}
            </h3>
            <div className="grid grid-cols-1 gap-2.5 md:grid-cols-2 xl:grid-cols-3 2xl:grid-cols-4">
              {items.map((t) => (
                <ToolCard key={t.id} tool={t} />
              ))}
            </div>
          </section>
        );
      })}

      {installed === 0 && !showMissing && (
        <div className="rounded-lg border border-dashed border-zinc-800 p-8 text-center text-xs text-zinc-500">
          <Boxes className="mx-auto mb-2 size-5 text-zinc-600" />
          Nenhuma ferramenta de desenvolvimento encontrada nesta distro.
        </div>
      )}
    </div>
  );
}

function ToolCard({ tool }: { tool: DetectedTool }) {
  const isFunction = tool.path !== null && !tool.path.startsWith("/");
  const brand = TOOL_BRANDS[tool.id];
  const FallbackIcon = CATEGORIES.find((c) => c.id === tool.category)?.icon ?? Wrench;
  return (
    <div
      className={`rounded-lg border p-3.5 ${
        tool.installed ? "border-zinc-800 bg-zinc-900" : "border-dashed border-zinc-800 bg-transparent opacity-50"
      }`}
    >
      <div className="flex items-center gap-2.5">
        <span className="flex size-7 shrink-0 items-center justify-center rounded-md border border-zinc-800 bg-zinc-950">
          {brand ? (
            <BrandIcon icon={brand} className={`size-4 ${tool.installed ? "" : "grayscale"}`} />
          ) : (
            <FallbackIcon className="size-4 text-zinc-400" />
          )}
        </span>
        <span className="text-sm font-medium text-zinc-100">{tool.label}</span>
        {tool.installed ? (
          <span className="ml-auto flex items-center gap-1.5 text-[10px] tracking-wider text-zinc-500 uppercase">
            <span className="size-1.5 rounded-full bg-emerald-400" />
            Instalado
          </span>
        ) : (
          <span className="ml-auto text-[10px] tracking-wider text-zinc-600 uppercase">
            Ausente
          </span>
        )}
      </div>
      {tool.version ? (
        <div className="mt-2 font-mono text-xl font-medium tracking-tight text-zinc-100 tabular-nums" title={tool.rawVersion ?? undefined}>
          {tool.version}
        </div>
      ) : (
        tool.installed && (
          <div className="mt-2.5 text-xs text-zinc-400" title={tool.rawVersion ?? undefined}>
            {isFunction ? "Função carregada pelo shell" : "Versão não informada"}
          </div>
        )
      )}
      {tool.path && !isFunction && (
        <div className="mt-1 truncate font-mono text-[11px] text-zinc-500" title={tool.path}>
          {shortenHome(tool.path)}
        </div>
      )}
    </div>
  );
}
