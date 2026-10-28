import { invoke } from "@tauri-apps/api/core";

// Mirrors of src-tauri/src/wslconfig.rs. null = key absent (WSL's default applies).

export type NetworkingMode = "nat" | "mirrored" | "virtioproxy";
export type MemoryReclaim = "disabled" | "gradual" | "dropcache";

export interface WslGlobalConfig {
  /** Bytes. Default: 50% of the Windows RAM. */
  memory: number | null;
  /** Default: every logical CPU. */
  processors: number | null;
  /** Bytes; 0 disables swap. Default: 25% of the Windows RAM. */
  swap: number | null;
  localhostForwarding: boolean | null;
  networkingMode: string | null;
  guiApplications: boolean | null;
  autoMemoryReclaim: string | null;
  sparseVhd: boolean | null;
}

export interface WslConfigSnapshot {
  config: WslGlobalConfig;
  path: string;
  exists: boolean;
  hasBackup: boolean;
  /** Keys the editor doesn't manage, kept as they are (`wsl2.kernel`...). */
  preserved: string[];
  hostMemory: number;
  hostCpus: number;
}

export const GIB = 1024 ** 3;

export const getWslConfig = () => invoke<WslConfigSnapshot>("get_wsl_config");
export const saveWslConfig = (config: WslGlobalConfig, restartWsl: boolean) =>
  invoke<WslConfigSnapshot>("save_wsl_config", { config, restartWsl });

/** Field-by-field comparison: what the save will write. */
export function configChanged(a: WslGlobalConfig, b: WslGlobalConfig): boolean {
  return (Object.keys(a) as (keyof WslGlobalConfig)[]).some((k) => a[k] !== b[k]);
}
