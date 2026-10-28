// Configurable shortcuts. settings.json keeps only the ones the user changed (id → binding);
// everything else uses the defaults below.
//
// Key bindings:   "Ctrl+Shift+Tab" (modifiers in the order Ctrl, Alt, Shift; letters by
//                 physical key, so ABNT2 and US layouts agree).
// Mouse bindings: comma-separated gestures, "ctrl,middle".
// ""              disabled.

export type ShortcutId =
  | "explorer.newTab"
  | "explorer.closeTab"
  | "explorer.nextTab"
  | "explorer.prevTab"
  | "explorer.openInNewTab"
  | "explorer.closeTabClick";

export interface ShortcutDef {
  id: ShortcutId;
  kind: "key" | "mouse";
  label: string;
  description: string;
  default: string;
}

export const SHORTCUT_GROUPS: { title: string; items: ShortcutDef[] }[] = [
  {
    title: "Explorador · teclado",
    items: [
      { id: "explorer.newTab", kind: "key", label: "Nova aba", description: "Abre uma aba na pasta atual", default: "Ctrl+T" },
      { id: "explorer.closeTab", kind: "key", label: "Fechar aba", description: "Fecha a aba ativa", default: "Ctrl+W" },
      { id: "explorer.nextTab", kind: "key", label: "Próxima aba", description: "Vai para a aba à direita", default: "Ctrl+Tab" },
      { id: "explorer.prevTab", kind: "key", label: "Aba anterior", description: "Vai para a aba à esquerda", default: "Ctrl+Shift+Tab" },
    ],
  },
  {
    title: "Explorador · mouse",
    items: [
      {
        id: "explorer.openInNewTab",
        kind: "mouse",
        label: "Abrir pasta em nova aba",
        description: "Clique numa pasta da lista; a aba abre em segundo plano",
        default: "ctrl,middle",
      },
      { id: "explorer.closeTabClick", kind: "mouse", label: "Fechar aba com clique", description: "Clique sobre a aba", default: "middle" },
    ],
  },
];

export const SHORTCUT_DEFS: ShortcutDef[] = SHORTCUT_GROUPS.flatMap((g) => g.items);

/** Built-in shortcuts, listed for reference (not configurable). */
export const FIXED_SHORTCUTS: { title: string; items: { label: string; binding: string }[] }[] = [
  {
    title: "Editor de arquivos",
    items: [
      { label: "Salvar", binding: "Ctrl+S" },
      { label: "Fechar", binding: "Escape" },
    ],
  },
  {
    title: "Explorador · lista",
    items: [
      { label: "Confirmar nome (renomear / criar)", binding: "Enter" },
      { label: "Cancelar edição de nome", binding: "Escape" },
    ],
  },
  {
    title: "Terminal",
    items: [
      { label: "Copiar seleção", binding: "Ctrl+C" },
      { label: "Copiar", binding: "Ctrl+Shift+C" },
      { label: "Colar", binding: "Ctrl+V" },
    ],
  },
];

export type Shortcuts = Record<ShortcutId, string>;

export function resolveShortcuts(overrides: Record<string, string> | undefined): Shortcuts {
  return Object.fromEntries(SHORTCUT_DEFS.map((d) => [d.id, overrides?.[d.id] ?? d.default])) as Shortcuts;
}

// ---------------------------------------------------------------------------
// Keyboard
// ---------------------------------------------------------------------------

const MODIFIER_KEYS = new Set(["Control", "Alt", "Shift", "Meta", "AltGraph", "CapsLock"]);

/** Canonical binding of a keydown, or null while only modifiers are held. */
export function keyFromEvent(e: KeyboardEvent): string | null {
  if (MODIFIER_KEYS.has(e.key)) return null;
  let key: string;
  if (/^Key[A-Z]$/.test(e.code)) key = e.code.slice(3);
  else if (/^Digit\d$/.test(e.code)) key = e.code.slice(5);
  else if (e.key === " ") key = "Space";
  else if (e.key === "+") key = "Plus";
  else key = e.key.length === 1 ? e.key.toUpperCase() : e.key;
  const mods = [e.ctrlKey && "Ctrl", e.altKey && "Alt", e.shiftKey && "Shift"].filter(Boolean);
  return [...mods, key].join("+");
}

