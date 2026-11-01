import { useState } from "react";
import { KeyRound, Loader2, Plus, Rows3 } from "lucide-react";
import { type DbColumn, dbInsertRow } from "../../lib/docker";
import { errorMessage } from "../../lib/wsl";
import { BTN_GHOST, BTN_PRIMARY, FIELD, INPUT, SELECT } from "../../lib/ui";
import { DialogShell, ErrorBox } from "./common";

interface Props {
  distro: string;
  containerId: string;
  /** null = Redis (new key). */
  table: string | null;
  columns: DbColumn[];
  onClose: () => void;
  onInserted: (key: string | null) => void;
}

type InputKind = "bool" | "json" | "number" | "datetime" | "date" | "text";

/** What kind of field a column needs, from the database's own type name. */
export function inputKind(dataType: string): InputKind {
  const t = dataType.toLowerCase();
  if (t === "boolean" || t === "bool" || t === "tinyint(1)") return "bool";
  if (t.startsWith("json")) return "json";
  if (t.startsWith("timestamp") || t.startsWith("datetime")) return "datetime";
  if (t === "date") return "date";
  if (/^(smallint|integer|bigint|int|tinyint|mediumint|numeric|decimal|real|double|float|serial|bigserial)/.test(t)) return "number";
  return "text";
}

const PLACEHOLDER: Record<InputKind, string> = {
  bool: "",
  json: '{"chave": "valor"}',
  number: "0",
  datetime: "2026-11-01 14:30:00",
  date: "2026-11-01",
  text: "",
};

/** "+ Inserir linha": one field per column. Empty = the database decides (default or NULL). */
export function InsertRowDialog({ distro, containerId, table, columns, onClose, onInserted }: Props) {
  const redis = table === null;
  const [values, setValues] = useState<Record<string, string>>({});
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const set = (name: string, v: string) => setValues((all) => ({ ...all, [name]: v }));

  const save = async () => {
    setError(null);
    const out: Record<string, unknown> = {};
    try {
      if (redis) {
        if (!values.key?.trim()) throw new Error("Dê um nome à chave.");
        out.key = values.key.trim();
        out.value = values.value ?? "";
        if (values.ttl) out.ttl = Number(values.ttl);
      } else {
        for (const c of columns) {
          const raw = values[c.name];
          if (raw === undefined || raw === "") continue;
          const kind = inputKind(c.dataType);
          if (kind === "bool") out[c.name] = raw === "true";
          else if (kind === "json") {
            try {
              out[c.name] = JSON.parse(raw);
            } catch {
              throw new Error(`${c.name}: o JSON não é válido.`);
            }
          } else out[c.name] = raw;
        }
      }
    } catch (err) {
      setError(errorMessage(err));
      return;
    }
    setBusy(true);
    try {
      await dbInsertRow(distro, containerId, table ?? "", out);
      onInserted(redis ? String(out.key) : null);
    } catch (err) {
      setError(errorMessage(err));
      setBusy(false);
    }
  };

  return (
    <DialogShell
      icon={<Rows3 className="size-5 text-zinc-100" />}
      eyebrow={redis ? "Nova chave" : `Inserir linha · ${table}`}
      title={redis ? "Guardar um valor" : "Nova linha"}
      description={
        redis
          ? "Cria uma chave de texto. Para listas, hashes e outros tipos use a aba Comandos Redis."
          : "Campos vazios ficam por conta do banco: ele usa o valor padrão da coluna (ou deixa vazio, quando pode)."
      }
      busy={busy}
      width="w-[min(620px,100%)]"
      onClose={onClose}
      footer={
        <>
          <button type="button" onClick={onClose} disabled={busy} className={`${BTN_GHOST} h-8`}>
            Cancelar
          </button>
          <button type="button" onClick={save} disabled={busy} className={`${BTN_PRIMARY} h-8 px-4`}>
            {busy ? <Loader2 className="size-3.5 animate-spin" /> : <Plus className="size-3.5" />}
            {redis ? "Guardar" : "Inserir"}
          </button>
        </>
      }
    >
      <div className="space-y-3 px-6 py-5">
        {redis ? (
          <>
            <Row label="Chave" hint="Ex.: user:1, sessao:abc">
              <input value={values.key ?? ""} onChange={(e) => set("key", e.target.value)} className={`${INPUT} py-0.5 font-mono`} autoFocus />
            </Row>
            <Row label="Valor">
              <textarea value={values.value ?? ""} onChange={(e) => set("value", e.target.value)} rows={4} className={`${INPUT} resize-y py-0.5 font-mono`} />
            </Row>
            <Row label="Apagar sozinha depois de (segundos)" hint="Vazio = nunca expira">
              <input value={values.ttl ?? ""} onChange={(e) => set("ttl", e.target.value.replace(/\D/g, ""))} inputMode="numeric" className={`${INPUT} py-0.5 font-mono`} />
            </Row>
          </>
        ) : (
          columns.map((c) => {
            const kind = inputKind(c.dataType);
            const auto = c.isIdentity || c.defaultValue !== null;
            const required = !c.isNullable && !auto;
            return (
              <Row
                key={c.name}
                label={
                  <>
                    {c.isPrimaryKey && <KeyRound className="size-3 text-zinc-300" />}
                    <span className="font-mono text-zinc-200">{c.name}</span>
                    <span className="font-mono text-[10px] text-zinc-600">{c.dataType}</span>
                    {required && <span className="text-[10px] text-zinc-400">obrigatório</span>}
                  </>
                }
                hint={c.isIdentity ? "Automático: deixe vazio" : c.defaultValue ? `Vazio = ${c.defaultValue}` : c.isNullable ? "Vazio = NULL" : undefined}
              >
                {kind === "bool" ? (
                  <select value={values[c.name] ?? ""} onChange={(e) => set(c.name, e.target.value)} className={SELECT}>
                    <option value="">{auto ? "(padrão)" : c.isNullable ? "(vazio)" : "Escolha"}</option>
                    <option value="true">Verdadeiro</option>
                    <option value="false">Falso</option>
                  </select>
                ) : kind === "json" ? (
                  <textarea
                    value={values[c.name] ?? ""}
                    onChange={(e) => set(c.name, e.target.value)}
                    rows={3}
                    placeholder={PLACEHOLDER.json}
                    className={`${INPUT} resize-y py-0.5 font-mono`}
                  />
                ) : (
                  <input
                    value={values[c.name] ?? ""}
                    onChange={(e) => set(c.name, e.target.value)}
                    placeholder={c.isIdentity ? "automático" : PLACEHOLDER[kind]}
                    inputMode={kind === "number" ? "decimal" : undefined}
                    className={`${INPUT} py-0.5 font-mono`}
                  />
                )}
              </Row>
            );
          })
        )}
        {error && <ErrorBox>{error}</ErrorBox>}
      </div>
    </DialogShell>
  );
}

function Row({ label, hint, children }: { label: React.ReactNode; hint?: string; children: React.ReactNode }) {
  return (
    <label className="block">
      <span className="mb-1 flex items-center gap-1.5 text-[11px] text-zinc-400">
        {label}
        {hint && <span className="ml-auto text-[10px] text-zinc-600">{hint}</span>}
      </span>
      <span className={FIELD}>{children}</span>
    </label>
  );
}
