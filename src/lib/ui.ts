// Shared Venom class recipes, so buttons, cards and labels look the same on every screen.

/** Primary action: white pill, black text. */
export const BTN_PRIMARY =
  "inline-flex items-center gap-1.5 rounded-full bg-zinc-100 px-3 py-1 text-xs font-semibold text-void transition-colors hover:bg-white disabled:cursor-not-allowed disabled:bg-zinc-800 disabled:text-zinc-500";

/** Secondary action: hairline pill. */
export const BTN_GHOST =
  "inline-flex items-center gap-1.5 rounded-full border border-zinc-800 px-3 py-1 text-xs font-medium text-zinc-300 transition-colors hover:border-zinc-600 hover:text-zinc-100 disabled:cursor-not-allowed disabled:opacity-40 disabled:hover:border-zinc-800 disabled:hover:text-zinc-300";

/** Destructive action, revealed on hover: rose only on intent. */
export const BTN_DANGER =
  "inline-flex items-center gap-1.5 rounded-full border border-zinc-800 px-3 py-1 text-xs font-medium text-zinc-300 transition-colors hover:border-rose-500/50 hover:text-rose-300 disabled:cursor-not-allowed disabled:opacity-40";

/** Confirmed destructive action (second click). */
export const BTN_DANGER_SOLID =
  "inline-flex items-center gap-1 rounded-full border border-rose-500/40 px-2 py-0.5 text-[11px] font-semibold text-rose-300 transition-colors hover:bg-rose-500/15";

/** Square icon button. */
export const BTN_ICON =
  "rounded-md p-1.5 text-zinc-500 transition-colors hover:bg-zinc-800 hover:text-zinc-100 disabled:cursor-not-allowed disabled:opacity-30 disabled:hover:bg-transparent";

export const CARD = "rounded-lg border border-zinc-800 bg-zinc-900";

/** Uppercase technical label. */
export const LABEL = "text-[11px] font-medium tracking-wider text-zinc-500 uppercase";

/** Text field shell (wrap an <input> in a <label> with this). */
export const FIELD =
  "flex items-center gap-2 rounded-md border border-zinc-800 bg-zinc-950 px-2 py-1 transition-colors focus-within:border-zinc-500";

export const INPUT = "w-full bg-transparent text-xs text-zinc-100 placeholder:text-zinc-600 focus:outline-none";

/** Error / warning strips under toolbars. */
export const BANNER_ERROR = "flex items-start gap-2.5 border-b border-rose-500/20 bg-rose-500/[0.04] px-4 py-2 text-xs text-rose-300";
