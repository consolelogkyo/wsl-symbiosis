import { invoke } from "@tauri-apps/api/core";

// Mirrors of src-tauri/src/explorer.rs (serde camelCase).

export type EntryKind = "dir" | "file" | "symlink";

export interface DirEntry {
  name: string;
  /** Absolute Linux path. */
  path: string;
  kind: EntryKind;
  /** Symlinks only: whether the target is a directory. */
  targetIsDir: boolean;
  size: number;
  /** Unix epoch seconds. */
  modified: number;
  /** e.g. "rwxr-xr-x". */
  permissions: string;
  projectMarkers: string[];
  /** Git repositories: checked-out branch, or the short commit when `gitDetached`. */
  gitBranch: string | null;
  gitDetached: boolean;
}

export interface DirectoryListing {
  path: string;
  parent: string | null;
  windowsPath: string;
  entries: DirEntry[];
}

export type LineEnding = "lf" | "crlf" | "mixed" | "none";

export interface TextFileContent {
  path: string;
  content: string;
  size: number;
  truncated: boolean;
  binary: boolean;
  lineEnding: LineEnding;
}

export type ToolId = "antigravity" | "vscode" | "cursor" | "explorer" | "windows_terminal";

/** custom_missing: the configured file is gone, auto-detection took over. */
export type ToolSource = "custom" | "custom_missing" | "auto" | "none";

export interface ToolInfo {
  id: ToolId;
  label: string;
  available: boolean;
  /** Launcher actually used: custom path first, then auto-detected. */
  path: string | null;
  autoPath: string | null;
  customPath: string | null;
  source: ToolSource;
  customizable: boolean;
  /** Official icon extracted from the install (PNG data URL); filled in by useTools. */
  icon?: string;
}

export const IDE_IDS: ToolId[] = ["antigravity", "vscode", "cursor"];

export const getDistroHome = (distro: string) => invoke<string>("get_distro_home", { distro });
export const listDirectory = (distro: string, path: string, showHidden: boolean) =>
  invoke<DirectoryListing>("list_directory", { distro, path, showHidden });
export const createDirectory = (distro: string, path: string) => invoke<void>("create_directory", { distro, path });
export const createFile = (distro: string, path: string) => invoke<void>("create_file", { distro, path });
export const renameEntry = (distro: string, oldPath: string, newPath: string) =>
  invoke<void>("rename_entry", { distro, oldPath, newPath });
export const deleteEntry = (distro: string, path: string) => invoke<void>("delete_entry", { distro, path });
export const readTextFile = (distro: string, path: string, maxBytes?: number) =>
  invoke<TextFileContent>("read_text_file", { distro, path, maxBytes });
export const writeTextFile = (distro: string, path: string, content: string) =>
  invoke<void>("write_text_file", { distro, path, content });
export const openInTool = (distro: string, linuxPath: string, tool: ToolId) =>
  invoke<void>("open_in_tool", { distro, linuxPath, tool });
export const detectTools = () => invoke<ToolInfo[]>("detect_tools");
/** Tool id → PNG data URL, extracted from the installed executables (no network). */
export const getToolIcons = () => invoke<Record<string, string>>("get_tool_icons");

// --- Path helpers (mirror explorer.rs; the backend re-validates everything) ---

export function joinPath(dir: string, name: string): string {
  return dir === "/" ? `/${name}` : `${dir}/${name}`;
}

export function parentPath(path: string): string | null {
  if (path === "/") return null;
  const i = path.lastIndexOf("/");
  return i <= 0 ? "/" : path.slice(0, i);
}

export function toWindowsPath(distro: string, path: string): string {
  return `\\\\wsl.localhost\\${distro}${path.split("/").join("\\")}`;
}

/** Same rule as `explorer::is_protected` — used only to hide the delete button. */
export function isProtectedPath(path: string): boolean {
  const parts = path.split("/").filter(Boolean);
  return parts.length <= 1 || (parts.length === 2 && (parts[0] === "home" || parts[0] === "mnt"));
}

export function validateEntryName(name: string): string | null {
  if (!name || name === "." || name === "..") return "Nome inválido";
  if (name.includes("/")) return "O nome não pode conter '/'";
  return null;
}

export const isNavigable = (e: DirEntry) => e.kind === "dir" || (e.kind === "symlink" && e.targetIsDir);
