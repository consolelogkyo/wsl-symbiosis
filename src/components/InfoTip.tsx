import type { ReactNode } from "react";
import { Info } from "lucide-react";

/** Small "i" that reveals a note on hover/focus, for context that doesn't need to be always on screen. */
export function InfoTip({ children, align = "left" }: { children: ReactNode; align?: "left" | "right" }) {
  return (
    <span className="group/tip relative inline-flex">
      <button
        type="button"
        aria-label="Mais informações"
        className="rounded-full text-zinc-600 transition-colors hover:text-zinc-200 focus-visible:text-zinc-200"
      >
        <Info className="size-3.5" />
      </button>
      <span
        role="tooltip"
        className={`pointer-events-none absolute top-full z-40 mt-2 w-72 rounded-md border border-zinc-700 bg-zinc-900 px-3 py-2 text-[11px] leading-relaxed font-normal tracking-normal text-zinc-300 normal-case opacity-0 shadow-xl shadow-black/60 transition-opacity duration-150 group-hover/tip:opacity-100 group-focus-within/tip:opacity-100 ${
          align === "left" ? "left-0" : "right-0"
        }`}
      >
        {children}
      </span>
    </span>
  );
}
