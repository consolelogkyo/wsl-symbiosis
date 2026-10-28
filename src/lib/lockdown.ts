// Makes the webview behave like a native app: no browser context menu (Voltar, Atualizar,
// Salvar como, Imprimir, Inspecionar) and no browser accelerators (reload, print, find, view
// source, devtools, history navigation, new window).
//
// Only the browser's default action is cancelled: the app's own handlers still see the keys
// (e.g. Ctrl+S in the file editor, configurable Explorer shortcuts). Keys typed in the terminal
// are left alone: Ctrl+R, Ctrl+U, Ctrl+P, Ctrl+F... belong to the shell, and xterm already
// cancels the browser action for every key it handles.

/** Browser accelerators, in the same format as lib/shortcuts.ts (Ctrl, Alt, Shift + key). */
const BLOCKED_KEYS = new Set([
  "F5", "Ctrl+F5", "Shift+F5", "Ctrl+R", "Ctrl+Shift+R", // reload
  "F12", "Ctrl+Shift+I", "Ctrl+Shift+J", "Ctrl+Shift+C", // devtools
  "Ctrl+U", // view source
  "Ctrl+P", "Ctrl+Shift+P", // print
  "Ctrl+S", // save page as
  "Ctrl+F", "F3", "Shift+F3", "Ctrl+G", "Ctrl+Shift+G", // find
  "Ctrl+N", "Ctrl+Shift+N", "Ctrl+O", // new window, open file
  "Alt+ArrowLeft", "Alt+ArrowRight", "BrowserBack", "BrowserForward", "BrowserRefresh", // history
]);

function combo(e: KeyboardEvent): string {
  const key = /^Key[A-Z]$/.test(e.code) ? e.code.slice(3) : e.key.length === 1 ? e.key.toUpperCase() : e.key;
  return [e.ctrlKey && "Ctrl", e.altKey && "Alt", e.shiftKey && "Shift", key].filter(Boolean).join("+");
}

const inTerminal = (target: EventTarget | null) => target instanceof Element && target.closest(".xterm") !== null;

export function lockDownWebview() {
  window.addEventListener("contextmenu", (e) => e.preventDefault(), true);

  window.addEventListener(
    "keydown",
    (e) => {
      if (!inTerminal(e.target) && BLOCKED_KEYS.has(combo(e))) e.preventDefault();
    },
    true,
  );

  // Mouse back/forward buttons would navigate the webview's history.
  const blockHistoryButtons = (e: MouseEvent) => {
    if (e.button === 3 || e.button === 4) e.preventDefault();
  };
  window.addEventListener("mousedown", blockHistoryButtons, true);
  window.addEventListener("mouseup", blockHistoryButtons, true);
  window.addEventListener("auxclick", blockHistoryButtons, true);
}
