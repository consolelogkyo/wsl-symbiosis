import { invoke } from "@tauri-apps/api/core";

// Mirrors of src-tauri/src/backup.rs (wsl --export / --import).

export interface BackupResult {
  file: string;
  size: number;
  seconds: number;
}

export interface ImportResult {
  distro: string;
  /** User the restored distro opens with; null = root. */
  defaultUser: string | null;
  seconds: number;
}

export interface BackupDefaults {
  backupDir: string;
  installRoot: string;
}

export const exportDistroBackup = (distro: string, destinationFile: string) =>
  invoke<BackupResult>("export_distro_backup", { distro, destinationFile });
export const importDistroBackup = (newDistroName: string, installDir: string, sourceTarFile: string) =>
  invoke<ImportResult>("import_distro_backup", { newDistroName, installDir, sourceTarFile });
export const getBackupDefaults = () => invoke<BackupDefaults>("get_backup_defaults");

/** Mirrors `validate_distro_name` (wsl.rs): what wsl --import accepts as a name. */
export function distroNameError(name: string, existing: string[]): string | null {
  if (!name) return null;
  if (!/^[A-Za-z0-9._-]{1,64}$/.test(name) || name.startsWith("-")) return "Use letras, números, ponto, hífen ou _ (sem espaços)";
  if (existing.some((e) => e.toLowerCase() === name.toLowerCase())) return "Já existe uma distro com esse nome";
  return null;
}

/** `Ubuntu_2026-10-28.tar` */
export function backupFileName(distro: string, date = new Date()): string {
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${distro}_${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}.tar`;
}

/** "3 min 12 s" */
export function formatDuration(seconds: number): string {
  const m = Math.floor(seconds / 60);
  const s = Math.floor(seconds % 60);
  return m > 0 ? `${m} min ${s} s` : `${s} s`;
}
