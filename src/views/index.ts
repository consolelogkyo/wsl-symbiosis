import { FolderTree, LayoutDashboard, Package, SquareTerminal, type LucideIcon } from "lucide-react";

export type ViewId = "dashboard" | "explorer" | "packages" | "terminal";

export interface NavItem {
  id: ViewId;
  label: string;
  icon: LucideIcon;
}

export const NAV_ITEMS: NavItem[] = [
  { id: "dashboard", label: "Dashboard & Recursos", icon: LayoutDashboard },
  { id: "explorer", label: "Explorador & Projetos", icon: FolderTree },
  { id: "packages", label: "Pacotes & Ambiente", icon: Package },
  { id: "terminal", label: "Terminal", icon: SquareTerminal },
];