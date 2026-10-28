import type { ReactNode } from "react";
import { Check, FolderSearch, Loader2, X } from "lucide-react";
import { IDE_IDS, type ToolId, type ToolInfo } from "../lib/explorer";
import type { PathCheck } from "../hooks/useToolPaths";
import { BTN_GHOST, FIELD, INPUT } from "../lib/ui";
import { ToolIcon, primaryIde } from "./explorer/OpenInButtons";

// IDE choice and custom tool paths, shared by Settings and the setup wizard.

/** "Automático" + one card per IDE. `tools` null = detection still running. */
export function IdeGrid({
  tools,
  allTools,
  value,
  onChange,
}: {
  tools: ToolInfo[] | null;
  allTools: ToolInfo[];
  value: ToolId | null;
  onChange: (id: ToolId | null) => void;
}) {
  return (
    <div className="grid grid-cols-4 gap-2" role="radiogroup">
      <IdeOption
        selected={value === null}
        onSelect={() => onChange(null)}
        label="Automático"
        detail={tools ? `hoje: ${allTools.find((t) => t.id === primaryIde(allTools, null))?.label ?? "—"}` : "…"}
      />
      {IDE_IDS.map((id) => {
        const tool = allTools.find((t) => t.id === id);
        return (
          <IdeOption
            key={id}
            selected={value === id}
            onSelect={() => onChange(id)}
            disabled={!!tools && !tool?.available}
            label={tool?.label ?? id}
            detail={!tools ? "…" : tool?.available ? "instalado" : "não encontrado"}
            icon={<ToolIcon tool={tool} id={id} className="size-4" />}
          />
        );
      })}
    </div>
  );
}

/** Auto-detected path + custom path field for every customizable tool. */
export function ToolPathList({
  tools,
  paths,
  checks,
  onChange,
  onBrowse,
}: {
  tools: ToolInfo[] | null;
  paths: Record<string, string>;
  checks: Record<string, PathCheck>;
  onChange: (id: string, value: string) => void;
  onBrowse: (tool: ToolInfo) => void;
}) {
  if (!tools) {
    return (
      <div className="flex h-24 items-center justify-center">
        <Loader2 className="size-4 animate-spin text-zinc-600" />
      </div>
    );
  }
  return (
    <div className="space-y-2.5">
      {tools
        .filter((t) => t.customizable)
        .map((t) => (
          <ToolRow
            key={t.id}
            tool={t}
            value={paths[t.id] ?? ""}
            check={checks[t.id] ?? { state: "idle" }}
            onChange={(v) => onChange(t.id, v)}
            onBrowse={() => onBrowse(t)}
          />
        ))}
    </div>
  );
}

function IdeOption({
  selected,
  onSelect,
  label,
  detail,
  icon,
  disabled,
}: {
  selected: boolean;
  onSelect: () => void;
  label: string;
  detail: string;
  icon?: ReactNode;
  disabled?: boolean;
}) {
  return (
    <button
      type="button"
      role="radio"
      aria-checked={selected}
      onClick={onSelect}
      disabled={disabled}
      className={`flex min-w-0 flex-col items-start gap-1.5 rounded-lg border px-3 py-2.5 text-left transition-colors disabled:cursor-not-allowed disabled:opacity-35 ${
        selected
          ? "border-zinc-100 bg-zinc-100 text-void"
          : "border-zinc-800 bg-zinc-900 text-zinc-200 hover:border-zinc-600"
      }`}
    >
      <span className="flex h-4 items-center gap-2">
        {icon ?? <span className={`size-1.5 rounded-full ${selected ? "bg-void" : "bg-zinc-500"}`} />}
        <span className="truncate text-sm font-semibold">{label}</span>
      </span>
      <span className={`truncate text-[11px] ${selected ? "text-zinc-600" : "text-zinc-500"}`}>{detail}</span>
    </button>
  );
}

function ToolRow({
  tool,
  value,
  check,
  onChange,
  onBrowse,
}: {
  tool: ToolInfo;
  value: string;
  check: PathCheck;
  onChange: (value: string) => void;
  onBrowse: () => void;
}) {
  const status =
    tool.source === "custom"
      ? { label: "Personalizado", dot: "bg-emerald-400" }
      : tool.source === "auto"
        ? { label: "Detectado", dot: "bg-emerald-400" }
        : tool.source === "custom_missing"
          ? { label: "Caminho salvo sumiu", dot: "bg-amber-400" }
          : { label: "Não encontrado", dot: "bg-zinc-600" };

  return (
    <div className="rounded-lg border border-zinc-800 bg-zinc-900 p-3.5">
      <div className="flex items-center gap-2.5">
        <ToolIcon tool={tool} id={tool.id} className="size-4 text-zinc-300" />
        <span className="text-sm font-semibold text-zinc-100">{tool.label}</span>
        <span className="ml-auto flex items-center gap-1.5 text-[11px] text-zinc-400">
          <span className={`size-1.5 rounded-full ${status.dot}`} />
          {status.label}
        </span>
      </div>

      <div className="mt-2 flex items-baseline gap-2 text-[11px]">
        <span className="w-24 shrink-0 tracking-wider text-zinc-600 uppercase">Auto</span>
        {tool.autoPath ? (
          <span className="truncate font-mono text-zinc-400" title={tool.autoPath}>
            {tool.autoPath}
          </span>
        ) : (
          <span className="text-zinc-500">Não encontrado no caminho padrão</span>
        )}
      </div>

      <div className="mt-2 flex items-center gap-2">
        <span className="w-24 shrink-0 text-[11px] tracking-wider text-zinc-600 uppercase">Personalizado</span>
        <label className={`${FIELD} min-w-0 flex-1 ${check.state === "error" ? "border-rose-500/50" : ""}`}>
          <input
            value={value}
            onChange={(e) => onChange(e.target.value)}
            spellCheck={false}
            placeholder={tool.id === "windows_terminal" ? "ex.: D:\\Apps\\Terminal\\wt.exe" : "ex.: D:\\Apps\\IDE\\bin\\ide.cmd"}
            className={`${INPUT} font-mono`}
          />
          {check.state === "checking" && <Loader2 className="size-3.5 shrink-0 animate-spin text-zinc-500" />}
          {check.state === "ok" && <Check className="size-3.5 shrink-0 text-emerald-400" />}
          {value && (
            <button type="button" onClick={() => onChange("")} aria-label="Limpar" className="shrink-0 text-zinc-600 hover:text-zinc-200">
              <X className="size-3.5" />
            </button>
          )}
        </label>
        <button type="button" onClick={onBrowse} className={`${BTN_GHOST} shrink-0`}>
          <FolderSearch className="size-3.5" />
          Procurar…
        </button>
      </div>
      {check.state === "error" && <p className="mt-1.5 pl-26 text-[11px] text-rose-300">{check.message}</p>}
    </div>
  );
}
