import { STACK_BRANDS, type SimpleIcon } from "../../lib/brands";
import { BrandIcon } from "../BrandIcon";

// The Explorer is where color earns its place: telling stacks apart at a glance.
// Class strings are literal so Tailwind picks them up.
export interface Stack {
  label: string;
  markers: string[];
  brand: SimpleIcon;
  /** Chip: border, background, text. */
  chip: string;
  /** Folder icon of a project whose main stack is this one. */
  folder: string;
}

const STACKS: Stack[] = [
  {
    label: "Node",
    markers: ["package.json"],
    brand: STACK_BRANDS.node,
    chip: "border-green-500/30 bg-green-500/10 text-green-300",
    folder: "text-green-400",
  },
  {
    label: "Rust",
    markers: ["Cargo.toml"],
    brand: STACK_BRANDS.rust,
    chip: "border-orange-500/30 bg-orange-500/10 text-orange-300",
    folder: "text-orange-400",
  },
  {
    label: "Python",
    markers: ["pyproject.toml", "requirements.txt", ".venv", "venv"],
    brand: STACK_BRANDS.python,
    chip: "border-yellow-500/30 bg-yellow-500/10 text-yellow-200",
    folder: "text-yellow-300",
  },
  {
    label: "Go",
    markers: ["go.mod"],
    brand: STACK_BRANDS.go,
    chip: "border-cyan-500/30 bg-cyan-500/10 text-cyan-300",
    folder: "text-cyan-400",
  },
  {
    label: "Docker",
    markers: ["Dockerfile", "docker-compose.yml", "docker-compose.yaml", "compose.yml", "compose.yaml"],
    brand: STACK_BRANDS.docker,
    chip: "border-sky-500/30 bg-sky-500/10 text-sky-300",
    folder: "text-sky-400",
  },
  {
    label: "Git",
    markers: [".git"],
    brand: STACK_BRANDS.git,
    chip: "border-red-500/30 bg-red-500/10 text-red-300",
    folder: "text-red-400",
  },
];

/** Build output folders offered for one-click cleanup. */
export const HEAVY_FOLDERS = ["node_modules", "target"] as const;

export function stacksFor(markers: string[]): Stack[] {
  return STACKS.filter((s) => s.markers.some((m) => markers.includes(m)));
}

export function isProject(markers: string[]): boolean {
  return stacksFor(markers).length > 0;
}

/** The stack that colors the row: the first language/runtime, else Docker, else Git. */
export function mainStack(markers: string[]): Stack | null {
  return stacksFor(markers)[0] ?? null;
}

export function ProjectBadges({ markers }: { markers: string[] }) {
  return (
    <>
      {stacksFor(markers).map((s) => (
        <span
          key={s.label}
          className={`flex shrink-0 items-center gap-1 rounded-sm border px-1.5 py-px font-mono text-[10px] leading-4 tracking-wide uppercase ${s.chip}`}
        >
          <BrandIcon icon={s.brand} className="size-2.5" color="currentColor" />
          {s.label}
        </span>
      ))}
    </>
  );
}
