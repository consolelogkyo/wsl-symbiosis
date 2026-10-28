import { createContext, useCallback, useContext, useEffect, useRef, useState } from "react";
import { type PullResult, formatStat, gitPull, splitGitError } from "../lib/git";
import { errorMessage } from "../lib/wsl";
import type { Activity } from "./useActivity";

export type PullStatus = "queued" | "running" | "updated" | "upToDate" | "failed" | "cancelled";

export interface PullItem {
  id: number;
  distro: string;
  path: string;
  name: string;
  status: PullStatus;
  result?: PullResult;
  /** Failures: the explanation and git's own output. */
  error?: { title: string; detail: string };
}

export interface GitQueue {
  items: PullItem[];
  /** Adds repositories to the end of the queue (skipping ones already waiting or running). */
  enqueue: (repos: { distro: string; path: string; name: string }[]) => void;
  /** Drops the repositories still waiting; the one running finishes. */
  cancelPending: () => void;
  retryFailed: () => void;
  /** Removes finished items (the panel closes when nothing is left). */
  clear: () => void;
  /** Latest item of a repository, to show its state in the Explorer row. */
  itemFor: (distro: string, path: string) => PullItem | undefined;
}

export const GitQueueContext = createContext<GitQueue | null>(null);

export function useGitQueueContext(): GitQueue {
  const ctx = useContext(GitQueueContext);
  if (!ctx) throw new Error("GitQueueContext ausente");
  return ctx;
}

const isWaiting = (i: PullItem) => i.status === "queued" || i.status === "running";
let seq = 1;

/**
 * Pulls repositories one at a time (`git pull --ff-only`, see git.rs): one after the other is
 * gentler on the network and the WSL VM than dozens of parallel pulls, and keeps the log
 * readable. Lives in the App, so the queue keeps going while the user switches screens.
 */
export function useGitQueue(notify: Activity["notify"]): GitQueue {
  const [items, setItems] = useState<PullItem[]>([]);
  const busy = useRef(false);

  const patch = useCallback((id: number, p: Partial<PullItem>) => {
    setItems((all) => all.map((i) => (i.id === id ? { ...i, ...p } : i)));
  }, []);

  // Runner: whenever nothing is running, start the next queued item.
  useEffect(() => {
    if (busy.current) return;
    const next = items.find((i) => i.status === "queued");
    if (!next) return;
    busy.current = true;
    patch(next.id, { status: "running" });
    gitPull(next.distro, next.path)
      .then((result) => patch(next.id, { status: result.commits > 0 ? "updated" : "upToDate", result }))
      .catch((err) => patch(next.id, { status: "failed", error: splitGitError(errorMessage(err)) }))
      .finally(() => {
        busy.current = false;
        // Re-run the effect even if nothing else changed.
        setItems((all) => [...all]);
      });
  }, [items, patch]);

  // Summary toast when a batch ends (once per batch).
  const announced = useRef(true);
  useEffect(() => {
    if (items.some(isWaiting)) {
      announced.current = false;
      return;
    }
    if (announced.current || items.length === 0) return;
    announced.current = true;
    const done = items.filter((i) => i.status !== "cancelled");
    const updated = done.filter((i) => i.status === "updated");
    const failed = done.filter((i) => i.status === "failed");
    if (done.length === 1) {
      const [only] = done;
      notify(
        only.status === "failed"
          ? { tone: "error", title: `git pull falhou em ${only.name}`, detail: only.error?.title }
          : {
              tone: "success",
              title: only.status === "updated" ? `${only.name} atualizado` : `${only.name} já estava em dia`,
              detail:
                only.result &&
                [
                  only.result.commits ? `${only.result.commits} commits · ${formatStat(only.result.stat)}` : only.result.branch,
                  only.result.viaSsh ? "via SSH" : "",
                ]
                  .filter(Boolean)
                  .join(" · "),
            },
      );
      return;
    }
    const parts = [
      `${updated.length} atualizado${updated.length === 1 ? "" : "s"}`,
      `${done.length - updated.length - failed.length} em dia`,
      failed.length ? `${failed.length} com erro` : "",
    ].filter(Boolean);
    notify({ tone: failed.length ? "error" : "success", title: `git pull concluído em ${done.length} repositórios`, detail: parts.join(" · ") });
  }, [items, notify]);

  const enqueue = useCallback<GitQueue["enqueue"]>((repos) => {
    setItems((all) => {
      // A new batch after a finished one starts with a clean list.
      const base = all.some(isWaiting) ? all : [];
      const fresh = repos
        .filter((r) => !base.some((i) => isWaiting(i) && i.distro === r.distro && i.path === r.path))
        .map((r) => ({ ...r, id: seq++, status: "queued" as const }));
      return [...base, ...fresh];
    });
  }, []);

  const cancelPending = useCallback(() => {
    setItems((all) => all.map((i) => (i.status === "queued" ? { ...i, status: "cancelled" } : i)));
  }, []);

  const retryFailed = useCallback(() => {
    setItems((all) =>
      all.map((i) => (i.status === "failed" || i.status === "cancelled" ? { ...i, status: "queued", error: undefined } : i)),
    );
  }, []);

  const clear = useCallback(() => setItems((all) => all.filter(isWaiting)), []);

  const itemFor = useCallback(
    (distro: string, path: string) => {
      for (let i = items.length - 1; i >= 0; i--) {
        if (items[i].distro === distro && items[i].path === path) return items[i];
      }
      return undefined;
    },
    [items],
  );

  return { items, enqueue, cancelPending, retryFailed, clear, itemFor };
}
