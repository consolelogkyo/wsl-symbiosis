import { useEffect, useRef } from "react";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { WebLinksAddon } from "@xterm/addon-web-links";
import { openUrl } from "@tauri-apps/plugin-opener";
import "@xterm/xterm/css/xterm.css";
import {
  closeTerminalSession,
  createTerminalSession,
  ensureTerminalEvents,
  resizeTerminal,
  subscribeTerminal,
  writeTerminal,
} from "../../lib/terminal";
import { errorMessage } from "../../lib/wsl";

export type TabStatus = "connecting" | "running" | "exited" | "error";

export interface PaneApi {
  send: (text: string) => void;
  clear: () => void;
  copySelection: () => Promise<boolean>;
  focus: () => void;
}

interface Props {
  distro: string;
  cwd: string | null;
  /** Bumped to restart the session in place. */
  generation: number;
  visible: boolean;
  onStatus: (patch: { status?: TabStatus; exitCode?: number | null; error?: string; touched?: boolean; title?: string }) => void;
  onRestart: () => void;
  registerApi: (api: PaneApi | null) => void;
}

// Venom palette: void background, white cursor/selection; ANSI colors kept readable but cool-toned.
const THEME = {
  background: "#09090c",
  foreground: "#dcdce2",
  cursor: "#f4f4f6",
  cursorAccent: "#050507",
  selectionBackground: "#f4f4f640",
  black: "#2a2a32",
  red: "#f87171",
  green: "#34d399",
  yellow: "#fbbf24",
  blue: "#818cf8",
  magenta: "#c084fc",
  cyan: "#22d3ee",
  white: "#babac4",
  brightBlack: "#4a4a55",
  brightRed: "#fca5a5",
  brightGreen: "#6ee7b7",
  brightYellow: "#fde68a",
  brightBlue: "#a5b4fc",
  brightMagenta: "#d8b4fe",
  brightCyan: "#67e8f9",
  brightWhite: "#ffffff",
};

