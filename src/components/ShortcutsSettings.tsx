import { useEffect, useState, type ReactNode } from "react";
import { Keyboard, Lock, MousePointerClick, RotateCcw, X } from "lucide-react";
import {
  FIXED_SHORTCUTS,
  MOUSE_GESTURES,
  SHORTCUT_GROUPS,
  type ShortcutDef,
  type ShortcutId,
  type Shortcuts,
  bindingKeys,
  invalidKeyBinding,
  keyFromEvent,
  mouseGestures,
  toggleGesture,
} from "../lib/shortcuts";
import { BTN_GHOST } from "../lib/ui";

interface Props {
  shortcuts: Shortcuts;
  /** Key bindings used by more than one action. */
  conflicts: Set<ShortcutId>;
  onChange: (id: ShortcutId, binding: string) => void;
  onResetAll: () => void;
}

export function ShortcutsSettings({ shortcuts, conflicts, onChange, onResetAll }: Props) {
  const changed = SHORTCUT_GROUPS.some((g) => g.items.some((d) => shortcuts[d.id] !== d.default));

  return (
    <div className="space-y-6">
      <div className="flex items-center gap-3">
        <p className="flex-1 text-xs text-zinc-500">
          Clique num atalho e pressione a nova combinação. <Kbd>Esc</Kbd> cancela, <Kbd>Backspace</Kbd> desativa.
        </p>
        <button type="button" onClick={onResetAll} disabled={!changed} className={`${BTN_GHOST} shrink-0`}>
          <RotateCcw className="size-3.5" />
          Restaurar padrões
        </button>
      </div>

      {SHORTCUT_GROUPS.map((group) => (
        <section key={group.title}>
          <GroupTitle icon={group.items[0].kind === "key" ? Keyboard : MousePointerClick}>{group.title}</GroupTitle>
          <div className="overflow-hidden rounded-lg border border-zinc-800 bg-zinc-900">
            {group.items.map((def) => (
              <Row key={def.id} def={def}>
                {def.kind === "key" ? (
                  <KeyRecorder
                    binding={shortcuts[def.id]}
                    conflict={conflicts.has(def.id)}
                    onChange={(b) => onChange(def.id, b)}
                  />
                ) : (
                  <GestureChips binding={shortcuts[def.id]} onChange={(b) => onChange(def.id, b)} />
                )}
                <button
                  type="button"
                  onClick={() => onChange(def.id, def.default)}
                  title="Restaurar padrão"
                  aria-label="Restaurar padrão"
                  className={`rounded p-1 text-zinc-500 transition-colors hover:bg-zinc-800 hover:text-zinc-100 ${
                    shortcuts[def.id] === def.default ? "invisible" : ""
                  }`}
                >
                  <RotateCcw className="size-3.5" />
                </button>
              </Row>
            ))}
          </div>
        </section>
      ))}

      <section>
        <GroupTitle icon={Lock}>Fixos</GroupTitle>
        <div className="grid grid-cols-3 gap-2">
          {FIXED_SHORTCUTS.map((group) => (
            <div key={group.title} className="rounded-lg border border-zinc-800 bg-zinc-900 px-3 py-2.5">
              <div className="mb-2 text-[10px] tracking-wider text-zinc-500 uppercase">{group.title}</div>
              <div className="space-y-1.5">
                {group.items.map((item) => (
                  <div key={item.label} className="flex items-center gap-2 text-xs text-zinc-300">
                    <span className="min-w-0 flex-1 truncate" title={item.label}>
                      {item.label}
                    </span>
                    <Keys binding={item.binding} />
                  </div>
                ))}
              </div>
            </div>
          ))}
        </div>
      </section>
    </div>
  );
}

function GroupTitle({ icon: Icon, children }: { icon: typeof Keyboard; children: string }) {
  return (
    <h3 className="mb-2.5 flex items-center gap-2 text-[11px] font-semibold tracking-wider text-zinc-300 uppercase">
      <Icon className="size-3.5 text-zinc-500" />
      {children}
    </h3>
  );
}

