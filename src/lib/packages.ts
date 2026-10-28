import { invoke } from "@tauri-apps/api/core";

// Mirrors of src-tauri/src/packages.rs (serde camelCase).

export type ToolCategory = "runtime" | "package_manager" | "tool" | "version_manager";

export interface DetectedTool {
  id: string;
  label: string;
  category: ToolCategory;
  installed: boolean;
  version: string | null;
  rawVersion: string | null;
  /** Binary path, or the bare name for shell functions (nvm). */
  path: string | null;
}

export interface GlobalPackage {
  name: string;
  version: string;
  /** Ships with Node (npm, corepack). */
  bundled: boolean;
}

export interface NodeVersion {
  version: string;
  active: boolean;
  isDefault: boolean;
}

export interface NvmInfo {
  dir: string;
  versions: NodeVersion[];
  defaultAlias: string | null;
}

export interface ListeningPort {
  protocol: string;
  localAddress: string;
  port: number;
  /** null when owned by another distro or by WSL itself. */
  pid: number | null;
  process: string | null;
  user: string | null;
  /** Working directory of the process: the project folder for dev servers. */
  cwd: string | null;
  command: string | null;
}

/** A global package of one nvm Node version. */
export interface NodeGlobalPackage {
  nodeVersion: string;
  name: string;
  version: string;
}

export interface EnvironmentScan {
  distro: string;
  shell: string;
  tools: DetectedTool[];
  nvm: NvmInfo | null;
  /** null when the distro has no Linux npm. */
  npmPrefix: string | null;
  npmGlobals: GlobalPackage[];
  /** Globals of every nvm Node version. */
  nvmGlobals: NodeGlobalPackage[];
  listeningPorts: ListeningPort[];
  portsError: string | null;
  durationMs: number;
}

/**
 * Ports that belong to the user's work (dev servers, APIs), not to system daemons: owned by
 * a process whose working directory isn't `/`.
 */
export const isUserPort = (p: ListeningPort) => p.pid !== null && !!p.cwd && p.cwd !== "/" && p.user !== "root";

export const portUrl = (p: ListeningPort) => `http://localhost:${p.port}`;

export const scanEnvironment = (distro: string) => invoke<EnvironmentScan>("scan_environment", { distro });
export const listListeningPorts = (distro: string) => invoke<ListeningPort[]>("list_listening_ports", { distro });
export const installNpmGlobal = (distro: string, packageSpec: string) =>
  invoke<string>("install_npm_global", { distro, packageSpec });
export const uninstallNpmGlobal = (distro: string, packageName: string) =>
  invoke<string>("uninstall_npm_global", { distro, packageName });
export const switchNvmDefault = (distro: string, version: string) =>
  invoke<string>("switch_nvm_default", { distro, version });

/** Same rule as `packages::validate_package_spec` (the backend re-validates). */
export function packageSpecError(spec: string): string | null {
  const part = /^[A-Za-z0-9~-][A-Za-z0-9._~-]*$/;
  if (!spec) return null;
  if (spec.startsWith("-")) return "Flags não são permitidas";
  const m = /^(?:@([^/@]+)\/)?([^@]+)(?:@(.+))?$/.exec(spec);
  if (!m || (m[1] !== undefined && !part.test(m[1])) || !part.test(m[2])) return "Nome de pacote inválido";
  if (m[3] !== undefined && !/^[A-Za-z0-9.^~<>=*+-]+$/.test(m[3])) return "Versão inválida";
  return null;
}

/** `/home/kyo/.nvm/...` → `~/.nvm/...` */
export function shortenHome(path: string): string {
  return path.replace(/^\/home\/[^/]+(?=\/|$)/, "~").replace(/^\/root(?=\/|$)/, "~");
}
