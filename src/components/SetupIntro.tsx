import { useEffect, useState } from "react";

/** How long the intro plays before dissolving into the setup (ms). */
const HOLD_MS = 6400;
const EXIT_MS = 650;

/**
 * First-run intro: the Symbiosis mark assembles itself (prompt stroke, drips, cursor), then the
 * wordmark settles in. Click or any key skips it; "reduce motion" skips it entirely.
 * Same geometry as LogoMark / src-tauri/icons/app-icon.svg.
 */
export function SetupIntro({ onDone }: { onDone: () => void }) {
  const [leaving, setLeaving] = useState(false);

  useEffect(() => {
    if (window.matchMedia("(prefers-reduced-motion: reduce)").matches) {
      onDone();
      return;
    }
    const hold = window.setTimeout(() => setLeaving(true), HOLD_MS);
    const skip = () => setLeaving(true);
    window.addEventListener("keydown", skip);
    return () => {
      window.clearTimeout(hold);
      window.removeEventListener("keydown", skip);
    };
  }, [onDone]);

  useEffect(() => {
    if (!leaving) return;
    const t = window.setTimeout(onDone, EXIT_MS);
    return () => window.clearTimeout(t);
  }, [leaving, onDone]);

  return (
    <div
      role="presentation"
      onClick={() => setLeaving(true)}
      className={`fixed inset-0 z-50 flex cursor-default flex-col items-center justify-center bg-void ${leaving ? "intro-exit" : ""}`}
    >
      <svg viewBox="12 14 40 40" className="size-24 text-zinc-100" aria-hidden>
        <path
          className="intro-chevron"
          pathLength={1}
          d="M17 18 L33 32 L17 46"
          fill="none"
          stroke="currentColor"
          strokeWidth={7}
          strokeLinejoin="round"
          strokeLinecap="round"
        />
        <g fill="currentColor">
          <path
            className="intro-drip"
            style={{ animationDelay: "1500ms" }}
            d="M22 41 C22 45 21.2 47 21.2 49.5 A2.6 2.6 0 0 0 26.4 49.5 C26.4 47 25.6 45 25.6 40Z"
          />
          <path
            className="intro-drip"
            style={{ animationDelay: "1800ms" }}
            d="M28.2 37 C28.2 39.5 27.7 40.5 27.7 42 A1.8 1.8 0 0 0 31.3 42 C31.3 40.5 30.8 39.5 30.8 36Z"
          />
          <rect className="intro-cursor" x="36" y="42" width="14" height="5.5" rx="1.5" />
        </g>
      </svg>

      <div className="intro-wordmark mt-8 pl-[0.32em] text-3xl font-extrabold text-zinc-100">SYMBIOSIS</div>
      <div className="intro-rule mt-5 h-px w-40 bg-zinc-700" />
      <div className="intro-fade-late mt-4 font-mono text-[11px] tracking-[0.3em] text-zinc-500 uppercase">
        Windows <span className="text-zinc-600">⇄</span> WSL2
      </div>
    </div>
  );
}
