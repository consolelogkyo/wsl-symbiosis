import { useState } from "react";
import { AppWindow, Code2, FolderOpen, Loader2, MousePointer2, Rocket, SquareTerminal, type LucideIcon } from "lucide-react";
import { IDE_IDS, type ToolId, type ToolInfo } from "../../lib/explorer";

/** Generic icons, used until (or when) the official icon can't be read from the install. */
export const TOOL_ICONS: Record<ToolId, LucideIcon> = {
  antigravity: Rocket,
  vscode: Code2,
  cursor: MousePointer2,
  explorer: FolderOpen,
  windows_terminal: AppWindow,
};

const LABELS: Record<ToolId, string> = {
  antigravity: "Antigravity",
  vscode: "VS Code",
  cursor: "Cursor",
  explorer: "Explorer",
  windows_terminal: "Windows Terminal",
};

/** Official icon from the install when available, else the generic one. */
export function ToolIcon({ tool, id, className = "size-3.5" }: { tool?: ToolInfo; id: ToolId; className?: string }) {
  if (tool?.icon) return <img src={tool.icon} alt="" draggable={false} className={`shrink-0 ${className}`} />;
  const Icon = TOOL_ICONS[id];
  return <Icon className={`shrink-0 ${className}`} />;
}

/** The IDE shown as the labeled primary button: the configured one if installed, else the first installed. */
export function primaryIde(tools: ToolInfo[], defaultIde: ToolId | null): ToolId {
  const available = (id: ToolId) => tools.find((t) => t.id === id)?.available ?? false;
  if (defaultIde && available(defaultIde)) return defaultIde;
  return IDE_IDS.find(available) ?? defaultIde ?? "antigravity";
}

// Labels collapse to icon-only circles by the width of the nearest @container: the "Abrir
// pasta em" bar (variant full) or the Explorer row (variant compact, primary IDE only).
const ICON_ONLY = "p-1.5";
const COLLAPSIBLE = {
  full: { label: "@max-[60rem]:hidden", pad: "px-2.5 py-1 @max-[60rem]:p-1.5" },
  compact: { label: "@max-5xl:hidden", pad: "p-1.5 @min-5xl:px-2.5 @min-5xl:py-1" },
} as const;

interface Props {
  tools: ToolInfo[];
  /** From the settings; null = first installed IDE. */
  defaultIde: ToolId | null;
  /** "full" shows labels (toolbar); "compact" shows icons only (rows), the primary IDE keeps its label. */
  variant: "full" | "compact";
  onOpen: (tool: ToolId) => Promise<void>;
  onTerminalHere: () => void;
  /** Hide the Windows Terminal button (rows keep it lean). */
  withWindowsTerminal?: boolean;
}

export function OpenInButtons({ tools, defaultIde, variant, onOpen, onTerminalHere, withWindowsTerminal = true }: Props) {
  const [busy, setBusy] = useState<string | null>(null);
  const compact = variant === "compact";
  const primary = primaryIde(tools, defaultIde);
  // Primary first, then the other IDEs, Explorer, Windows Terminal.
  const order: ToolId[] = [primary, ...IDE_IDS.filter((id) => id !== primary), "explorer", "windows_terminal"];

  const click = async (id: ToolId) => {
    setBusy(id);
    try {
      await onOpen(id);
    } finally {
      setBusy(null);
    }
  };

  return (
    <div className="flex items-center gap-1">
      {order
        .filter((id) => withWindowsTerminal || id !== "windows_terminal")
        // Rows only show what is installed; the toolbar keeps every tool (with a hint).
        .filter((id) => !compact || id === primary || tools.find((t) => t.id === id)?.available)
        .map((id) => {
          const info = tools.find((t) => t.id === id);
          const available = info?.available ?? false;
          const isPrimary = id === primary;
          const label = LABELS[id];
          // Full: the primary always keeps its label. Compact: only the primary has one.
          const sizing = compact
            ? isPrimary
              ? COLLAPSIBLE.compact
              : null
            : isPrimary
              ? { label: "", pad: "px-2.5 py-1" }
              : COLLAPSIBLE.full;
          return (
            <button
              key={id}
              type="button"
              onClick={(e) => {
                e.stopPropagation();
                click(id);
              }}
              disabled={!available || busy !== null}
              title={available ? `Abrir em ${label}` : `${label} não encontrado (configure o caminho no Diagnóstico)`}
              className={`flex items-center gap-1.5 rounded-full border text-xs whitespace-nowrap transition-colors disabled:cursor-not-allowed disabled:opacity-35 ${
                sizing?.pad ?? ICON_ONLY
              } ${
                isPrimary
                  ? "border-zinc-100 bg-zinc-100 font-semibold text-void hover:bg-white"
                  : "border-zinc-800 font-medium text-zinc-300 hover:border-zinc-600 hover:bg-zinc-900 hover:text-zinc-100"
              }`}
            >
              {busy === id ? <Loader2 className="size-3.5 shrink-0 animate-spin" /> : <ToolIcon tool={info} id={id} />}
              {sizing && <span className={sizing.label}>{label}</span>}
            </button>
          );
        })}
      <button
        type="button"
        onClick={(e) => {
          e.stopPropagation();
          onTerminalHere();
        }}
        title="Abrir no terminal do app"
        className={`flex items-center gap-1.5 rounded-full border border-zinc-800 text-xs font-medium whitespace-nowrap text-zinc-300 transition-colors hover:border-zinc-600 hover:bg-zinc-900 hover:text-zinc-100 ${
          compact ? ICON_ONLY : COLLAPSIBLE.full.pad
        }`}
      >
        <SquareTerminal className="size-3.5 shrink-0" />
        {!compact && <span className={COLLAPSIBLE.full.label}>Terminal aqui</span>}
      </button>
    </div>
  );
}
