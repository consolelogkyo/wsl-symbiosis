import { createContext, useCallback, useContext, useEffect, useRef, useState } from "react";
import { getVersion } from "@tauri-apps/api/app";
import type { Update } from "@tauri-apps/plugin-updater";
import {
  FAKE_UPDATE,
  FAKE_UPDATE_INFO,
  type UpdaterState,
  checkForUpdate,
  explainUpdateError,
  prepareForUpdate,
  relaunchApp,
  toUpdateInfo,
} from "../lib/updater";

/** Silent check a few seconds after start-up (does not compete with the first WSL scan). */
const FIRST_CHECK_DELAY_MS = 6_000;
/** The app is often left open for days. */
const RECHECK_INTERVAL_MS = 6 * 60 * 60 * 1000;
/** Download progress comes in small chunks: re-render at most this often. */
const PROGRESS_THROTTLE_MS = 120;

const sleep = (ms: number) => new Promise((resolve) => window.setTimeout(resolve, ms));

export interface Updater {
  state: UpdaterState;
  currentVersion: string | null;
  /** Epoch ms of the last check that reached GitHub. */
  lastChecked: number | null;
  dialogOpen: boolean;
  openDialog: () => void;
  closeDialog: () => void;
  /** Manual check (Settings): errors are shown. */
  checkNow: () => void;
  /** Download, close terminals/projects, install and relaunch. */
  install: () => void;
}

export const UpdaterContext = createContext<Updater | null>(null);

export function useUpdaterContext(): Updater {
  const ctx = useContext(UpdaterContext);
  if (!ctx) throw new Error("UpdaterContext ausente");
  return ctx;
}

/** App-wide updater state; lives in App so the Titlebar and Settings share it. */
export function useUpdater(): Updater {
  const [state, setState] = useState<UpdaterState>({ status: "idle" });
  const [currentVersion, setCurrentVersion] = useState<string | null>(null);
  const [lastChecked, setLastChecked] = useState<number | null>(null);
  const [dialogOpen, setDialogOpen] = useState(false);
  const updateRef = useRef<Update | null>(null);
  // A check or a download is in flight (also blocks the periodic check while downloading).
  const busy = useRef(false);
  const stateRef = useRef(state);
  stateRef.current = state;

  const currentVersionRef = useRef(currentVersion);
  currentVersionRef.current = currentVersion;

  useEffect(() => {
    getVersion().then(setCurrentVersion).catch(() => setCurrentVersion(null));
  }, []);

  const runCheck = useCallback(async (manual: boolean) => {
    if (busy.current) return;
    // Once an update was found, the silent check has nothing to add.
    const current = stateRef.current.status;
    if (!manual && (current === "available" || current === "ready")) return;
    busy.current = true;
    setState({ status: "checking", manual });
    if (FAKE_UPDATE) {
      await sleep(900);
      setLastChecked(Date.now());
      setState({ status: "available", update: { ...FAKE_UPDATE_INFO, currentVersion: currentVersionRef.current ?? "?" } });
      busy.current = false;
      return;
    }
    try {
      const update = await checkForUpdate();
      setLastChecked(Date.now());
      void updateRef.current?.close().catch(() => {});
      updateRef.current = update;
      setState(update ? { status: "available", update: toUpdateInfo(update) } : { status: "up-to-date" });
    } catch (err) {
      // Offline or no release published yet: the silent check never bothers the user.
      setState(manual ? { status: "error", message: explainUpdateError(err) } : { status: "idle" });
    } finally {
      busy.current = false;
    }
  }, []);

  useEffect(() => {
    const first = window.setTimeout(() => void runCheck(false), FIRST_CHECK_DELAY_MS);
    const every = window.setInterval(() => void runCheck(false), RECHECK_INTERVAL_MS);
    return () => {
      window.clearTimeout(first);
      window.clearInterval(every);
    };
  }, [runCheck]);

  const install = useCallback(async () => {
    if (FAKE_UPDATE) {
      if (busy.current || stateRef.current.status !== "available") return;
      busy.current = true;
      const info = stateRef.current.update;
      const total = 2_757_859;
      for (let downloaded = 0; downloaded < total; downloaded += 61_000) {
        setState({ status: "downloading", update: info, downloaded, total });
        await sleep(PROGRESS_THROTTLE_MS);
      }
      setState({ status: "ready", update: info });
      await sleep(2500);
      // Nothing was installed: back to the start so the flow can be reviewed again.
      setState({ status: "available", update: info });
      busy.current = false;
      return;
    }
    const update = updateRef.current;
    if (!update || busy.current) return;
    busy.current = true;
    const info = toUpdateInfo(update);
    let downloaded = 0;
    let total: number | null = null;
    let lastPaint = 0;
    setState({ status: "downloading", update: info, downloaded, total });
    try {
      await update.download((event) => {
        if (event.event === "Started") total = event.data.contentLength ?? null;
        else if (event.event === "Progress") downloaded += event.data.chunkLength;
        const now = performance.now();
        if (event.event !== "Progress" || now - lastPaint >= PROGRESS_THROTTLE_MS) {
          lastPaint = now;
          setState({ status: "downloading", update: info, downloaded, total });
        }
      });
      setState({ status: "ready", update: info });
      // Best effort: a failure here must not block the update itself.
      await prepareForUpdate().catch(() => {});
      // Windows: starts the installer (passive) and exits; the installer reopens the app.
      await update.install();
      await relaunchApp();
    } catch (err) {
      setState({ status: "error", message: explainUpdateError(err), update: info });
    } finally {
      busy.current = false;
    }
  }, []);

  return {
    state,
    currentVersion,
    lastChecked,
    dialogOpen,
    openDialog: useCallback(() => setDialogOpen(true), []),
    closeDialog: useCallback(() => setDialogOpen(false), []),
    checkNow: useCallback(() => void runCheck(true), [runCheck]),
    install: useCallback(() => void install(), [install]),
  };
}
