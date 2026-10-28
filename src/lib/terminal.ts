import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

// Mirrors of src-tauri/src/terminal.rs.

interface OutputPayload {
  sessionId: string;
  data: string;
}

interface ExitPayload {
  sessionId: string;
  code: number | null;
}

export const createTerminalSession = (distro: string, cwd: string | null, cols: number, rows: number) =>
  invoke<string>("create_terminal_session", { distro, cwd, cols, rows });
export const writeTerminal = (sessionId: string, data: string) => invoke<void>("write_terminal", { sessionId, data });
export const resizeTerminal = (sessionId: string, cols: number, rows: number) =>
  invoke<void>("resize_terminal", { sessionId, cols, rows });
export const closeTerminalSession = (sessionId: string) => invoke<void>("close_terminal_session", { sessionId });

// --- Event routing ---------------------------------------------------------
// One global listener per event, dispatching by session id. Output can arrive before the
// pane knows its session id (the reader starts inside create_terminal_session), so events
// for unknown sessions are buffered and replayed on subscribe.

interface Handlers {
  output: (data: string) => void;
  exit: (code: number | null) => void;
}

const handlers = new Map<string, Handlers>();
const pending = new Map<string, { output: string[]; exit?: { code: number | null } }>();
const MAX_PENDING_CHUNKS = 2000;
let listening: Promise<void> | null = null;

function pendingFor(id: string) {
  let entry = pending.get(id);
  if (!entry) pending.set(id, (entry = { output: [] }));
  return entry;
}

/** Must be awaited before creating a session, so no early output is missed. */
export function ensureTerminalEvents(): Promise<void> {
  listening ??= Promise.all([
    listen<OutputPayload>("terminal-output", ({ payload }) => {
      const h = handlers.get(payload.sessionId);
      if (h) h.output(payload.data);
      else {
        const entry = pendingFor(payload.sessionId);
        if (entry.output.length < MAX_PENDING_CHUNKS) entry.output.push(payload.data);
      }
    }),
    listen<ExitPayload>("terminal-exit", ({ payload }) => {
      const h = handlers.get(payload.sessionId);
      if (h) h.exit(payload.code);
      else pendingFor(payload.sessionId).exit = { code: payload.code };
    }),
  ]).then(() => undefined);
  return listening;
}

export function subscribeTerminal(sessionId: string, h: Handlers): () => void {
  handlers.set(sessionId, h);
  const queued = pending.get(sessionId);
  if (queued) {
    pending.delete(sessionId);
    for (const chunk of queued.output) h.output(chunk);
    if (queued.exit) h.exit(queued.exit.code);
  }
  return () => {
    handlers.delete(sessionId);
    pending.delete(sessionId);
  };
}

// --- Quick snippets -----------------------------------------------------------

export interface Snippet {
  id: string;
  label: string;
  command: string;
}

export const SNIPPETS: Snippet[] = [
  { id: "apt", label: "Atualizar pacotes", command: "sudo apt update && sudo apt upgrade -y" },
  { id: "git", label: "Status Git", command: "git status -sb" },
  { id: "top", label: "Processos", command: "command -v htop >/dev/null 2>&1 && htop || top" },
  { id: "df", label: "Espaço em disco", command: "df -h" },
  { id: "ports", label: "Portas abertas", command: "ss -tlnp" },
  { id: "reload", label: "Recarregar shell", command: "exec $SHELL -l" },
];
