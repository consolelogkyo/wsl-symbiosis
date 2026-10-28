import { useEffect, useRef, useState } from "react";
import { Check, ChevronDown, Star } from "lucide-react";
import type { Distro } from "../lib/wsl";
import { StatusDot } from "./StatusDot";

interface Props {
  distros: Distro[];
  selected: Distro | null;
  loading: boolean;
  onSelect: (name: string) => void;
}

/** Pill selector for the active distro (titlebar). */
export function DistroSelect({ distros, selected, loading, onSelect }: Props) {
  const [open, setOpen] = useState(false);
  const rootRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    const onPointer = (e: PointerEvent) => {
      if (!rootRef.current?.contains(e.target as Node)) setOpen(false);
    };
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && setOpen(false);
    document.addEventListener("pointerdown", onPointer);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("pointerdown", onPointer);
      document.removeEventListener("keydown", onKey);
    };
  }, [open]);

  return (
    <div ref={rootRef} className="relative">
      <button
        type="button"
        onClick={() => setOpen((o) => !o)}
        disabled={distros.length === 0}
        aria-haspopup="listbox"
        aria-expanded={open}
        title="Distro ativa"
        className={`flex h-7 max-w-52 items-center gap-2 rounded-full border px-3 text-xs leading-none transition-colors disabled:opacity-60 ${
          open ? "border-zinc-500 bg-zinc-900" : "border-zinc-800 hover:border-zinc-600"
        }`}
      >
        {selected ? (
          <>
            <StatusDot state={selected.state} />
            {/* Baseline alignment: the tag's text sits on the same line as the name's text. */}
            <span className="flex min-w-0 items-baseline gap-1.5">
              <span className="truncate font-semibold text-zinc-100">{selected.name}</span>
              <span className="rounded-sm bg-zinc-800 px-1 font-mono text-[10px] leading-[14px] text-zinc-400">
                WSL{selected.version}
              </span>
            </span>
          </>
        ) : (
          <span className="text-zinc-500">{loading ? "Carregando…" : "Nenhuma distro"}</span>
        )}
        <ChevronDown className={`size-3.5 shrink-0 text-zinc-500 transition-transform ${open ? "rotate-180" : ""}`} />
      </button>

      {open && (
        <ul
          role="listbox"
          className="absolute top-full right-0 z-50 mt-2 w-64 overflow-hidden rounded-lg border border-zinc-800 bg-zinc-900 p-1 shadow-2xl shadow-black/60"
        >
          <li className="px-2.5 pt-1.5 pb-1 text-[10px] font-medium tracking-wider text-zinc-500 uppercase">
            Distribuições
          </li>
          {distros.map((d) => {
            const isSelected = d.name === selected?.name;
            return (
              <li key={d.name}>
                <button
                  type="button"
                  role="option"
                  aria-selected={isSelected}
                  onClick={() => {
                    onSelect(d.name);
                    setOpen(false);
                  }}
                  className={`flex w-full items-center gap-2.5 rounded-md px-2.5 py-1.5 text-left text-sm transition-colors ${
                    isSelected ? "bg-zinc-100 text-void" : "text-zinc-300 hover:bg-zinc-800"
                  }`}
                >
                  <StatusDot state={d.state} />
                  <span className="min-w-0 flex-1 truncate font-medium">{d.name}</span>
                  {d.isDefault && (
                    <Star
                      className={`size-3 shrink-0 ${isSelected ? "fill-void text-void" : "fill-zinc-500 text-zinc-500"}`}
                      aria-label="Padrão do WSL"
                    />
                  )}
                  <span className={`shrink-0 font-mono text-[10px] ${isSelected ? "text-zinc-600" : "text-zinc-500"}`}>
                    v{d.version}
                  </span>
                  <Check className={`size-3.5 shrink-0 ${isSelected ? "" : "invisible"}`} />
                </button>
              </li>
            );
          })}
        </ul>
      )}
    </div>
  );
}
