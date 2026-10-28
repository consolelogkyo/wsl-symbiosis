import type { DistroState } from "../lib/wsl";

export function StatusDot({ state }: { state: DistroState }) {
  const running = state === "Running";
  const transitional = state === "Installing" || state === "Converting" || state === "Uninstalling";
  return (
    <span
      className={`inline-flex size-1.5 shrink-0 rounded-full ${
        running ? "bg-emerald-400" : transitional ? "bg-amber-400" : "bg-zinc-600"
      }`}
    />
  );
}
