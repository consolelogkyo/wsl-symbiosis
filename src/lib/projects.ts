import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

// Mirrors of src-tauri/src/projects.rs.

export interface ProjectScript {
  name: string;
  command: string;
}

export interface ProjectInfo {
  packageManager: "npm" | "pnpm" | "yarn" | "bun" | string;
  scripts: ProjectScript[];
  /** What "Rodar" runs: dev > start:dev > develop > serve > start. */
  suggested: string | null;
  hasNodeModules: boolean;
}

export type TaskKind = "run" | "install";
export type TaskStatus = "starting" | "running" | "stopping" | "stopped" | "exited" | "failed";

export interface TaskInfo {
  id: string;
  distro: string;
  path: string;
  name: string;
  kind: TaskKind;
  command: string;
  status: TaskStatus;
  exitCode: number | null;
  pid: number | null;
  /** Local URLs printed by the dev server. */
  urls: string[];
  startedAt: number;
}

export const isActive = (t: TaskInfo) => t.status === "starting" || t.status === "running" || t.status === "stopping";

export const inspectProject = (distro: string, path: string) => invoke<ProjectInfo>("inspect_project", { distro, path });
export const startProject = (distro: string, path: string, kind: TaskKind, script?: string) =>
  invoke<TaskInfo>("start_project", { distro, path, kind, script: script ?? null });
export const stopProject = (id: string) => invoke<void>("stop_project", { id });
export const listProjectTasks = () => invoke<TaskInfo[]>("list_project_tasks");
export const getProjectLog = (id: string) => invoke<string[]>("get_project_log", { id });
export const dismissProjectTask = (id: string) => invoke<void>("dismiss_project_task", { id });
/** Stops a listening process the app didn't start (its whole tree, as root). */
export const stopProcessTree = (distro: string, pid: number) => invoke<void>("stop_process_tree", { distro, pid });

export const onProjectTask = (handler: (task: TaskInfo) => void) =>
  listen<TaskInfo>("project-task", ({ payload }) => handler(payload));

/** `/home/kyo/dev/api` contains `/home/kyo/dev/api/dist`. */
export const isInside = (path: string, root: string) => path === root || path.startsWith(root.endsWith("/") ? root : `${root}/`);
