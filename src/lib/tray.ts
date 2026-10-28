import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

// Mirrors of src-tauri/src/tray.rs (background mode and the close button).

export interface TrayNotice {
  tone: "success" | "error" | "info";
  title: string;
  detail: string | null;
}

/** Hides the window to the system tray: terminals and dev servers keep running. */
export const hideToTray = () => invoke<void>("hide_to_tray");
/** Stops projects, closes terminals and exits. */
export const quitApplication = () => invoke<void>("quit_application");
/** The selected distro, so the tray menu matches the window. */
export const setTrayContext = (distro: string | null, running: boolean) =>
  invoke<void>("set_tray_context", { distro, running });

/** ✕ / Alt+F4 with `closeBehavior: "ask"`: the backend kept the window open and waits for a choice. */
export const onCloseRequested = (cb: () => void) => listen("close-requested", () => cb());
/** The window was hidden to (or restored from) the tray. */
export const onAppVisibility = (cb: (visible: boolean) => void) =>
  listen<boolean>("app-visibility", (e) => cb(e.payload));
/** Result of an action taken from the tray menu (drop caches, shutdown...). */
export const onTrayNotice = (cb: (notice: TrayNotice) => void) =>
  listen<TrayNotice>("tray-notice", (e) => cb(e.payload));
/** WSL was shut down from somewhere other than the Titlebar (tray, .wslconfig, VHDX). */
export const onWslChanged = (cb: () => void) => listen("wsl-changed", () => cb());

/** Subscribes inside an effect: tolerates the browser preview (no Tauri) and StrictMode. */
export function subscribe(add: () => Promise<() => void>): () => void {
  let unlisten: (() => void) | undefined;
  let alive = true;
  try {
    add()
      .then((fn) => (alive ? (unlisten = fn) : fn()))
      .catch(() => {});
  } catch {
    /* not running inside Tauri */
  }
  return () => {
    alive = false;
    unlisten?.();
  };
}
