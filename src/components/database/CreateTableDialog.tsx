import { useMemo, useState } from "react";
import { Loader2, Plus, Table2, X } from "lucide-react";
import { InfoTip } from "../InfoTip";
import { type ColumnDefault, type ColumnType, type DbKind, type NewColumnDef, COLUMN_TYPES, dbCreateTable, newIdentifierError } from "../../lib/docker";
import { errorMessage } from "../../lib/wsl";
import { BTN_GHOST, BTN_ICON, BTN_PRIMARY, FIELD, INPUT, SELECT } from "../../lib/ui";
import { DialogShell, ErrorBox } from "./common";

interface Props {
  distro: string;
  containerId: string;
  kind: DbKind;
  onClose: () => void;
  onCreated: (table: string) => void;
}

type DefaultChoice = "none" | "value" | "now" | "random_uuid" | "true" | "false";

interface Draft {
  key: number;
  name: string;
  type: ColumnType;
  required: boolean;
  unique: boolean;
  primaryKey: boolean;
  defaultChoice: DefaultChoice;
  defaultValue: string;
}

const DEFAULT_LABEL: Record<DefaultChoice, string> = {
  none: "Sem valor padrão",
  value: "Um valor fixo…",
  now: "Agora (data/hora atual)",
  random_uuid: "UUID aleatório",
  true: "Verdadeiro",
  false: "Falso",
};

function defaultChoices(type: ColumnType): DefaultChoice[] {
  switch (type) {
    case "identity":
      return ["none"];
    case "timestamp":
    case "date":
      return ["none", "now", "value"];
    case "uuid":
      return ["none", "random_uuid", "value"];
    case "bool":
      return ["none", "true", "false"];
    default:
      return ["none", "value"];
  }
}

let nextKey = 0;
const draft = (d: Partial<Draft>): Draft => ({
  key: nextKey++,
  name: "",
  type: "text",
  required: false,
  unique: false,
  primaryKey: false,
  defaultChoice: "none",
  defaultValue: "",
  ...d,
});

function toColumn(d: Draft): NewColumnDef {
  const def: ColumnDefault | null =
    d.defaultChoice === "none"
      ? null
      : d.defaultChoice === "value"
        ? { kind: "value", value: d.defaultValue }
        : ({ kind: d.defaultChoice } as ColumnDefault);
  return { name: d.name, type: d.type, nullable: !d.required, primaryKey: d.primaryKey, unique: d.unique, default: def };
}

