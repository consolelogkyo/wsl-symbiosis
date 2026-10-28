import { invoke } from "@tauri-apps/api/core";

// Mirrors of the structs in src-tauri/src/wsl.rs (serde camelCase).

export type DistroState =
  | "Running"
  | "Stopped"
  | "Installing"
  | "Converting"
  | "Uninstalling"
  | "Unknown";

export interface Distro {
  name: string;
  state: DistroState;
  /** Raw (possibly localized) state text from wsl.exe. */
  stateLabel: string;
  version: number;
  isDefault: boolean;
}

export interface WslInfo {
  wslVersion: string | null;
  kernelVersion: string | null;
}

// Mirrors of src-tauri/src/metrics.rs. All sizes are in bytes.

export interface VmProcess {
  name: string;
  pid: number;
  /** Private working set (Task Manager's "Memory" column). */
  memory: number;
  /** Normalized to the whole host, 0-100. */
  cpuUsage: number;
}

/** wslservice / wslhost / wslrelay / wsl.exe, summed. */
export interface WslHelpers {
  memory: number;
  processes: number;
}

export interface HostMetrics {
  totalMemory: number;
  usedMemory: number;
  cpuUsage: number;
  cpuCount: number;
  /** The utility VM (vmmemWSL); null while WSL is shut down. */
  vm: VmProcess | null;
  wslHelpers: WslHelpers;
}

/** Shared by every distro: WSL2 runs them all on one VM kernel. */
export interface GuestMemory {
  total: number;
  used: number;
  free: number;
  available: number;
  cache: number;
}

export interface LoadAverage {
  one: number;
  five: number;
  fifteen: number;
  running: number;
  total: number;
}

export interface DiskUsage {
  filesystem: string;
  total: number;
  used: number;
  available: number;
}

export interface LinuxProcess {
  pid: number;
  ppid: number;
  user: string;
  /** procps lifetime %CPU; null on busybox distros. */
  cpu: number | null;
  rss: number;
  command: string;
}

export interface GuestMetrics {
  memory: GuestMemory;
  load: LoadAverage;
  cpuUsage: number | null;
  cpuCount: number;
  disk: DiskUsage | null;
  processes: LinuxProcess[];
}

export interface WslResourceMetrics {
  distro: string;
  host: HostMetrics;
  guest: GuestMetrics | null;
  guestError: string | null;
  /** The distro's ext4.vhdx on Windows (also while stopped); null for WSL1 / not found. */
  vhdx: VhdxInfo | null;
}

// Mirrors of src-tauri/src/vhdx.rs.

export interface VhdxInfo {
  path: string;
  /** What the file takes on the Windows disk. */
  fileSize: number;
}

export interface VhdxCompactResult {
  path: string;
  sizeBefore: number;
  sizeAfter: number;
  reclaimedBytes: number;
  method: "optimize-vhd" | "diskpart";
  trimmed: boolean;
}

export interface DropCachesResult {
  cacheBefore: number;
  cacheAfter: number;
  freeBefore: number;
  freeAfter: number;
}

export type KillSignal = "TERM" | "KILL";

export const listDistros = () => invoke<Distro[]>("list_distros");

/** Internal VMs of desktop tools (Docker/Rancher Desktop, Podman): never a development distro.
 *  Mirrors `is_system_distro` (provisioner.rs). */
export function isSystemDistro(name: string): boolean {
  return /^(docker-desktop|rancher-desktop|podman-machine)/i.test(name);
}

/** The distro to select when nothing is chosen: a real one (WSL's default if it is real). */
export function preferredDistro(list: Distro[]): Distro | undefined {
  const real = list.filter((d) => !isSystemDistro(d.name));
  return real.find((d) => d.isDefault) ?? real[0] ?? list.find((d) => d.isDefault) ?? list[0];
}
export const getWslInfo = () => invoke<WslInfo>("get_wsl_info");
export const startDistro = (name: string) => invoke<void>("start_distro", { name });
export const shutdownWsl = () => invoke<void>("shutdown_wsl");
export const getResourceMetrics = (distro: string) =>
  invoke<WslResourceMetrics>("get_wsl_resource_metrics", { distro });
export const freeCachedMemory = (distro: string) =>
  invoke<DropCachesResult>("free_wsl_cached_memory", { distro });
export const killLinuxProcess = (distro: string, pid: number, signal: KillSignal) =>
  invoke<void>("kill_linux_process", { distro, pid, signal });
/** fstrim, wsl --shutdown, then Optimize-VHD / diskpart elevated (one UAC prompt). Takes minutes. */
export const compactDistroVhdx = (distro: string) =>
  invoke<VhdxCompactResult>("compact_distro_vhdx", { distro });

/** Tauri rejects with the Rust `Err(String)`; normalize anything else. */
export function errorMessage(err: unknown): string {
  if (typeof err === "string") return err;
  if (err instanceof Error) return err.message;
  return String(err);
}
