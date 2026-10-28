import { readableBrandColor, type SimpleIcon } from "../lib/brands";

/** Official logo (bundled SVG path) in its brand color. */
export function BrandIcon({ icon, className = "size-4", color }: { icon: SimpleIcon; className?: string; color?: string }) {
  return (
    <svg viewBox="0 0 24 24" role="img" aria-label={icon.title} className={`shrink-0 ${className}`} fill={color ?? readableBrandColor(icon.hex)}>
      <path d={icon.path} />
    </svg>
  );
}