export const matchesKey = (e: KeyboardEvent, binding: string) => binding !== "" && keyFromEvent(e) === binding;

/** Combos the app or the system already uses: recording them is refused. */
const RESERVED = new Set(["Ctrl+C", "Ctrl+V", "Ctrl+X", "Ctrl+A", "Ctrl+Z", "Ctrl+Y", "Ctrl+S", "Alt+F4", "Alt+Tab"]);

/** Why a recorded combo can't be used, or null when it can. */
export function invalidKeyBinding(binding: string): string | null {
  if (RESERVED.has(binding)) return `${formatBinding(binding)} já é usado pelo sistema ou pelo editor`;
  const parts = binding.split("+");
  const key = parts[parts.length - 1];
  const hasModifier = parts.includes("Ctrl") || parts.includes("Alt");
  if (!hasModifier && !/^F\d{1,2}$/.test(key)) return "Use Ctrl ou Alt junto (ou uma tecla F1–F12), para não atrapalhar a digitação";
  return null;
}

const KEY_LABELS: Record<string, string> = {
  Escape: "Esc",
  ArrowLeft: "←",
  ArrowRight: "→",
  ArrowUp: "↑",
  ArrowDown: "↓",
  Plus: "+",
  PageUp: "PgUp",
  PageDown: "PgDn",
  Delete: "Del",
};

/** Keycaps for display: "Ctrl+Shift+Tab" → ["Ctrl", "Shift", "Tab"]. */
export const bindingKeys = (binding: string) => (binding ? binding.split("+").map((k) => KEY_LABELS[k] ?? k) : []);

export const formatBinding = (binding: string) => bindingKeys(binding).join(" + ");

// ---------------------------------------------------------------------------
// Mouse
// ---------------------------------------------------------------------------

export type MouseGesture = "ctrl" | "shift" | "alt" | "middle";

export const MOUSE_GESTURES: { id: MouseGesture; label: string }[] = [
  { id: "ctrl", label: "Ctrl + clique" },
  { id: "shift", label: "Shift + clique" },
  { id: "alt", label: "Alt + clique" },
  { id: "middle", label: "Clique do meio" },
];

export const mouseGestures = (binding: string) =>
  binding.split(",").filter((g): g is MouseGesture => MOUSE_GESTURES.some((m) => m.id === g));

export const toggleGesture = (binding: string, gesture: MouseGesture) => {
  const set = new Set(mouseGestures(binding));
  if (set.has(gesture)) set.delete(gesture);
  else set.add(gesture);
  return MOUSE_GESTURES.map((m) => m.id).filter((id) => set.has(id)).join(",");
};

type ClickLike = { button: number; ctrlKey: boolean; metaKey: boolean; shiftKey: boolean; altKey: boolean };

/** The click (onClick for the left button, onAuxClick for the middle one) is one of the gestures. */
export function matchesClick(e: ClickLike, binding: string): boolean {
  const gestures = mouseGestures(binding);
  if (e.button === 1) return gestures.includes("middle");
  if (e.button !== 0) return false;
  const ctrl = e.ctrlKey || e.metaKey;
  return (
    (gestures.includes("ctrl") && ctrl && !e.shiftKey && !e.altKey) ||
    (gestures.includes("shift") && e.shiftKey && !ctrl && !e.altKey) ||
    (gestures.includes("alt") && e.altKey && !ctrl && !e.shiftKey)
  );
}

export const formatGestures = (binding: string) =>
  mouseGestures(binding)
    .map((g) => MOUSE_GESTURES.find((m) => m.id === g)!.label)
    .join(" ou ");
