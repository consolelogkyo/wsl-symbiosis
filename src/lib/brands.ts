// Official tool logos, bundled at build time from simple-icons (CC0): no network at runtime.
// Only the icons imported here end up in the bundle.
import {
  siBun,
  siDeno,
  siDocker,
  siGit,
  siGnu,
  siGo,
  siNodedotjs,
  siNpm,
  siOpenjdk,
  siPnpm,
  siPypi,
  siPython,
  siRust,
  siYarn,
  type SimpleIcon,
} from "simple-icons";

export type { SimpleIcon };

/** packages.rs tool id → logo. Version managers borrow the logo of what they manage. */
export const TOOL_BRANDS: Record<string, SimpleIcon> = {
  node: siNodedotjs,
  npm: siNpm,
  yarn: siYarn,
  pnpm: siPnpm,
  bun: siBun,
  deno: siDeno,
  python: siPython,
  pip: siPypi,
  rustc: siRust,
  cargo: siRust,
  go: siGo,
  java: siOpenjdk,
  git: siGit,
  docker: siDocker,
  gcc: siGnu,
  nvm: siNodedotjs,
  fnm: siNodedotjs,
  volta: siNodedotjs,
  pyenv: siPython,
  rustup: siRust,
};

export const STACK_BRANDS = {
  node: siNodedotjs,
  rust: siRust,
  python: siPython,
  go: siGo,
  docker: siDocker,
  git: siGit,
} as const;

/** Brand color, unless it would vanish on the near-black UI (Rust, Bun, Deno are black). */
export function readableBrandColor(hex: string): string {
  const [r, g, b] = [0, 2, 4].map((i) => {
    const c = parseInt(hex.slice(i, i + 2), 16) / 255;
    return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
  });
  const luminance = 0.2126 * r + 0.7152 * g + 0.0722 * b;
  return luminance < 0.06 ? "#f4f4f6" : `#${hex}`;
}
