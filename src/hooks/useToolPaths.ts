import { useEffect, useState } from "react";
import { open as pickFile } from "@tauri-apps/plugin-dialog";
import type { ToolInfo } from "../lib/explorer";
import { checkToolPath } from "../lib/settings";
import { errorMessage } from "../lib/wsl";

export type PathCheck = { state: "idle" | "checking" | "ok" | "error"; message?: string };

/** Custom IDE/tool paths with live validation (Settings and the setup wizard). */
export function useToolPaths(initial: Record<string, string>, onError: (message: string) => void) {
  const [paths, setPaths] = useState<Record<string, string>>(initial);
  const [checks, setChecks] = useState<Record<string, PathCheck>>({});

  // Live validation of the custom paths (debounced).
  useEffect(() => {
    const timers = Object.entries(paths).map(([id, value]) => {
      if (!value.trim()) {
        setChecks((c) => ({ ...c, [id]: { state: "idle" } }));
        return 0;
      }
      setChecks((c) => ({ ...c, [id]: { state: "checking" } }));
      return window.setTimeout(() => {
        checkToolPath(value)
          .then((normalized) => setChecks((c) => ({ ...c, [id]: { state: "ok", message: normalized ?? undefined } })))
          .catch((err) => setChecks((c) => ({ ...c, [id]: { state: "error", message: errorMessage(err) } })));
      }, 300);
    });
    return () => timers.forEach((t) => window.clearTimeout(t));
  }, [paths]);

  const setPath = (id: string, value: string) => setPaths((p) => ({ ...p, [id]: value }));

  const browse = async (tool: ToolInfo) => {
    try {
      const picked = await pickFile({
        title: `Executável do ${tool.label}`,
        multiple: false,
        directory: false,
        defaultPath: paths[tool.id] || tool.autoPath || undefined,
        filters: [{ name: "Executável", extensions: ["exe", "cmd", "bat"] }],
      });
      if (typeof picked === "string") setPath(tool.id, picked);
    } catch (err) {
      onError(errorMessage(err));
    }
  };

  return {
    paths,
    setPath,
    checks,
    browse,
    /** A path is invalid or still being checked: saving must wait. */
    invalid: Object.values(checks).some((c) => c.state === "error" || c.state === "checking"),
    /** What goes to settings.json (empty = auto-detect). */
    toSave: () => Object.fromEntries(Object.entries(paths).filter(([, v]) => v.trim())),
  };
}