/** "+ Nova tabela", Supabase style: starts with id + created_at, add columns with plain names. */
export function CreateTableDialog({ distro, containerId, kind, onClose, onCreated }: Props) {
  const mysql = kind === "mysql" || kind === "mariadb";
  const [name, setName] = useState("");
  const [rows, setRows] = useState<Draft[]>(() => [
    draft({ name: "id", type: "identity", primaryKey: true, required: true }),
    draft({ name: "created_at", type: "timestamp", required: true, defaultChoice: "now" }),
  ]);
  // The column just added gets the cursor.
  const [added, setAdded] = useState<number | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const update = (key: number, patch: Partial<Draft>) =>
    setRows((all) =>
      all.map((r) => {
        if (r.key !== key) return r;
        const next = { ...r, ...patch };
        if (patch.type) {
          if (!defaultChoices(next.type).includes(next.defaultChoice)) next.defaultChoice = "none";
          if (next.type === "identity") next.primaryKey = true;
        }
        if (next.primaryKey) next.required = true;
        return next;
      }),
    );

  const problems = useMemo(() => {
    const out: string[] = [];
    const nameError = newIdentifierError(name);
    if (name && nameError) out.push(`Nome da tabela: ${nameError}`);
    const seen = new Set<string>();
    for (const r of rows) {
      const e = newIdentifierError(r.name);
      if (r.name && e) out.push(`Coluna “${r.name}”: ${e}`);
      if (seen.has(r.name.toLowerCase())) out.push(`A coluna “${r.name}” aparece duas vezes`);
      seen.add(r.name.toLowerCase());
      if (r.type === "identity" && !r.primaryKey) out.push(`“${r.name}”: número automático só na chave primária`);
      if (mysql && (r.type === "text" || r.type === "json") && (r.primaryKey || r.unique))
        out.push(`“${r.name}”: no MySQL use Texto curto para chave ou valor único`);
    }
    return out;
  }, [name, rows, mysql]);
  const complete = !!name && rows.length > 0 && rows.every((r) => r.name);

  const create = async () => {
    setBusy(true);
    setError(null);
    try {
      await dbCreateTable(distro, containerId, name, rows.map(toColumn));
      onCreated(name);
    } catch (err) {
      setError(errorMessage(err));
      setBusy(false);
    }
  };

  return (
    <DialogShell
      icon={<Table2 className="size-5 text-zinc-100" />}
      eyebrow="Nova tabela"
      title="Criar uma tabela"
      description="Uma tabela é como uma planilha: cada coluna guarda um tipo de informação e cada linha é um registro. Já começamos com as colunas que quase toda tabela tem."
      busy={busy}
      width="w-[min(900px,100%)]"
      onClose={onClose}
      footer={
        <>
          <button type="button" onClick={onClose} disabled={busy} className={`${BTN_GHOST} h-8`}>
            Cancelar
          </button>
          <button type="button" onClick={create} disabled={busy || !complete || problems.length > 0} className={`${BTN_PRIMARY} h-8 px-4`}>
            {busy ? <Loader2 className="size-3.5 animate-spin" /> : <Plus className="size-3.5" />}
            Criar tabela
          </button>
        </>
      }
    >
      <div className="space-y-4 px-6 py-5">
        <label className="block max-w-sm">
          <span className="mb-1 flex items-center gap-1.5 text-[11px] text-zinc-400">
            Nome da tabela
            <InfoTip>Use minúsculas e _ no lugar de espaços, sem acentos (ex.: clientes, itens_pedido). É o nome que o seu código vai usar.</InfoTip>
          </span>
          <span className={FIELD}>
            <input value={name} onChange={(e) => setName(e.target.value.trim())} placeholder="clientes" className={`${INPUT} py-0.5 font-mono text-sm`} autoFocus spellCheck={false} />
          </span>
        </label>

        <div>
          <div className="mb-1.5 grid grid-cols-[minmax(0,1.1fr)_minmax(0,1.2fr)_minmax(0,1.3fr)_3.75rem_3.75rem_3.75rem_1.75rem] items-center gap-2 text-[11px] text-zinc-500">
            <span>Coluna</span>
            <span>Tipo</span>
            <span>Valor padrão</span>
            <span className="flex items-center justify-center gap-1">
              Obrig.
              <InfoTip>Obrigatória: toda linha precisa ter esse valor preenchido.</InfoTip>
            </span>
            <span className="flex items-center justify-center gap-1">
              Único
              <InfoTip>Único: duas linhas não podem ter o mesmo valor (bom para e-mail, CPF, slug).</InfoTip>
            </span>
            <span className="flex items-center justify-center gap-1">
              Chave
              <InfoTip align="right">
                Chave primária: a coluna que identifica cada linha (normalmente o <span className="font-mono">id</span>). É ela que permite apagar
                uma linha pela planilha.
              </InfoTip>
            </span>
            <span />
          </div>
          <div className="space-y-1.5">
            {rows.map((r) => {
              const choices = defaultChoices(r.type);
              return (
                <div key={r.key} className="grid grid-cols-[minmax(0,1.1fr)_minmax(0,1.2fr)_minmax(0,1.3fr)_3.75rem_3.75rem_3.75rem_1.75rem] items-center gap-2">
                  <span className={FIELD}>
                    <input value={r.name} onChange={(e) => update(r.key, { name: e.target.value.trim() })} placeholder="nome" className={`${INPUT} py-0.5 font-mono`} spellCheck={false} autoFocus={r.key === added} />
                  </span>
                  <span className={FIELD} title={COLUMN_TYPES.find((t) => t.type === r.type)?.hint}>
                    <select value={r.type} onChange={(e) => update(r.key, { type: e.target.value as ColumnType })} className={SELECT}>
                      {COLUMN_TYPES.map((t) => (
                        <option key={t.type} value={t.type}>
                          {t.label} ({t.sql})
                        </option>
                      ))}
                    </select>
                  </span>
                  <span className={FIELD}>
                    {r.defaultChoice === "value" ? (
                      <>
                        <input
                          value={r.defaultValue}
                          onChange={(e) => update(r.key, { defaultValue: e.target.value })}
                          placeholder="valor"
                          className={`${INPUT} py-0.5 font-mono`}
                        />
                        <button type="button" onClick={() => update(r.key, { defaultChoice: "none", defaultValue: "" })} className="text-zinc-600 hover:text-zinc-200" title="Sem valor padrão">
                          <X className="size-3" />
                        </button>
                      </>
                    ) : (
                      <select
                        value={r.defaultChoice}
                        disabled={choices.length === 1}
                        onChange={(e) => update(r.key, { defaultChoice: e.target.value as DefaultChoice })}
                        className={`${SELECT} disabled:cursor-default disabled:text-zinc-600`}
                      >
                        {choices.map((c) => (
                          <option key={c} value={c}>
                            {r.type === "identity" ? "Automático (1, 2, 3…)" : DEFAULT_LABEL[c]}
                          </option>
                        ))}
                      </select>
                    )}
                  </span>
                  <Check checked={r.required} disabled={r.primaryKey} onChange={(v) => update(r.key, { required: v })} label="Obrigatória" />
                  <Check checked={r.unique && !r.primaryKey} disabled={r.primaryKey} onChange={(v) => update(r.key, { unique: v })} label="Único" />
                  <Check checked={r.primaryKey} disabled={r.type === "identity"} onChange={(v) => update(r.key, { primaryKey: v })} label="Chave primária" />
                  <button
                    type="button"
                    onClick={() => setRows((all) => all.filter((x) => x.key !== r.key))}
                    disabled={rows.length === 1}
                    className={`${BTN_ICON} hover:text-rose-300`}
                    title="Remover coluna"
                  >
                    <X className="size-3.5" />
                  </button>
                </div>
              );
            })}
          </div>
          <button type="button" onClick={() => {
              const d = draft({});
              setAdded(d.key);
              setRows((all) => [...all, d]);
            }} className={`${BTN_GHOST} mt-3`}>
            <Plus className="size-3.5" />
            Adicionar coluna
          </button>
        </div>

        {problems.length > 0 && (
          <ul className="space-y-0.5 text-[11px] text-rose-300">
            {problems.map((p) => (
              <li key={p}>• {p}</li>
            ))}
          </ul>
        )}
        {error && <ErrorBox>{error}</ErrorBox>}
      </div>
    </DialogShell>
  );
}

function Check({ checked, disabled, onChange, label }: { checked: boolean; disabled?: boolean; onChange: (v: boolean) => void; label: string }) {
  return (
    <span className="flex justify-center">
      <input
        type="checkbox"
        aria-label={label}
        title={label}
        checked={checked}
        disabled={disabled}
        onChange={(e) => onChange(e.target.checked)}
        className="size-3.5 cursor-pointer accent-zinc-100 disabled:cursor-default disabled:opacity-40"
      />
    </span>
  );
}
