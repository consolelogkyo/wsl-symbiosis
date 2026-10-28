import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { Distro } from "./wsl";

// Mirrors of src-tauri/src/provisioner.rs (setup "Zero-to-Hero").

export interface WindowsTerminalState {
  installed: boolean;
  settingsPath: string | null;
  /** The selected distro's profile is Windows Terminal's default. */
  ubuntuIsDefault: boolean;
  hasDraculaScheme: boolean;
  ubuntuUsesDracula: boolean;
  /** Windows Terminal already lists a profile for the distro (it creates them itself). */
  profileFound: boolean;
}

export interface LinuxState {
  user: string;
  hasApt: boolean;
  zshInstalled: boolean;
  zshIsDefaultShell: boolean;
  ohMyZshInstalled: boolean;
  spaceshipInstalled: boolean;
  /** Active ZSH_THEME, e.g. "powerlevel10k/powerlevel10k". */
  zshTheme: string | null;
  zshPlugins: { autosuggestions: boolean; syntaxHighlighting: boolean; completions: boolean };
  nvmInstalled: boolean;
  nodeLtsInstalled: boolean;
}

export interface SetupStateInspection {
  wslInstalled: boolean;
  wslVersion: string | null;
  needsReboot: boolean;
  distros: Distro[];
  hasLinuxDistro: boolean;
  windowsTerminal: WindowsTerminalState;
  distro: string | null;
  distroRunning: boolean;
  /** null: no distro, or it is stopped (inspection never boots it). */
  linux: LinuxState | null;
}

export interface WslEnableResult {
  needsReboot: boolean;
  message: string;
}

export type StepStatus = "pending" | "running" | "done" | "warning" | "skipped" | "error";

export interface ProvisionProgress {
  stepId: string;
  status: Exclude<StepStatus, "pending">;
  message: string | null;
  logLine: string | null;
}

export interface StepResult {
  stepId: string;
  status: StepStatus;
  message: string;
}

export type ModuleId = "windows_terminal_dracula" | "zsh_ohmyzsh" | "spaceship_theme" | "zsh_plugins" | "nvm_node_lts";

export const inspectSetupState = (distro: string | null) =>
  invoke<SetupStateInspection>("inspect_setup_state", { distro });
export const enableWslEngine = () => invoke<WslEnableResult>("enable_wsl_engine");
/** Startup rule (provisioner::setup_required): first run, coming from 0.1.2, reboot pending, or
 *  no development distro (only docker-desktop & co.). */
export const shouldOpenSetup = () => invoke<boolean>("should_open_setup");
/** The password goes to `chpasswd` through stdin inside the distro; it is never stored. */
export const installUbuntuDistro = (distroName: string, username: string, password: string) =>
  invoke<void>("install_ubuntu_distro", { distroName, username, password });
export const runProvisioningTasks = (distro: string, modules: ModuleId[]) =>
  invoke<StepResult[]>("run_provisioning_tasks", { distro, modules });
export const onProvisionProgress = (cb: (p: ProvisionProgress) => void): Promise<UnlistenFn> =>
  listen<ProvisionProgress>("provision-progress", (e) => cb(e.payload));

export interface OnlineDistro {
  name: string;
  friendlyName: string;
}

/** Ubuntu/Debian from this machine's `wsl --list --online` (the catalog varies by WSL version). */
export const listOnlineDistros = () => invoke<OnlineDistro[]>("list_online_distros");

/** Offline or catalog unavailable: `Ubuntu` exists in every catalog. */
export const FALLBACK_DISTROS: OnlineDistro[] = [{ name: "Ubuntu", friendlyName: "Ubuntu" }];

/** Mirrors validate_linux_username (provisioner.rs). */
export function linuxUsernameError(name: string): string | null {
  if (!name) return null;
  if (name === "root") return "Escolha um usuário diferente de root.";
  if (name.length > 32 || !/^[a-z_][a-z0-9_-]*$/.test(name)) {
    return "Use letras minúsculas, números, _ ou -, começando por letra.";
  }
  return null;
}