export function TerminalPane({ distro, cwd, generation, visible, onStatus, onRestart, registerApi }: Props) {
  const containerRef = useRef<HTMLDivElement>(null);
  const termRef = useRef<Terminal | null>(null);
  const fitRef = useRef<FitAddon | null>(null);
  const sessionRef = useRef<string | null>(null);
  const exitedRef = useRef(false);
  const touchedRef = useRef(false);
  // Latest callbacks without re-running the terminal effect.
  const cb = useRef({ onStatus, onRestart });
  cb.current = { onStatus, onRestart };

  // xterm instance: one per pane, lives as long as the tab.
  useEffect(() => {
    const term = new Terminal({
      fontFamily: '"JetBrains Mono", "Cascadia Mono", "Cascadia Code", Consolas, monospace',
      fontSize: 13,
      lineHeight: 1.15,
      cursorBlink: true,
      scrollback: 10000,
      allowProposedApi: true,
      theme: THEME,
    });
    const fit = new FitAddon();
    term.loadAddon(fit);
    term.loadAddon(new WebLinksAddon((_e, uri) => void openUrl(uri)));
    term.open(containerRef.current!);
    termRef.current = term;
    fitRef.current = fit;

    // Windows Terminal conventions: Ctrl+C copies when there is a selection, Ctrl+V and
    // Ctrl+Shift+C/V copy/paste. Everything else goes to the shell.
    term.attachCustomKeyEventHandler((e) => {
      if (e.type !== "keydown" || !e.ctrlKey) return true;
      const key = e.key.toLowerCase();
      if (key === "c" && (e.shiftKey || term.hasSelection())) {
        void navigator.clipboard.writeText(term.getSelection());
        term.clearSelection();
        return false;
      }
      if (key === "v") {
        void navigator.clipboard.readText().then((text) => text && term.paste(text));
        return false;
      }
      return true;
    });

    const dataSub = term.onData((data) => {
      if (exitedRef.current) {
        if (data === "\r") cb.current.onRestart();
        return;
      }
      const id = sessionRef.current;
      if (!id) return;
      // xterm's automatic replies (cursor position for ConPTY, focus events) start with ESC;
      // only real typing marks the tab as used.
      if (!touchedRef.current && !data.startsWith("\x1b")) {
        touchedRef.current = true;
        cb.current.onStatus({ touched: true });
      }
      writeTerminal(id, data).catch(() => {});
    });
    const resizeSub = term.onResize(({ cols, rows }) => {
      const id = sessionRef.current;
      if (id) resizeTerminal(id, cols, rows).catch(() => {});
    });
    const titleSub = term.onTitleChange((title) => cb.current.onStatus({ title }));

    const observer = new ResizeObserver(() => {
      if (containerRef.current?.offsetParent) fit.fit();
    });
    observer.observe(containerRef.current!);

    registerApi({
      send: (text) => {
        const id = sessionRef.current;
        if (id && !exitedRef.current) writeTerminal(id, text).catch(() => {});
        term.focus();
      },
      clear: () => {
        term.clear();
        term.focus();
      },
      copySelection: async () => {
        const text = term.getSelection();
        if (!text) return false;
        await navigator.clipboard.writeText(text);
        return true;
      },
      focus: () => term.focus(),
    });

    return () => {
      registerApi(null);
      observer.disconnect();
      dataSub.dispose();
      resizeSub.dispose();
      titleSub.dispose();
      term.dispose();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // PTY session: (re)created per generation.
  useEffect(() => {
    const term = termRef.current!;
    let cancelled = false;
    let unsubscribe: (() => void) | null = null;
    exitedRef.current = false;
    touchedRef.current = false;
    sessionRef.current = null;
    if (generation > 0) term.reset();
    cb.current.onStatus({ status: "connecting", error: undefined, exitCode: undefined, touched: false });

    (async () => {
      try {
        if (containerRef.current?.offsetParent) fitRef.current!.fit();
        await ensureTerminalEvents();
        const id = await createTerminalSession(distro, cwd, term.cols, term.rows);
        if (cancelled) {
          closeTerminalSession(id).catch(() => {});
          return;
        }
        sessionRef.current = id;
        unsubscribe = subscribeTerminal(id, {
          output: (data) => term.write(data),
          exit: (code) => {
            exitedRef.current = true;
            sessionRef.current = null;
            cb.current.onStatus({ status: "exited", exitCode: code });
            term.write(
              `\r\n\x1b[90m[processo encerrado${code !== null ? ` · código ${code}` : ""}] ` +
                `Pressione Enter para reiniciar.\x1b[0m\r\n`,
            );
          },
        });
        cb.current.onStatus({ status: "running" });
        term.focus();
      } catch (err) {
        if (cancelled) return;
        exitedRef.current = true;
        cb.current.onStatus({ status: "error", error: errorMessage(err) });
        term.write(`\x1b[31m${errorMessage(err)}\x1b[0m\r\n\x1b[90mPressione Enter para tentar de novo.\x1b[0m\r\n`);
      }
    })();

    return () => {
      cancelled = true;
      unsubscribe?.();
      const id = sessionRef.current;
      sessionRef.current = null;
      if (id) closeTerminalSession(id).catch(() => {});
    };
  }, [distro, cwd, generation]);

  // Hidden panes (display:none) have no size, and rows rendered meanwhile use bogus character
  // metrics (wide letter spacing). When shown: refit, then repaint every row.
  useEffect(() => {
    if (!visible) return;
    const id = requestAnimationFrame(() => {
      const term = termRef.current;
      if (!term) return;
      fitRef.current?.fit();
      term.refresh(0, term.rows - 1);
      term.focus();
    });
    return () => cancelAnimationFrame(id);
  }, [visible]);

  return <div ref={containerRef} className={`size-full ${visible ? "" : "hidden"}`} />;
}
