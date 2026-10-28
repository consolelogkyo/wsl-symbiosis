import { invoke } from "@tauri-apps/api/core";

// Mirrors of src-tauri/src/git.rs.

export interface LocalBranch {
  name: string;
  /** "origin/dev" */
  upstream: string | null;
  ahead: number;
  behind: number;
  /** The upstream was deleted on the remote. */
  gone: boolean;
}

export interface GitBranches {
  current: string | null;
  /** Short commit when HEAD is detached. */
  detached: string | null;
  local: LocalBranch[];
  /** Remote branches with no local branch of the same name ("origin/prod"). */
  remote: string[];
  /** Modified/staged tracked files. */
  changes: number;
  untracked: number;
}

export interface PullResult {
  branch: string;
  /** 0 = already up to date. */
  commits: number;
  /** "3 files changed, 10 insertions(+), 2 deletions(-)" */
  stat: string;
  /** An HTTPS remote was reached over SSH (Settings → Git). */
  viaSsh: boolean;
}

export const gitBranches = (distro: string, path: string) => invoke<GitBranches>("git_branches", { distro, path });
/** `remote`: create a local branch tracking `branch` ("origin/dev" → "dev"). */
export const gitSwitch = (distro: string, path: string, branch: string, remote: boolean) =>
  invoke<void>("git_switch", { distro, path, branch, remote });
export const gitFetch = (distro: string, path: string) => invoke<void>("git_fetch", { distro, path });
export const gitPull = (distro: string, path: string) => invoke<PullResult>("git_pull", { distro, path });

export interface CloneResult {
  /** The new repository folder. */
  path: string;
  name: string;
  viaSsh: boolean;
}

/** `name` null = the repository's own name, like `git clone`. */
export const gitClone = (distro: string, parent: string, url: string, name: string | null) =>
  invoke<CloneResult>("git_clone", { distro, parent, url, name });

/** Same rule as git.rs `validate_clone_url`: HTTPS/HTTP/SSH/git links or `user@host:path`. */
export function isCloneUrl(url: string): boolean {
  const u = url.trim();
  if (!u || /\s/.test(u) || u.startsWith("-")) return false;
  return /^(https?|ssh|git):\/\/.+/i.test(u) || /^[\w.-]+@[\w.-]+:(?!\/\/).+/.test(u);
}

/** Folder `git clone` creates: the last path segment without ".git". */
export function repoNameFromUrl(url: string): string {
  const last = url.trim().replace(/\/+$/, "").split(/[/:]/).pop() ?? "";
  const name = last.replace(/\.git$/, "");
  return name === "." || name === ".." ? "" : name;
}

/** Backend errors: first line is the explanation, the rest is git's own output. */
export function splitGitError(message: string): { title: string; detail: string } {
  const [title, ...rest] = message.split("\n");
  return { title, detail: rest.join("\n").trim() };
}

/** "3 files changed, 10 insertions(+), 2 deletions(-)" → "3 arquivos · +10 −2". */
export function formatStat(stat: string): string {
  const n = (re: RegExp) => Number(stat.match(re)?.[1] ?? 0);
  const files = n(/(\d+) files? changed/);
  const plus = n(/(\d+) insertions?/);
  const minus = n(/(\d+) deletions?/);
  if (!files) return "";
  return [`${files} arquivo${files > 1 ? "s" : ""}`, `+${plus} −${minus}`].join(" · ");
}
