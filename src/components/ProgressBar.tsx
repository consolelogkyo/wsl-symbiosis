/** The Dashboard's segmented meter as a progress bar. `percent` null = no measure available:
 *  a segment travels along the bar (always moving, so a busy task never looks frozen). */
export function ProgressBar({ percent, className = "h-1.5" }: { percent: number | null; className?: string }) {
  const value = percent === null ? null : Math.max(0, Math.min(100, percent));
  return (
    <div
      role="progressbar"
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={value === null ? undefined : Math.round(value)}
      className={`meter-cells relative overflow-hidden bg-zinc-800 ${className}`}
    >
      {value === null ? (
        <div className="progress-indeterminate absolute inset-y-0 left-0 w-1/3 bg-zinc-100" />
      ) : (
        <div className="h-full bg-zinc-100 transition-[width] duration-500 ease-out" style={{ width: `${value}%` }} />
      )}
    </div>
  );
}
