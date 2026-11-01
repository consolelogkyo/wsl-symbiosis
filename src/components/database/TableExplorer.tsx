import { useCallback, useEffect, useMemo, useState } from "react";
import {
  ArrowLeft,
  ChevronLeft,
  ChevronRight,
  Code2,
  Columns3,
  Eye,
  KeyRound,
  Loader2,
  Plus,
  RefreshCw,
  Search,
  Table2,
  Trash2,
} from "lucide-react";
import { BrandIcon } from "../BrandIcon";
import { InfoTip } from "../InfoTip";
import {
  type ContainerInfo,
  type DbTableData,
  type DbTableSummary,
  DB_ICONS,
  DB_TITLES,
  dbDeleteRow,
  dbListTables,
  dbQueryTable,
} from "../../lib/docker";
import { errorMessage } from "../../lib/wsl";
import { BTN_DANGER_SOLID, BTN_GHOST, BTN_ICON, BTN_PRIMARY, FIELD, INPUT } from "../../lib/ui";
import { ErrorBox, StateBadge } from "./common";
import { CreateTableDialog } from "./CreateTableDialog";
import { InsertRowDialog } from "./InsertRowDialog";
import { SqlEditor } from "./SqlEditor";

const PAGE = 50;

type Tab = "data" | "structure" | "sql";

interface Props {
  distro: string;
  container: ContainerInfo;
  onBack: () => void;
}

