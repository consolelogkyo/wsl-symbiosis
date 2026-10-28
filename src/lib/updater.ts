// Auto-update via GitHub Releases (tauri-plugin-updater + tauri-plugin-process).
// The endpoint and the public key live in src-tauri/tauri.conf.json (plugins.updater);
// `npm run release:build` produces the signed installer and the latest.json for a release.
import { invoke } from "@tauri-apps/api/core";
import { check, type Update } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { errorMessage } from "./wsl";

export interface UpdateInfo {
  version: string;
  currentVersion: string;
  /** RFC 3339 (`pub_date` of latest.json), when present. */
  date: string | null;
  /** Release notes (`notes` of latest.json). */
  notes: string;
}

export type UpdaterState =
  | { status: "idle" }
  | { status: "checking"; manual: boolean }
  | { status: "available"; update: UpdateInfo }
  | { status: "downloading"; update: UpdateInfo; downloaded: number; total: number | null }
  | { status: "ready"; update: UpdateInfo }
  | { status: "up-to-date" }
  /** Only for actions the user started (a manual check or the download). */
  | { status: "error"; message: string; update?: UpdateInfo };

const CHECK_TIMEOUT_MS = 20_000;

/**
 * Visual QA in `npm run tauri dev` without a published release: VITE_FAKE_UPDATE=1 makes the
 * check find a fake version and the download progress for a few seconds (nothing is installed).
 * Never active in production builds.
 */
export const FAKE_UPDATE = import.meta.env.DEV && import.meta.env.VITE_FAKE_UPDATE === "1";

export const FAKE_UPDATE_INFO: UpdateInfo = {
  version: "9.9.9",
  currentVersion: "0.0.0",
  date: new Date().toISOString(),
  notes:
    "### Simulação (VITE_FAKE_UPDATE=1)\n- Esta versão não existe: serve só para revisar o visual\n- O download é falso e nada é instalado\n\n### Exemplo de notas\n- Item com **negrito** e `código`\n  - Subitem recuado",
};

/** Asks GitHub for latest.json; null when the installed version is the newest. */
export function checkForUpdate(): Promise<Update | null> {
  return check({ timeout: CHECK_TIMEOUT_MS });
}

export function toUpdateInfo(update: Update): UpdateInfo {
  const raw = update.rawJson as { pub_date?: unknown };
  return {
    version: update.version,
    currentVersion: update.currentVersion,
    date: typeof raw.pub_date === "string" ? raw.pub_date : null,
    notes: update.body?.trim() ?? "",
  };
}

/** Closes terminals and stops running projects: the Windows installer exits the app abruptly. */
export function prepareForUpdate(): Promise<void> {
  return invoke("prepare_for_update");
}

export function relaunchApp(): Promise<void> {
  return relaunch();
}

/** Plugin errors (English, technical) → one line in pt-BR. */
export function explainUpdateError(err: unknown): string {
  const raw = errorMessage(err);
  if (/valid release JSON|404|Not Found/i.test(raw)) {
    return "Nenhuma versão publicada no GitHub ainda (ou a release não tem o latest.json).";
  }
  if (/signature|minisign|verify/i.test(raw)) {
    return "A assinatura do pacote não confere. O download foi descartado por segurança.";
  }
  if (/sending request|dns|connect|timed out|timeout|network|offline/i.test(raw)) {
    return "Sem conexão com o GitHub. Verifique a internet e tente de novo.";
  }
  return raw;
}

export function percentOf(downloaded: number, total: number | null): number | null {
  return total && total > 0 ? Math.min(100, (downloaded / total) * 100) : null;
}
