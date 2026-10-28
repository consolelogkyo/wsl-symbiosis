import type { ReactNode } from "react";
import type { LucideIcon } from "lucide-react";

export interface Segment {
  label: string;
  value: number;
  /** Tailwind background classes for the bar slice and its legend swatch. */
  className: string;
}

interface Props {
  icon: LucideIcon;
  title: string;
  /** Big headline number, e.g. "3.4 GB". */
  value: string;
  valueClassName?: string;
  /** Tiny semantic status dot next to the value (LEVEL_DOT). */
  dot?: string;
  caption?: ReactNode;
  segments: Segment[];
  total: number;
  /** Shows the segment legend under the bar. */
  legend?: boolean;
  footer?: ReactNode;
  action?: ReactNode;
}

export function MeterCard({
  icon: Icon,
  title,
  value,
  valueClassName = "text-zinc-100",
  dot,
  caption,
  segments,
  total,
  legend = true,
  footer,
  action,
}: Props) {
  return (
    <div className="flex flex-col rounded-lg border border-zinc-800 bg-zinc-900 p-4">
      <div className="flex items-center gap-2">
        <Icon className="size-3.5 text-zinc-500" />
        <span className="min-w-0 flex-1 truncate text-[11px] font-medium tracking-wider text-zinc-500 uppercase">
          {title}
        </span>
        {action}
      </div>

      {/* Value, status dot and caption share one vertical center line. */}
      <div className="mt-3 flex h-7 min-w-0 items-center gap-2">
        <span className={`shrink-0 font-mono text-2xl leading-none font-medium tracking-tight whitespace-nowrap tabular-nums ${valueClassName}`}>
          {value}
        </span>
        {dot && <span className={`size-1.5 shrink-0 rounded-full ${dot}`} />}
        {caption && <span className="min-w-0 truncate text-xs leading-none text-zinc-500">{caption}</span>}
      </div>

      <div
        className="meter-cells mt-3 flex h-2 overflow-hidden bg-zinc-800"
        role="meter"
        aria-label={title}
        aria-valuemin={0}
        aria-valuemax={total}
        aria-valuenow={segments.reduce((sum, s) => sum + s.value, 0)}
      >
        {segments.map((s) => (
          <div
            key={s.label}
            className={`h-full transition-[width] duration-500 ease-out ${s.className}`}
            style={{ width: `${total > 0 ? (s.value / total) * 100 : 0}%` }}
            title={s.label}
          />
        ))}
      </div>

      {legend && (
        <div className="mt-2.5 flex flex-wrap gap-x-3 gap-y-1 text-[11px] text-zinc-500">
          {segments.map((s) => (
            <span key={s.label} className="flex items-center gap-1.5">
              <span className={`size-1.5 ${s.className}`} />
              {s.label}
            </span>
          ))}
        </div>
      )}

      {footer && (
        <div className="mt-auto pt-3">
          <div className="border-t border-zinc-800 pt-3 text-xs text-zinc-400">{footer}</div>
        </div>
      )}
    </div>
  );
}
