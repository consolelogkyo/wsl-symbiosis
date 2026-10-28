import { useState } from "react";
import {
  ArrowLeft,
  ArrowUp,
  Check,
  ChevronRight,
  Copy,
  Eye,
  EyeOff,
  FilePlus,
  FolderPlus,
  Home,
  Loader2,
  RefreshCw,
  Search,
  Slash,
} from "lucide-react";

interface Props {
  distro: string;
  path: string;
  windowsPath: string;
  home: string | null;
  loading: boolean;
  canGoBack: boolean;
  showHidden: boolean;
  filter: string;
  onFilter: (value: string) => void;
  onNavigate: (path: string) => void;
  onBack: () => void;
  onUp: () => void;
  onRefresh: () => void;
  onToggleHidden: () => void;
  onNew: (kind: "dir" | "file") => void;
}

const iconButton =
  "rounded-md p-1.5 text-zinc-400 transition-colors hover:bg-zinc-800 hover:text-zinc-100 disabled:cursor-not-allowed disabled:opacity-30 disabled:hover:bg-transparent";

function CopyButton({ label, value }: { label: string; value: string }) {
  const [copied, setCopied] = useState(false);
  return (
    <button
      type="button"
      onClick={async () => {
        try {
          await navigator.clipboard.writeText(value);
          setCopied(true);
          window.setTimeout(() => setCopied(false), 1200);
        } catch {
          /* clipboard unavailable: nothing useful to show */
        }
      }}
      title={value}
      className="flex shrink-0 items-center gap-1 rounded-full border border-zinc-800 px-2 py-0.5 text-[11px] text-zinc-400 transition-colors hover:border-zinc-600 hover:text-zinc-100"
    >
      {copied ? <Check className="size-3 text-zinc-100" /> : <Copy className="size-3" />}
      {label}
    </button>
  );
}

export function ExplorerToolbar(p: Props) {
  const segments = p.path.split("/").filter(Boolean);

  return (
    <div className="space-y-2 border-b border-zinc-800 bg-zinc-950 px-4 py-3">
      <div className="flex items-center gap-1">
        <button type="button" onClick={p.onBack} disabled={!p.canGoBack} title="Voltar" className={iconButton}>
          <ArrowLeft className="size-4" />
        </button>
        <button type="button" onClick={p.onUp} disabled={p.path === "/"} title="Subir nível (..)" className={iconButton}>
          <ArrowUp className="size-4" />
        </button>
        <button
          type="button"
          onClick={() => p.home && p.onNavigate(p.home)}
          disabled={!p.home}
          title={`Home (${p.home ?? "~"})`}
          className={iconButton}
        >
          <Home className="size-4" />
        </button>
        <button type="button" onClick={() => p.onNavigate("/")} title="Raiz (/)" className={iconButton}>
          <Slash className="size-4" />
        </button>
        <button type="button" onClick={p.onRefresh} title="Atualizar" className={iconButton}>
          {p.loading ? <Loader2 className="size-4 animate-spin" /> : <RefreshCw className="size-4" />}
        </button>

        <div className="mx-2 h-5 w-px bg-zinc-800" />

        <label className="flex w-64 items-center gap-2 rounded-md border border-zinc-800 bg-zinc-900 px-2 py-1 transition-colors focus-within:border-zinc-500">
          <Search className="size-3.5 text-zinc-500" />
          <input
            value={p.filter}
            onChange={(e) => p.onFilter(e.target.value)}
            onKeyDown={(e) => e.key === "Escape" && p.onFilter("")}
            placeholder="Filtrar nesta pasta"
            className="w-full bg-transparent text-xs text-zinc-100 placeholder:text-zinc-600 focus:outline-none"
          />
        </label>

        <button
          type="button"
          onClick={p.onToggleHidden}
          title="Mostrar arquivos ocultos (.dotfiles)"
          aria-pressed={p.showHidden}
          className={`ml-1 flex items-center gap-1.5 rounded-full border px-2.5 py-1 text-xs transition-colors ${
            p.showHidden
              ? "border-zinc-100 bg-zinc-100 font-semibold text-void"
              : "border-zinc-800 text-zinc-400 hover:border-zinc-600 hover:text-zinc-100"
          }`}
        >
          {p.showHidden ? <Eye className="size-3.5" /> : <EyeOff className="size-3.5" />}
          Ocultos
        </button>

        <div className="ml-auto flex items-center gap-1.5">
          <button
            type="button"
            onClick={() => p.onNew("dir")}
            className="flex items-center gap-1.5 rounded-full border border-zinc-800 px-2.5 py-1 text-xs whitespace-nowrap text-zinc-300 transition-colors hover:border-zinc-600 hover:text-zinc-100"
          >
            <FolderPlus className="size-3.5" />
            Nova pasta
          </button>
          <button
            type="button"
            onClick={() => p.onNew("file")}
            className="flex items-center gap-1.5 rounded-full border border-zinc-800 px-2.5 py-1 text-xs whitespace-nowrap text-zinc-300 transition-colors hover:border-zinc-600 hover:text-zinc-100"
          >
            <FilePlus className="size-3.5" />
            Novo arquivo
          </button>
        </div>
      </div>

      <div className="flex items-center gap-2">
        <nav aria-label="Caminho" className="flex min-w-0 flex-1 items-center gap-0.5 overflow-x-auto text-sm">
          <button
            type="button"
            onClick={() => p.onNavigate("/")}
            className="shrink-0 rounded px-1.5 py-0.5 font-medium text-zinc-300 hover:bg-zinc-800 hover:text-zinc-100"
          >
            {p.distro}
          </button>
          {segments.map((seg, i) => {
            const target = "/" + segments.slice(0, i + 1).join("/");
            const last = i === segments.length - 1;
            return (
              <span key={target} className="flex shrink-0 items-center gap-0.5">
                <ChevronRight className="size-3.5 text-zinc-600" />
                <button
                  type="button"
                  onClick={() => p.onNavigate(target)}
                  aria-current={last ? "page" : undefined}
                  className={`rounded px-1.5 py-0.5 hover:bg-zinc-800 ${
                    last ? "font-medium text-zinc-100" : "text-zinc-400 hover:text-zinc-100"
                  }`}
                >
                  {seg}
                </button>
              </span>
            );
          })}
        </nav>
        <CopyButton label="Linux" value={p.path} />
        <CopyButton label="Windows" value={p.windowsPath} />
      </div>
    </div>
  );
}
