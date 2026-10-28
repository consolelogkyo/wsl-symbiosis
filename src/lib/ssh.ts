import { invoke } from "@tauri-apps/api/core";

// Mirrors of src-tauri/src/ssh.rs (Git identity and SSH keys).

export interface SshKeyInfo {
  name: string;
  /** ED25519, ECDSA, RSA. */
  kind: string | null;
  /** The line to paste into GitHub; null when the .pub file is missing. */
  publicKey: string | null;
  fingerprint: string | null;
  /** Octal mode of the private key. */
  mode: string;
}

export interface WindowsKey {
  name: string;
  hasPublic: boolean;
  existsInLinux: boolean;
}

export interface GitIdentityStatus {
  name: string | null;
  email: string | null;
  gitInstalled: boolean;
  keygenInstalled: boolean;
  sshDirMode: string | null;
  keys: SshKeyInfo[];
  /** False when ssh would refuse the key ("UNPROTECTED PRIVATE KEY FILE"). */
  permissionsOk: boolean;
  windowsKeys: WindowsKey[];
}

export interface SshImportResult {
  imported: string[];
  skipped: string[];
}

export interface SshTestResult {
  ok: boolean;
  username: string | null;
  message: string;
  detail: string;
}

export const GITHUB_KEYS_URL = "https://github.com/settings/keys";

export const getGitIdentity = (distro: string) => invoke<GitIdentityStatus>("get_git_identity_and_ssh", { distro });
export const saveGitIdentity = (distro: string, name: string, email: string) =>
  invoke<void>("save_git_identity", { distro, name, email });
export const generateSshKey = (distro: string, email: string) => invoke<GitIdentityStatus>("generate_ssh_key", { distro, email });
export const importWindowsSshKeys = (distro: string) => invoke<SshImportResult>("import_windows_ssh_keys", { distro });
export const fixSshPermissions = (distro: string) => invoke<void>("fix_ssh_permissions", { distro });
export const testSshGithub = (distro: string) => invoke<SshTestResult>("test_ssh_github", { distro });

/** Mirrors `validate_git_email` closely enough for live feedback (the backend has the last word). */
export function emailError(email: string): string | null {
  const e = email.trim();
  if (!e) return null;
  return /^[^\s@<>]+@[^\s@<>.][^\s@<>]*\.[^\s@<>]+$/.test(e) && !e.includes("..") && !e.endsWith(".") ? null : "E-mail inválido";
}
