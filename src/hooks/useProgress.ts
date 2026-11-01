import { createContext, useCallback, useContext, useMemo, useRef, useState } from "react";
import { errorMessage } from "../lib/wsl";

/** One long operation shown in the progress panel (ProgressDock). */
export interface ProgressOp {
  id: string;
  title: string;
  /** Current stage or the last line of output. */
  detail: string | null;
  /** 0–100 when the operation can be measured; null = it can't (the bar keeps moving). */
  percent: number | null;
  startedAt: number;
  status: "running" | "done" | "error";
}

export type ProgressPatch = Partial<Pick<ProgressOp, "title" | "detail" | "percent">>;

export interface Progress {
  ops: ProgressOp[];
  /** Runs `work` as a tracked operation: shown while it runs, "pronto" or the error at the end.
   *  `work` gets `update` to move the bar or change the stage text. Rethrows errors. */
  track: <T>(title: string, work: (update: (patch: ProgressPatch) => void) => Promise<T>, init?: ProgressPatch) => Promise<T>;
}

/** Done ones linger a moment so the full bar is seen; errors a bit longer. */
const LINGER_DONE_MS = 1600;
const LINGER_ERROR_MS = 5000;

let seq = 1;

export function useProgress(): Progress {
  const [ops, setOps] = useState<ProgressOp[]>([]);
  const timers = useRef(new Map<string, number>());

  const update = useCallback((id: string, patch: Partial<ProgressOp>) => {
    setOps((all) => all.map((op) => (op.id === id ? { ...op, ...patch } : op)));
  }, []);

  const track = useCallback(
    async <T,>(title: string, work: (update: (patch: ProgressPatch) => void) => Promise<T>, init?: ProgressPatch): Promise<T> => {
      const id = `op-${seq++}`;
      setOps((all) => [...all, { id, title, detail: null, percent: null, startedAt: Date.now(), status: "running", ...init }]);
      const finish = (status: ProgressOp["status"], detail?: string) => {
        update(id, status === "done" ? { status, percent: 100 } : { status, detail: detail ?? null });
        const timer = window.setTimeout(() => {
          setOps((all) => all.filter((op) => op.id !== id));
          timers.current.delete(id);
        }, status === "done" ? LINGER_DONE_MS : LINGER_ERROR_MS);
        timers.current.set(id, timer);
      };
      try {
        const result = await work((patch) => update(id, patch));
        finish("done");
        return result;
      } catch (err) {
        finish("error", errorMessage(err).split("\n")[0]);
        throw err;
      }
    },
    [update],
  );

  return useMemo(() => ({ ops, track }), [ops, track]);
}

export const ProgressContext = createContext<Progress | null>(null);

/** Outside the provider (first-run setup) operations simply run untracked. */
const UNTRACKED: Progress = { ops: [], track: (_title, work) => work(() => {}) };

export function useProgressContext(): Progress {
  return useContext(ProgressContext) ?? UNTRACKED;
}
