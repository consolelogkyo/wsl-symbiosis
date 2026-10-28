// Symbiosis mark: the terminal prompt (>_) dripping like symbiote goo.
// Same geometry as src-tauri/icons/app-icon.svg (source of the native icons).

export function LogoMark({ className = "size-5" }: { className?: string }) {
  return (
    // Cropped to the glyph (the native icon keeps the 64×64 frame) so it fills small sizes.
    <svg viewBox="12 14 40 40" className={className} aria-hidden>
      <path
        d="M17 18 L33 32 L17 46"
        fill="none"
        stroke="currentColor"
        strokeWidth={7}
        strokeLinejoin="round"
        strokeLinecap="round"
      />
      <g fill="currentColor">
        <path d="M22 41 C22 45 21.2 47 21.2 49.5 A2.6 2.6 0 0 0 26.4 49.5 C26.4 47 25.6 45 25.6 40Z" />
        <path d="M28.2 37 C28.2 39.5 27.7 40.5 27.7 42 A1.8 1.8 0 0 0 31.3 42 C31.3 40.5 30.8 39.5 30.8 36Z" />
        <rect x="36" y="42" width="14" height="5.5" rx="1.5" />
      </g>
    </svg>
  );
}
