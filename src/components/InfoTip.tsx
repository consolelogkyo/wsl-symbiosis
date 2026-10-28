import { useLayoutEffect, useRef, useState, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { Info } from "lucide-react";

const GAP = 8;
const MARGIN = 8;

/**
 * Small "i" that reveals a note on hover/focus, for context that doesn't need to be always on screen.
 * The note is portaled to <body> with fixed coordinates, so dialogs (overflow-hidden) and scroll
 * areas never clip it; it opens below, or above when there's no room, and stays inside the window.
 */
export function InfoTip({ children, align = "left" }: { children: ReactNode; align?: "left" | "right" }) {
  const anchor = useRef<HTMLButtonElement>(null);
  const tip = useRef<HTMLSpanElement>(null);
  const [open, setOpen] = useState(false);
  const [pos, setPos] = useState<{ left: number; top: number } | null>(null);

  useLayoutEffect(() => {
    if (!open || !anchor.current || !tip.current) return;
    const a = anchor.current.getBoundingClientRect();
    const t = tip.current.getBoundingClientRect();
    const preferred = align === "left" ? a.left - 4 : a.right + 4 - t.width;
    const left = Math.min(Math.max(MARGIN, preferred), window.innerWidth - t.width - MARGIN);
    const below = a.bottom + GAP;
    const fitsBelow = below + t.height <= window.innerHeight - MARGIN;
    const top = fitsBelow || a.top - GAP - t.height < MARGIN ? below : a.top - GAP - t.height;
    setPos({ left, top });
  }, [open, align]);

  // Scrolling moves the anchor away from a fixed tooltip: just close it.
  useLayoutEffect(() => {
    if (!open) return;
    const close = () => setOpen(false);
    window.addEventListener("scroll", close, true);
    window.addEventListener("resize", close);
    return () => {
      window.removeEventListener("scroll", close, true);
      window.removeEventListener("resize", close);
    };
  }, [open]);

  const show = () => {
    setPos(null);
    setOpen(true);
  };
  const hide = () => setOpen(false);

  return (
    <span className="relative inline-flex align-middle">
      <button
        ref={anchor}
        type="button"
        aria-label="Mais informações"
        onMouseEnter={show}
        onMouseLeave={hide}
        onFocus={show}
        onBlur={hide}
        className="rounded-full text-zinc-600 transition-colors hover:text-zinc-200 focus-visible:text-zinc-200"
      >
        <Info className="size-3.5" />
      </button>
      {open &&
        createPortal(
          <span
            ref={tip}
            role="tooltip"
            style={{ left: pos?.left ?? 0, top: pos?.top ?? 0, visibility: pos ? "visible" : "hidden" }}
            className="pointer-events-none fixed z-[100] block w-72 rounded-md border border-zinc-700 bg-zinc-900 px-3 py-2 text-[11px] leading-relaxed font-normal tracking-normal text-zinc-300 normal-case shadow-xl shadow-black/60"
          >
            {children}
          </span>,
          document.body,
        )}
    </span>
  );
}
