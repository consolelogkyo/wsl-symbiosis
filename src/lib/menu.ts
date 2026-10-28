// Floating menus rendered in a portal (scripts, branches). The webview can't draw outside
// the window: open on the side with more room (below by default), cap the height there and
// let the menu scroll inside.

export interface MenuPlacement {
  left: number;
  top?: number;
  bottom?: number;
  maxHeight: number;
}

const MENU_GAP = 6;
/** Keeps the menu off the window edges. */
const MENU_MARGIN = 12;

/** `align`: which edge of the anchor the menu lines up with. */
export function placeMenu(anchor: DOMRect, width: number, align: "left" | "right" = "right"): MenuPlacement {
  const below = window.innerHeight - anchor.bottom - MENU_GAP - MENU_MARGIN;
  const above = anchor.top - MENU_GAP - MENU_MARGIN;
  const wanted = align === "right" ? anchor.right - width : anchor.left;
  const left = Math.max(MENU_MARGIN, Math.min(wanted, window.innerWidth - width - MENU_MARGIN));
  // Below fits ~4 items: good enough, don't flip.
  if (below >= 220 || below >= above) return { left, top: anchor.bottom + MENU_GAP, maxHeight: below };
  return { left, bottom: window.innerHeight - anchor.top + MENU_GAP, maxHeight: above };
}

/** Closes a menu on outside pointer, scroll (except inside the panel itself) and resize. */
export function closeOnOutside(panel: { current: HTMLElement | null }, close: () => void): () => void {
  const handler = (e: Event) => {
    if (!panel.current?.contains(e.target as Node)) close();
  };
  window.addEventListener("pointerdown", handler);
  window.addEventListener("scroll", handler, true);
  window.addEventListener("resize", handler);
  return () => {
    window.removeEventListener("pointerdown", handler);
    window.removeEventListener("scroll", handler, true);
    window.removeEventListener("resize", handler);
  };
}