/** "Gerenciar tabelas & dados": tables on the left, data / structure / SQL in the middle. */
export function TableExplorer({ distro, container, onBack }: Props) {
  const db = container.database!;
  const redis = db.kind === "redis";
  const [tables, setTables] = useState<DbTableSummary[] | null>(null);
  const [listError, setListError] = useState<string | null>(null);
  const [listing, setListing] = useState(false);
  const [filter, setFilter] = useState("");
  const [selected, setSelected] = useState<string | null>(null);
  const [tab, setTab] = useState<Tab>("data");
  const [data, setData] = useState<DbTableData | null>(null);
  const [dataError, setDataError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [offset, setOffset] = useState(0);
  const [dialog, setDialog] = useState<"insert" | "create" | null>(null);
  const [confirmDelete, setConfirmDelete] = useState<string | null>(null);
  const [deleting, setDeleting] = useState(false);

  const loadTables = useCallback(async () => {
    setListing(true);
    try {
      const list = await dbListTables(distro, container.id);
      setTables(list);
      setListError(null);
      return list;
    } catch (err) {
      setListError(errorMessage(err));
      return null;
    } finally {
      setListing(false);
    }
  }, [distro, container.id]);

  const loadData = useCallback(
    async (table: string, at: number) => {
      setLoading(true);
      try {
        setData(await dbQueryTable(distro, container.id, table, PAGE, at));
        setDataError(null);
      } catch (err) {
        setData(null);
        setDataError(errorMessage(err));
      } finally {
        setLoading(false);
      }
    },
    [distro, container.id],
  );

  useEffect(() => {
    void loadTables().then((list) => {
      const first = list?.find((t) => t.kind === "table") ?? list?.[0];
      if (first) setSelected((s) => s ?? first.id);
    });
  }, [loadTables]);

  useEffect(() => {
    setConfirmDelete(null);
    if (selected) void loadData(selected, offset);
  }, [selected, offset, loadData]);

  const select = (id: string) => {
    setSelected(id);
    setOffset(0);
    if (tab === "sql" && !redis) return;
    setTab((t) => (t === "structure" || t === "data" ? t : "data"));
  };

  const refreshAll = async () => {
    const list = await loadTables();
    if (selected && list?.some((t) => t.id === selected)) await loadData(selected, offset);
    else if (selected) {
      setSelected(list?.[0]?.id ?? null);
      setData(null);
    }
  };

  const visible = useMemo(() => {
    const q = filter.trim().toLowerCase();
    return (tables ?? []).filter((t) => !q || t.id.toLowerCase().includes(q));
  }, [tables, filter]);

  const current = tables?.find((t) => t.id === selected) ?? null;
  const pk = data?.columns.filter((c) => c.isPrimaryKey) ?? [];
  const canDelete = redis || pk.length === 1;
  const isView = current?.kind === "view";

  const deleteRow = async (row: Record<string, unknown>) => {
    if (!selected) return;
    setDeleting(true);
    try {
      if (redis) await dbDeleteRow(distro, container.id, selected, "", "");
      else await dbDeleteRow(distro, container.id, selected, pk[0].name, String(row[pk[0].name]));
      setConfirmDelete(null);
      if (redis) {
        setSelected(null);
        setData(null);
        await loadTables();
      } else {
        await Promise.all([loadData(selected, offset), loadTables()]);
      }
    } catch (err) {
      setDataError(errorMessage(err));
    } finally {
      setDeleting(false);
    }
  };

  const tabs: { id: Tab; label: string; icon: typeof Table2 }[] = redis
    ? [
        { id: "data", label: "Valor da chave", icon: Eye },
        { id: "sql", label: "Comandos Redis", icon: Code2 },
      ]
    : [
        { id: "data", label: "Dados da tabela", icon: Table2 },
        { id: "structure", label: "Estrutura / Colunas", icon: Columns3 },
        { id: "sql", label: "Editor SQL", icon: Code2 },
      ];

  return (
    <div className="flex h-full flex-col">
      {/* Header */}
      <div className="flex items-center gap-3 border-b border-zinc-800 bg-zinc-950 px-4 py-2.5">
        <button type="button" onClick={onBack} className={`${BTN_GHOST} whitespace-nowrap`}>
          <ArrowLeft className="size-3.5" />
          Bancos
        </button>
        <BrandIcon icon={DB_ICONS[db.kind]} className="size-4" />
        <span className="truncate text-sm font-semibold text-zinc-100">{container.name}</span>
        <span className="text-xs whitespace-nowrap text-zinc-500">
          {DB_TITLES[db.kind]}
          {db.database ? ` · ${db.database}` : ""}
        </span>
        <StateBadge state={container.state} />
        <span className="flex-1" />
        <button type="button" onClick={refreshAll} disabled={listing || loading} className={`${BTN_GHOST} whitespace-nowrap`}>
          {listing || loading ? <Loader2 className="size-3.5 animate-spin" /> : <RefreshCw className="size-3.5" />}
          Atualizar
        </button>
      </div>

      <div className="flex min-h-0 flex-1">
        {/* Tables */}
        <aside className="flex w-60 shrink-0 flex-col border-r border-zinc-800 bg-void">
          <div className="space-y-2 p-3">
            <button type="button" onClick={() => setDialog(redis ? "insert" : "create")} className={`${BTN_PRIMARY} w-full justify-center whitespace-nowrap`}>
              <Plus className="size-3.5" />
              {redis ? "Nova chave" : "Nova tabela"}
            </button>
            <label className={FIELD}>
              <Search className="size-3.5 shrink-0 text-zinc-600" />
              <input value={filter} onChange={(e) => setFilter(e.target.value)} placeholder={redis ? "Buscar chave" : "Buscar tabela"} className={INPUT} />
            </label>
          </div>
          <div className="flex items-center gap-1.5 px-4 pb-1.5 text-[11px] font-medium tracking-wider text-zinc-600 uppercase">
            {redis ? "Chaves" : "Tabelas"}
            {tables && <span className="font-mono">{tables.length}</span>}
            {redis && (
              <InfoTip>
                No Redis não há tabelas: cada informação fica guardada numa chave (como <span className="font-mono">user:1</span>). Mostramos
                até 1000 chaves.
              </InfoTip>
            )}
          </div>
          <div className="min-h-0 flex-1 overflow-y-auto px-2 pb-3">
            {listError && <p className="px-2 py-2 text-[11px] leading-relaxed whitespace-pre-wrap text-rose-300">{listError}</p>}
            {!tables && !listError && (
              <div className="flex items-center gap-2 px-2 py-3 text-xs text-zinc-500">
                <Loader2 className="size-3.5 animate-spin" />
                Lendo o banco…
              </div>
            )}
            {tables?.length === 0 && (
              <p className="px-2 py-3 text-xs leading-relaxed text-zinc-500">
                {redis ? "Nenhuma chave ainda." : "Nenhuma tabela ainda. Crie a primeira com “Nova tabela” ou pelo Editor SQL."}
              </p>
            )}
            {visible.map((t) => {
              const active = t.id === selected;
              return (
                <button
                  key={t.id}
                  type="button"
                  onClick={() => select(t.id)}
                  className={`relative flex w-full items-center gap-2 rounded-md px-2.5 py-1.5 text-left text-[13px] transition-colors ${
                    active ? "bg-zinc-900 text-zinc-100" : "text-zinc-400 hover:bg-zinc-900/60 hover:text-zinc-200"
                  }`}
                  title={t.id}
                >
                  {active && <span className="absolute inset-y-1 -left-2 w-0.5 bg-zinc-100" />}
                  {t.kind === "view" ? <Eye className="size-3.5 shrink-0 text-zinc-600" /> : <Table2 className="size-3.5 shrink-0 text-zinc-600" />}
                  <span className="min-w-0 flex-1 truncate font-mono text-xs">{t.id}</span>
                  <span className="shrink-0 font-mono text-[10px] text-zinc-600">{redis ? t.kind : (t.rows ?? "")}</span>
                </button>
              );
            })}
          </div>
        </aside>

        {/* Main */}
        <section className="flex min-w-0 flex-1 flex-col">
          <div className="flex items-center gap-1 border-b border-zinc-800 px-4 pt-2">
            {tabs.map(({ id, label, icon: Icon }) => {
              const active = tab === id;
              return (
                <button
                  key={id}
                  type="button"
                  onClick={() => setTab(id)}
                  className={`relative flex items-center gap-2 px-3 pt-1.5 pb-2.5 text-sm whitespace-nowrap transition-colors ${
                    active ? "font-medium text-zinc-100" : "text-zinc-500 hover:text-zinc-200"
                  }`}
                >
                  <Icon className="size-4" />
                  {label}
                  {active && <span className="absolute inset-x-2 -bottom-px h-0.5 bg-zinc-100" />}
                </button>
              );
            })}
          </div>

          {tab === "sql" ? (
            <SqlEditor
              key={container.id}
              distro={distro}
              containerId={container.id}
              kind={db.kind}
              table={redis ? null : selected}
              columns={data?.columns ?? []}
              onChanged={() => void refreshAll()}
            />
          ) : !selected ? (
            <div className="flex flex-1 items-center justify-center p-8 text-center text-xs text-zinc-500">
              {tables?.length === 0 ? "Comece criando uma tabela." : "Escolha uma tabela à esquerda."}
            </div>
          ) : (
            <div className="flex min-h-0 flex-1 flex-col">
              {/* Toolbar */}
              <div className="flex items-center gap-3 px-4 py-2.5">
                <span className="truncate font-mono text-sm text-zinc-100">{selected}</span>
                {data && (
                  <span className="text-xs whitespace-nowrap text-zinc-500">
                    {redis ? (
                      <>
                        {data.kind} · {data.ttl === -1 || data.ttl === null ? "não expira" : `expira em ${data.ttl} s`}
                        {data.total !== null && ` · ${data.total} ${data.total === 1 ? "item" : "itens"}`}
                      </>
                    ) : (
                      `${data.total ?? "?"} ${data.total === 1 ? "linha" : "linhas"}`
                    )}
                  </span>
                )}
                {isView && <span className="text-[11px] text-zinc-500">visão (somente leitura)</span>}
                <span className="flex-1" />
                {tab === "data" && data && data.total !== null && data.total > PAGE && (
                  <div className="flex items-center gap-1 font-mono text-[11px] whitespace-nowrap text-zinc-500">
                    <button type="button" className={BTN_ICON} disabled={offset === 0 || loading} onClick={() => setOffset(Math.max(0, offset - PAGE))} title="Página anterior">
                      <ChevronLeft className="size-3.5" />
                    </button>
                    {offset + 1}–{Math.min(offset + PAGE, data.total)} de {data.total}
                    <button
                      type="button"
                      className={BTN_ICON}
                      disabled={offset + PAGE >= data.total || loading}
                      onClick={() => setOffset(offset + PAGE)}
                      title="Próxima página"
                    >
                      <ChevronRight className="size-3.5" />
                    </button>
                  </div>
                )}
                {tab === "data" && redis && (
                  confirmDelete === "__key" ? (
                    <button type="button" onClick={() => data && deleteRow({})} disabled={deleting} className={BTN_DANGER_SOLID}>
                      {deleting ? <Loader2 className="size-3 animate-spin" /> : <Trash2 className="size-3" />}
                      Apagar a chave?
                    </button>
                  ) : (
                    <button type="button" onClick={() => setConfirmDelete("__key")} className={`${BTN_GHOST} whitespace-nowrap`}>
                      <Trash2 className="size-3.5" />
                      Apagar chave
                    </button>
                  )
                )}
                {tab === "data" && !redis && !isView && (
                  <button type="button" onClick={() => setDialog("insert")} disabled={!data} className={`${BTN_PRIMARY} whitespace-nowrap`}>
                    <Plus className="size-3.5" />
                    Inserir linha
                  </button>
                )}
              </div>

              {dataError && (
                <div className="px-4 pb-3">
                  <ErrorBox>{dataError}</ErrorBox>
                </div>
              )}

              {!data && loading && (
                <div className="flex items-center gap-2 px-4 py-3 text-xs text-zinc-500">
                  <Loader2 className="size-3.5 animate-spin" />
                  Carregando…
                </div>
              )}

              {data && tab === "data" && (
                <DataGrid
                  data={data}
                  deletable={canDelete && !isView && !redis}
                  confirm={confirmDelete}
                  deleting={deleting}
                  onAskDelete={setConfirmDelete}
                  onDelete={deleteRow}
                  pkName={pk.length === 1 ? pk[0].name : null}
                  dim={loading}
                />
              )}
              {data && tab === "structure" && <StructureTable data={data} />}
            </div>
          )}
        </section>
      </div>

      {dialog === "create" && (
        <CreateTableDialog
          distro={distro}
          containerId={container.id}
          kind={db.kind}
          onClose={() => setDialog(null)}
          onCreated={async (name) => {
            setDialog(null);
            await loadTables();
            setSelected(name);
            setOffset(0);
            setTab("data");
          }}
        />
      )}
      {dialog === "insert" && (
        <InsertRowDialog
          distro={distro}
          containerId={container.id}
          table={redis ? null : selected}
          columns={redis ? [] : (data?.columns ?? [])}
          onClose={() => setDialog(null)}
          onInserted={async (key) => {
            setDialog(null);
            await loadTables();
            if (redis && key) setSelected(key);
            else if (selected) await loadData(selected, offset);
          }}
        />
      )}
    </div>
  );
}

const REDIS_COLUMN_LABEL: Record<string, string> = { value: "valor", field: "campo", index: "posição", score: "pontuação", id: "id", fields: "campos" };

export function cellText(v: unknown): string {
  if (v === null || v === undefined) return "NULL";
  if (typeof v === "object") return JSON.stringify(v);
  return String(v);
}

function DataGrid(p: {
  data: DbTableData;
  deletable: boolean;
  pkName: string | null;
  confirm: string | null;
  deleting: boolean;
  dim: boolean;
  onAskDelete: (key: string | null) => void;
  onDelete: (row: Record<string, unknown>) => void;
}) {
  const { data } = p;
  const redis = data.kind !== null;
  if (data.rows.length === 0) {
    return <div className="px-4 py-6 text-xs text-zinc-500">{redis ? "Vazio." : "Nenhuma linha ainda. Use “Inserir linha” para adicionar a primeira."}</div>;
  }
  return (
    <div className={`min-h-0 flex-1 overflow-auto border-t border-zinc-800 transition-opacity ${p.dim ? "opacity-60" : ""}`}>
      <table className="w-max min-w-full border-collapse text-xs">
        <thead className="sticky top-0 z-10 bg-zinc-900">
          <tr>
            {data.columns.map((c) => (
              <th key={c.name} className="border-r border-b border-zinc-800 px-3 py-1.5 text-left align-top font-normal">
                <div className="flex items-center gap-1.5 font-medium whitespace-nowrap text-zinc-200">
                  {c.isPrimaryKey && (
                    <span title="Chave primária: identifica cada linha">
                      <KeyRound className="size-3 text-zinc-300" />
                    </span>
                  )}
                  {redis ? (REDIS_COLUMN_LABEL[c.name] ?? c.name) : c.name}
                </div>
                {!redis && <div className="font-mono text-[10px] whitespace-nowrap text-zinc-500">{c.dataType}</div>}
              </th>
            ))}
            {p.deletable && <th className="w-16 border-b border-zinc-800" />}
          </tr>
        </thead>
        <tbody>
          {data.rows.map((row, i) => {
            const key = p.pkName ? cellText(row[p.pkName]) : String(i);
            const confirming = p.confirm === key;
            return (
              <tr key={key + i} className="group hover:bg-zinc-900/60">
                {data.columns.map((c) => {
                  const v = row[c.name];
                  const text = cellText(v);
                  return (
                    <td
                      key={c.name}
                      title={text.length > 60 ? text : undefined}
                      className={`max-w-80 truncate border-r border-b border-zinc-800/70 px-3 py-1.5 font-mono text-[11px] ${
                        v === null || v === undefined ? "text-zinc-600 italic" : typeof v === "boolean" ? "text-zinc-100" : "text-zinc-300"
                      }`}
                    >
                      {text}
                    </td>
                  );
                })}
                {p.deletable && (
                  <td className="border-b border-zinc-800/70 px-2 py-1 text-right whitespace-nowrap">
                    {confirming ? (
                      <button type="button" onClick={() => p.onDelete(row)} disabled={p.deleting} className={BTN_DANGER_SOLID}>
                        {p.deleting ? <Loader2 className="size-3 animate-spin" /> : <Trash2 className="size-3" />}
                        Apagar?
                      </button>
                    ) : (
                      <button
                        type="button"
                        onClick={() => p.onAskDelete(key)}
                        className={`${BTN_ICON} opacity-0 group-hover:opacity-100 hover:text-rose-300`}
                        title="Apagar esta linha"
                      >
                        <Trash2 className="size-3.5" />
                      </button>
                    )}
                  </td>
                )}
              </tr>
            );
          })}
        </tbody>
      </table>
    </div>
  );
}

function StructureTable({ data }: { data: DbTableData }) {
  return (
    <div className="min-h-0 flex-1 overflow-auto border-t border-zinc-800">
      <table className="w-full border-collapse text-xs">
        <thead className="sticky top-0 bg-zinc-900 text-left text-[11px] tracking-wider text-zinc-500 uppercase">
          <tr>
            {["Coluna", "Tipo", "Obrigatória", "Valor padrão", "Chave"].map((h) => (
              <th key={h} className="border-b border-zinc-800 px-4 py-2 font-medium">
                {h}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {data.columns.map((c) => (
            <tr key={c.name} className="border-b border-zinc-800/70">
              <td className="px-4 py-2 font-mono text-zinc-100">{c.name}</td>
              <td className="px-4 py-2 font-mono text-zinc-400">{c.dataType}</td>
              <td className="px-4 py-2 text-zinc-400">{c.isNullable ? "não" : "sim"}</td>
              <td className="px-4 py-2 font-mono text-zinc-500">{c.isIdentity && !c.defaultValue ? "automático" : (c.defaultValue ?? "—")}</td>
              <td className="px-4 py-2 text-zinc-400">
                {c.isPrimaryKey ? (
                  <span className="flex items-center gap-1.5">
                    <KeyRound className="size-3 text-zinc-300" />
                    primária{c.isIdentity ? " · automática" : ""}
                  </span>
                ) : (
                  "—"
                )}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}
