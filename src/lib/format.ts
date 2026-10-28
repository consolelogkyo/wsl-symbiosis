const UNITS = ["B", "KB", "MB", "GB", "TB"];

export function formatBytes(bytes: number, digits = 1): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return "0 B";
  const i = Math.min(Math.floor(Math.log(bytes) / Math.log(1024)), UNITS.length - 1);
  const value = bytes / 1024 ** i;
  return `${value.toFixed(i >= 3 ? digits : 0)} ${UNITS[i]}`;
}

export function percent(part: number, total: number): number {
  return total > 0 ? Math.min(100, Math.max(0, (part / total) * 100)) : 0;
}

export type Level = "ok" | "warn" | "critical";

export function level(pct: number): Level {
  return pct >= 85 ? "critical" : pct >= 60 ? "warn" : "ok";
}

/** Bar fill per pressure level: Venom white → chrome, rose only when critical. */
export const LEVEL_BAR: Record<Level, string> = {
  ok: "bg-zinc-100",
  warn: "bg-zinc-300",
  critical: "bg-rose-500",
};

export const LEVEL_TEXT: Record<Level, string> = {
  ok: "text-zinc-100",
  warn: "text-zinc-100",
  critical: "text-rose-400",
};

/** The only place semantic colors show up for metrics: a tiny dot next to the value. */
export const LEVEL_DOT: Record<Level, string> = {
  ok: "bg-emerald-400",
  warn: "bg-amber-400",
  critical: "bg-rose-500",
};
