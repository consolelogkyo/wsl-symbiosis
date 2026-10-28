import { invoke } from "@tauri-apps/api/core";

// Mirrors of src-tauri/src/settings.rs.

export interface Settings {
  onboardingCompleted: boolean;
  /** Tool id → launcher path (.exe/.cmd/.bat). Missing id = auto-detect. */
  customToolPaths: Record<string, string>;
  defaultDistro: string | null;
  /** antigravity | vscode | cursor; null = first installed. */
  defaultIde: string | null;
  /** Shortcut id → binding, only the ones changed by the user ("" = disabled). See shortcuts.ts. */
  shortcuts: Record<string, string>;
  /** How git pull/fetch reach HTTPS remotes: retry over SSH when asked for a password (auto),
   *  always SSH, or the repository's remote as is. */
  gitAuth: GitAuth;
  /** Setup enabled WSL and Windows must restart (owned by the backend; saving keeps it). */
  wslRebootPending?: boolean;
  /** The full-window setup was finished or skipped once (false after updating from 0.1.2). */
  setupWizardSeen?: boolean;
  /** What ✕ / Alt+F4 does: ask (CloseDialog), hide to the tray, or quit. */
  closeBehavior?: CloseBehavior;
}

export type GitAuth = "auto" | "ssh" | "remote";
export type CloseBehavior = "ask" | "tray" | "quit";

export interface SettingsSnapshot extends Settings {
  /** settings.json location, or null when settings live in memory only. */
  file: string | null;
}

export const getSettings = () => invoke<SettingsSnapshot>("get_settings");
export const saveSettings = (settings: Settings) => invoke<SettingsSnapshot>("save_settings", { settings });
/** "Lembrar minha escolha" in the close dialog (changes only this field). */
export const setCloseBehavior = (behavior: CloseBehavior) =>
  invoke<SettingsSnapshot>("set_close_behavior", { behavior });
/** Normalized path, null for empty input; rejects with the reason it can't be used. */
export const checkToolPath = (path: string) => invoke<string | null>("check_tool_path", { path });