function Row({ def, children }: { def: ShortcutDef; children: ReactNode }) {
  return (
    <div className="flex items-center gap-3 border-b border-zinc-800 px-4 py-2.5 last:border-0">
      <div className="min-w-0 flex-1">
        <div className="text-sm text-zinc-100">{def.label}</div>
        <div className="truncate text-[11px] text-zinc-500">{def.description}</div>
      </div>
      {children}
    </div>
  );
}

/** Click, then press the combo. Listens in the capture phase so the dialog's Esc (close) and
 *  the Explorer's own shortcuts never see the keys being recorded. */
function KeyRecorder({ binding, conflict, onChange }: { binding: string; conflict: boolean; onChange: (b: string) => void }) {
  const [recording, setRecording] = useState(false);
  const [problem, setProblem] = useState<string | null>(null);

  useEffect(() => {
    if (!recording) return;
    const onKey = (e: KeyboardEvent) => {
      e.preventDefault();
      e.stopPropagation();
      const plain = !e.ctrlKey && !e.altKey && !e.shiftKey;
      if (plain && e.key === "Escape") return setRecording(false);
      if (plain && (e.key === "Backspace" || e.key === "Delete")) {
        onChange("");
        setProblem(null);
        return setRecording(false);
      }
      const next = keyFromEvent(e);
      if (!next) return; // only modifiers so far
      const invalid = invalidKeyBinding(next);
      setProblem(invalid);
      if (invalid) return;
      onChange(next);
      setRecording(false);
    };
    const cancel = () => setRecording(false);
    window.addEventListener("keydown", onKey, true);
    window.addEventListener("mousedown", cancel, true);
    return () => {
      window.removeEventListener("keydown", onKey, true);
      window.removeEventListener("mousedown", cancel, true);
    };
  }, [recording, onChange]);

  return (
    <div className="flex flex-col items-end gap-1">
      <button
        type="button"
        onClick={() => {
          setProblem(null);
          setRecording(true);
        }}
        className={`flex h-8 min-w-36 items-center justify-center gap-1 rounded-md border px-2 transition-colors ${
          recording
            ? "border-zinc-100 bg-zinc-950"
            : conflict
              ? "border-amber-400/50 hover:border-amber-300"
              : "border-zinc-800 hover:border-zinc-600"
        }`}
      >
        {recording ? (
          <span className="animate-pulse text-xs text-zinc-300">Pressione o atalho…</span>
        ) : binding ? (
          <Keys binding={binding} />
        ) : (
          <span className="text-xs text-zinc-600">Desativado</span>
        )}
      </button>
      {problem && <span className="max-w-72 text-right text-[11px] text-amber-300">{problem}</span>}
      {!problem && conflict && <span className="text-[11px] text-amber-300">Usado em outra ação</span>}
    </div>
  );
}

function GestureChips({ binding, onChange }: { binding: string; onChange: (b: string) => void }) {
  const active = mouseGestures(binding);
  return (
    <div className="flex flex-wrap justify-end gap-1.5">
      {MOUSE_GESTURES.map((g) => {
        const on = active.includes(g.id);
        return (
          <button
            key={g.id}
            type="button"
            role="checkbox"
            aria-checked={on}
            onClick={() => onChange(toggleGesture(binding, g.id))}
            className={`rounded-full border px-2.5 py-1 text-[11px] whitespace-nowrap transition-colors ${
              on
                ? "border-zinc-100 bg-zinc-100 font-semibold text-void"
                : "border-zinc-800 text-zinc-500 hover:border-zinc-600 hover:text-zinc-200"
            }`}
          >
            {g.label}
          </button>
        );
      })}
      {active.length === 0 && (
        <span className="flex items-center gap-1 self-center text-[11px] text-zinc-600">
          <X className="size-3" />
          Desativado
        </span>
      )}
    </div>
  );
}

function Keys({ binding }: { binding: string }) {
  return (
    <span className="flex items-center gap-1">
      {bindingKeys(binding).map((k, i) => (
        <Kbd key={i}>{k}</Kbd>
      ))}
    </span>
  );
}

function Kbd({ children }: { children: string }) {
  return (
    <kbd className="inline-flex h-5 min-w-5 items-center justify-center rounded border border-b-2 border-zinc-700 bg-zinc-950 px-1.5 font-mono text-[10px] text-zinc-200">
      {children}
    </kbd>
  );
}
