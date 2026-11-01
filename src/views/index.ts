import { Database, FolderTree, LayoutDashboard, Package, SquareTerminal, type LucideIcon } from "lucide-react";

export type ViewId = "dashboard" | "explorer" | "packages" | "terminal" | "database";

export interface NavItem {
  id: ViewId;
  label: string;
  icon: LucideIcon;
}

/** Order = Ctrl+1 … Ctrl+5. */
export const NAV_ITEMS: NavItem[] = [
  { id: "dashboard", label: "Dashboard & Recursos", icon: LayoutDashboard },
  { id: "explorer", label: "Explorador & Projetos", icon: FolderTree },
  { id: "packages", label: "Pacotes & Ambiente", icon: Package },
  { id: "terminal", label: "Terminal", icon: SquareTerminal },
  { id: "database", label: "Bancos & Containers", icon: Database },
];

/** "Ctrl+5" → the 5th module; null for anything else. */
export function navShortcut(e: KeyboardEvent): ViewId | null {
  if (!e.ctrlKey || e.altKey || e.shiftKey || e.metaKey) return null;
  const n = /^Digit([1-9])$/.exec(e.code)?.[1];
  return n ? (NAV_ITEMS[Number(n) - 1]?.id ?? null) : null;
}
