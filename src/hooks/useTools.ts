import { useEffect, useState } from "react";
import { ToolInfo, detectTools, getToolIcons } from "../lib/explorer";

// Installed tools rarely change: detect once and share the result. Saving custom paths in
// the settings calls reloadTools() so every screen picks up the new launchers.
// Official icons arrive a bit later (one PowerShell run) and are merged in when ready.
let current: ToolInfo[] = [];
let loading: Promise<ToolInfo[]> | null = null;
const listeners = new Set<(tools: ToolInfo[]) => void>();

function publish(tools: ToolInfo[]) {
  current = tools;
  listeners.forEach((l) => l(tools));
}

function load(): Promise<ToolInfo[]> {
  loading ??= detectTools()
    .then((tools) => {
      publish(tools);
      getToolIcons()
        .then((icons) => publish(tools.map((t) => ({ ...t, icon: icons[t.id] }))))
        .catch(() => {});
      return tools;
    })
    .catch(() => {
      loading = null;
      return [];
    });
  return loading;
}

export async function reloadTools(): Promise<ToolInfo[]> {
  loading = null;
  return load();
}

export function useTools(): ToolInfo[] {
  const [tools, setTools] = useState<ToolInfo[]>(current);
  useEffect(() => {
    listeners.add(setTools);
    setTools(current);
    load();
    return () => {
      listeners.delete(setTools);
    };
  }, []);
  return tools;
}