/** Official download pages (opened in the browser; the app itself never downloads them). */
export const IDE_DOWNLOADS: Record<"antigravity" | "vscode" | "cursor", string> = {
  antigravity: "https://antigravity.google/download",
  vscode: "https://code.visualstudio.com/",
  cursor: "https://cursor.com/",
};

export interface ModuleDef {
  id: ModuleId;
  title: string;
  detail: string;
  /** Runs inside the distro (needs one selected). */
  linux: boolean;
}

export const MODULE_DEFS: ModuleDef[] = [
  {
    id: "windows_terminal_dracula",
    title: "Windows Terminal + Tema Dracula",
    detail: "Instala o Windows Terminal se faltar, deixa a distro como perfil padrão e aplica o Dracula.",
    linux: false,
  },
  {
    id: "zsh_ohmyzsh",
    title: "Zsh + Oh-My-Zsh",
    detail: "Instala zsh, git e curl (apt), o Oh-My-Zsh e define o zsh como shell padrão.",
    linux: true,
  },
  {
    id: "spaceship_theme",
    title: "Tema Spaceship Prompt",
    detail: "Baixa o Spaceship e define ZSH_THEME=\"spaceship\" no ~/.zshrc.",
    linux: true,
  },
  {
    id: "zsh_plugins",
    title: "Plugins Zsh",
    detail: "Autosuggestions, Syntax Highlighting e Completions, sem remover seus plugins atuais.",
    linux: true,
  },
  {
    id: "nvm_node_lts",
    title: "NVM + Node.js LTS",
    detail: "Instala o nvm e o Node.js LTS como versão padrão.",
    linux: true,
  },
];

export type ModuleState = "configured" | "missing" | "unknown" | "unavailable";

export interface ModuleAssessment {
  state: ModuleState;
  /** Extra line under the card (e.g. another theme would be replaced). */
  note?: string;
  /** Default checkbox value. */
  recommended: boolean;
}

/** What the inspection says about each module. */
export function assessModule(id: ModuleId, s: SetupStateInspection | null): ModuleAssessment {
  if (!s) return { state: "unknown", recommended: false };
  const wt = s.windowsTerminal;
  const l = s.linux;
  if (id === "windows_terminal_dracula") {
    if (!s.distro) return { state: "unavailable", note: "Precisa de uma distro instalada.", recommended: false };
    const done = wt.installed && wt.profileFound && wt.ubuntuIsDefault && wt.ubuntuUsesDracula;
    return { state: done ? "configured" : "missing", recommended: !done };
  }
  if (!s.distro) return { state: "unavailable", note: "Precisa de uma distro instalada.", recommended: false };
  if (!l) return { state: "unknown", note: "Distro parada: ligue-a para analisar.", recommended: false };
  switch (id) {
    case "zsh_ohmyzsh": {
      if (!l.hasApt && !l.zshInstalled) return { state: "unavailable", note: "Feito para Ubuntu/Debian (apt).", recommended: false };
      const done = l.zshInstalled && l.zshIsDefaultShell && l.ohMyZshInstalled;
      return { state: done ? "configured" : "missing", recommended: !done };
    }
    case "spaceship_theme": {
      if (l.spaceshipInstalled) return { state: "configured", recommended: false };
      const other = l.zshTheme && l.zshTheme !== "robbyrussell" ? l.zshTheme : null;
      return other
        ? { state: "missing", note: `Substitui o tema atual (${other}).`, recommended: false }
        : { state: "missing", recommended: true };
    }
    case "zsh_plugins": {
      const p = l.zshPlugins;
      const done = p.autosuggestions && p.syntaxHighlighting && p.completions;
      return { state: done ? "configured" : "missing", recommended: !done };
    }
    case "nvm_node_lts": {
      const done = l.nvmInstalled && l.nodeLtsInstalled;
      return { state: done ? "configured" : "missing", recommended: !done };
    }
  }
  return { state: "unknown", recommended: false };
}
