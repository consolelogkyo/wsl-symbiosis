import { useEffect, useState, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { AlertTriangle, Check, CornerDownRight, Loader2, RotateCw, SlidersHorizontal, X } from "lucide-react";
import { formatBytes } from "../lib/format";
import { BTN_GHOST, BTN_PRIMARY } from "../lib/ui";
import { errorMessage } from "../lib/wsl";
import { GIB, type WslConfigSnapshot, type WslGlobalConfig, configChanged, getWslConfig, saveWslConfig } from "../lib/wslconfig";
import { InfoTip } from "./InfoTip";

interface Props {
  onClose: () => void;
  /** After a save: whether WSL was restarted (the new limits are already active). */
  onSaved: (restarted: boolean) => void;
}

/** Choices for "Devolver memória ao Windows" that deserve a second look before applying. */
const RECLAIM_WARNINGS: Record<string, { title: string; body: ReactNode; confirm: string }> = {
  dropcache: {
    title: "Usar o modo Imediato?",
    body: (
      <>
        <p>
          No modo Imediato, sempre que o Linux fica parado por um tempo, <b>todo</b> o cache é jogado fora de uma vez. A RAM volta
          rápido para o Windows, mas o Linux esquece os arquivos que tinha à mão.
        </p>
        <p>
          Na prática: ao voltar a trabalhar, o primeiro <i>build</i>, o <i>npm install</i> ou a abertura do projeto na IDE ficam
          visivelmente mais lentos, porque tudo precisa ser lido do disco de novo.
        </p>
        <p>
          O <b>Gradual</b> devolve a mesma memória, só que aos poucos, sem esse tranco. Por isso ele é o recomendado.
        </p>
      </>
    ),
    confirm: "Usar Imediato mesmo assim",
  },
  disabled: {
    title: "Desativar a devolução de memória?",
    body: (
      <>
        <p>
          Desativado, o Linux <b>nunca</b> devolve sozinho a memória que usou como cache. Depois de um dia de trabalho, o
          <span className="font-mono"> vmmemWSL</span> pode ficar ocupando vários GB no Gerenciador de Tarefas, mesmo com o Linux parado.
        </p>
        <p>
          Essa memória só volta quando você desligar o WSL ou usar o <b>Liberar cache</b> do Dashboard. Em computadores com pouca RAM,
          o Windows pode ficar lento.
        </p>
        <p>
          O <b>Gradual</b> resolve isso sem perder desempenho. Por isso ele é o recomendado.
        </p>
      </>
    ),
    confirm: "Desativar mesmo assim",
  },
};

/** Visual editor for %USERPROFILE%\.wslconfig, written for people who don't know WSL's internals. */
export function WslConfigDialog({ onClose, onSaved }: Props) {
  const [snapshot, setSnapshot] = useState<WslConfigSnapshot | null>(null);
  const [draft, setDraft] = useState<WslGlobalConfig | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState<"save" | "restart" | null>(null);
  const [confirmReclaim, setConfirmReclaim] = useState<string | null>(null);

  useEffect(() => {
    getWslConfig()
      .then((s) => {
        setSnapshot(s);
        setDraft(s.config);
      })
      .catch((err) => setError(errorMessage(err)));
  }, []);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape" || saving) return;
      e.preventDefault();
      e.stopPropagation();
      if (confirmReclaim) setConfirmReclaim(null);
      else onClose();
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [onClose, saving, confirmReclaim]);

  const set = <K extends keyof WslGlobalConfig>(key: K, value: WslGlobalConfig[K]) =>
    setDraft((d) => (d ? { ...d, [key]: value } : d));

  const chooseReclaim = (value: string | null) => {
    if (value && RECLAIM_WARNINGS[value] && value !== draft?.autoMemoryReclaim) setConfirmReclaim(value);
    else set("autoMemoryReclaim", value);
  };

  const save = async (restart: boolean) => {
    if (!draft) return;
    setSaving(restart ? "restart" : "save");
    setError(null);
    try {
      await saveWslConfig(draft, restart);
      onSaved(restart);
    } catch (err) {
      setError(errorMessage(err));
      setSaving(null);
    }
  };

  const hostGb = snapshot ? Math.max(1, Math.floor(snapshot.hostMemory / GIB)) : 1;
  const changed = snapshot && draft ? configChanged(snapshot.config, draft) : false;
  const mirrored = draft?.networkingMode === "mirrored";

  // Portaled: inside a screen's layout, spacing utilities (space-y-*) would shrink the backdrop.
  return createPortal(
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/70 p-6" onMouseDown={() => !saving && onClose()}>
      <div
        role="dialog"
        aria-modal
        aria-label="Limites do Linux"
        onMouseDown={(e) => e.stopPropagation()}
        className="relative flex h-[min(820px,100%)] w-[min(760px,100%)] flex-col overflow-hidden rounded-xl border border-zinc-800 bg-zinc-950 shadow-2xl shadow-black"
      >
        <header className="flex items-start gap-3.5 border-b border-zinc-800 bg-void px-6 py-5">
          <div className="flex size-10 shrink-0 items-center justify-center rounded-lg border border-zinc-800 bg-zinc-950">
            <SlidersHorizontal className="size-5 text-zinc-100" />
          </div>
          <div className="min-w-0 flex-1">
            <div className="text-[11px] font-medium tracking-[0.2em] text-zinc-500 uppercase">Configurações do WSL · .wslconfig</div>
            <h2 className="mt-0.5 text-lg font-semibold tracking-tight text-zinc-100">Limites do Linux</h2>
            <p className="mt-1 text-xs leading-relaxed text-zinc-500">
              Todas as distros rodam juntas numa mesma máquina virtual (a VM do WSL): estes ajustes valem para todas elas e só passam a
              valer quando o WSL reinicia.
            </p>
          </div>
          <button type="button" onClick={onClose} disabled={!!saving} aria-label="Fechar" className="rounded-md p-1.5 text-zinc-500 hover:bg-zinc-900 hover:text-zinc-100">
            <X className="size-4" />
          </button>
        </header>

        <div className="min-h-0 flex-1 space-y-7 overflow-y-auto px-6 py-5">
          {!snapshot || !draft ? (
            !error && <Loader2 className="mx-auto mt-10 size-5 animate-spin text-zinc-600" />
          ) : (
            <>
              <Section index="01" title="Memória e processador">
                <div className="space-y-2.5">
                  <Limit
                    label="Memória máxima"
                    description="O máximo de RAM do computador que o Linux pode ocupar. Um limite menor deixa o Windows mais folgado; se o Linux precisar de mais do que isso, ele fica mais lento (ou usa o Swap, abaixo)."
                    value={draft.memory === null ? null : Math.round(draft.memory / GIB)}
                    min={1}
                    max={hostGb}
                    fallback={Math.max(1, Math.floor(hostGb / 2))}
                    format={(v) => `${v} GB`}
                    defaultValue={formatBytes(snapshot.hostMemory / 2)}
                    defaultLabel="Padrão do WSL: metade da RAM do computador"
                    scale={`de ${hostGb} GB`}
                    onChange={(v) => set("memory", v === null ? null : v * GIB)}
                  />
                  <Limit
                    label="Núcleos do processador"
                    info={
                      <>
                        Conta os <b>núcleos lógicos</b>, os mesmos que o Gerenciador de Tarefas mostra: um processador de 8 núcleos com
                        Hyper-Threading aparece como 16. Este computador tem {snapshot.hostCpus}.
                      </>
                    }
                    description="Quantos núcleos o Linux pode usar ao mesmo tempo. Limitar deixa o Windows mais livre durante tarefas pesadas (builds, testes, Docker), que em troca demoram um pouco mais."
                    value={draft.processors}
                    min={1}
                    max={snapshot.hostCpus}
                    fallback={snapshot.hostCpus}
                    format={(v) => `${v} ${v === 1 ? "núcleo" : "núcleos"}`}
                    defaultValue={`${snapshot.hostCpus} núcleos`}
                    defaultLabel="Padrão do WSL: todos os núcleos do computador"
                    scale={`de ${snapshot.hostCpus}`}
                    onChange={(v) => set("processors", v)}
                  />
                  <Limit
                    label="Swap (memória reserva no disco)"
                    info={
                      <>
                        O Swap é um arquivo no disco que o Linux usa como memória extra quando a RAM acaba. Ele é bem mais lento que a RAM,
                        mas evita o pior: <b>sem Swap</b>, quando a memória acaba, o Linux fecha à força o programa que mais consome (o
                        build, o servidor de desenvolvimento, o banco de dados). Recomendado manter alguns GB.
                      </>
                    }
                    description="Uma reserva no disco para quando a memória acabar. Não deixa o Linux mais rápido, mas impede que programas sejam fechados de repente por falta de memória."
                    value={draft.swap === null ? null : Math.round(draft.swap / GIB)}
                    min={0}
                    max={Math.min(64, Math.max(8, hostGb))}
                    fallback={Math.max(1, Math.round(hostGb / 4))}
                    format={(v) => (v === 0 ? "Sem reserva" : `${v} GB`)}
                    defaultValue={formatBytes(snapshot.hostMemory / 4)}
                    defaultLabel="Padrão do WSL: 1/4 da RAM do computador. Arrastar até o 0 desliga a reserva."
                    scale={`de ${Math.min(64, Math.max(8, hostGb))} GB`}
                    onChange={(v) => set("swap", v === null ? null : v * GIB)}
                  />
                </div>
              </Section>

              <Section index="02" title="Devolver memória ao Windows">
                <Setting
                  description="Para ficar rápido, o Linux guarda na memória os arquivos que você usou há pouco (o cache). Esta opção decide quando essa memória volta para o Windows depois que o Linux fica parado."
                  info={
                    <>
                      É por causa do cache que o <span className="font-mono">vmmemWSL</span> parece enorme no Gerenciador de Tarefas. Nome
                      técnico: <span className="font-mono">autoMemoryReclaim</span>.
                    </>
                  }
                  value={draft.autoMemoryReclaim}
                  onChange={chooseReclaim}
                  options={[
                    {
                      value: "gradual",
                      label: "Gradual",
                      tag: "Recomendado",
                      help: "Com o Linux parado, o cache volta aos poucos para o Windows. Você recupera RAM sem perder desempenho perceptível.",
                    },
                    {
                      value: "dropcache",
                      label: "Imediato",
                      help: "Com o Linux parado, todo o cache é liberado de uma vez. A RAM volta mais rápido, mas o próximo build ou abertura de projeto fica mais lento.",
                    },
                    {
                      value: "disabled",
                      label: "Desativado",
                      help: "O cache nunca volta sozinho: o vmmemWSL pode segurar vários GB até você desligar o WSL ou usar Liberar cache.",
                    },
                  ]}
                  unset={{
                    help: "Nenhuma escolha salva: o WSL decide sozinho (nas versões recentes, ele usa o modo Imediato).",
                    reset: "Deixar o WSL decidir",
                  }}
                />
              </Section>

              <Section index="03" title="Rede">
                <div className="space-y-2.5">
                  <Setting
                    title="Como o Linux se conecta à rede"
                    info={
                      <>
                        Na dúvida, fique no <b>Isolado (NAT)</b>. Mude para o <b>Igual ao Windows</b> se a VPN do trabalho não funciona
                        dentro do Linux, ou se você quer abrir o projeto pelo celular ou outro computador. Nome técnico:{" "}
                        <span className="font-mono">networkingMode</span>.
                      </>
                    }
                    value={draft.networkingMode}
                    onChange={(v) => set("networkingMode", v)}
                    options={[
                      {
                        value: "nat",
                        label: "Isolado (NAT)",
                        isDefault: true,
                        help: "O Linux fica numa rede própria dentro do Windows. Seus projetos abrem normalmente em localhost no navegador do Windows, mas outros aparelhos (celular, outro PC) não os enxergam. Funciona em qualquer Windows.",
                      },
                      {
                        value: "mirrored",
                        label: "Igual ao Windows (Mirrored)",
                        help: "O Linux usa a mesma rede do Windows, como se fossem um computador só. Ajuda com VPN corporativa, IPv6 e para testar o site no celular pela mesma Wi-Fi. Precisa do Windows 11 (22H2 ou mais novo).",
                      },
                    ]}
                  />
                  <Setting
                    title="Abrir os projetos do Linux pelo localhost do Windows"
                    info={
                      <>
                        Nome técnico: <span className="font-mono">localhostForwarding</span>. Só vale no modo Isolado (NAT).
                      </>
                    }
                    disabled={
                      mirrored
                        ? "No modo Igual ao Windows isto não se aplica: o localhost já é o mesmo para o Windows e para o Linux."
                        : undefined
                    }
                    {...boolSetting(draft.localhostForwarding, true, (v) => set("localhostForwarding", v), {
                      on: "Quando um projeto roda no Linux (ex.: na porta 5173), ele abre em http://localhost:5173 no navegador do Windows. É o que quase todo mundo quer.",
                      off: "O navegador do Windows deixa de alcançar os projetos do Linux pelo localhost. Só faz sentido se você quer isolar o Linux de propósito.",
                    })}
                  />
                </div>
              </Section>

              <Section index="04" title="Disco e janelas">
                <div className="space-y-2.5">
                  <Setting
                    title="Disco do Linux que encolhe sozinho"
                    info={
                      <>
                        Todo o Linux (programas, arquivos e projetos) fica dentro de um único arquivo no Windows, o disco virtual
                        (<span className="font-mono">.vhdx</span>). Ele cresce quando você cria arquivos, mas normalmente não diminui quando
                        você apaga. Nome técnico: <span className="font-mono">sparseVhd</span>.
                      </>
                    }
                    {...boolSetting(draft.sparseVhd, false, (v) => set("sparseVhd", v), {
                      on: "Distros instaladas a partir de agora devolvem espaço ao Windows sozinhas quando você apaga arquivos no Linux. As que você já tem continuam como estão (para elas, use o Compactar VHDX do Dashboard).",
                      off: "O disco virtual só cresce. Para recuperar o espaço de arquivos apagados, use o Compactar VHDX do Dashboard de vez em quando.",
                    })}
                  />
                  <Setting
                    title="Programas do Linux com janela"
                    info={
                      <>
                        Recurso chamado WSLg. Nome técnico: <span className="font-mono">guiApplications</span>.
                      </>
                    }
                    {...boolSetting(draft.guiApplications, true, (v) => set("guiApplications", v), {
                      on: "Programas do Linux que têm interface (editores, navegadores, ferramentas visuais) abrem em janelas normais no Windows.",
                      off: "Só o terminal funciona: programas do Linux com janela não abrem. Economiza um pouco de memória se você nunca os usa.",
                    })}
                  />
                </div>
              </Section>

              {snapshot.preserved.length > 0 && (
                <p className="text-[11px] leading-relaxed text-zinc-500">
                  Outras configurações que já estão no seu arquivo continuam como estão:{" "}
                  <span className="font-mono text-zinc-400">{snapshot.preserved.join(", ")}</span>
                </p>
              )}
            </>
          )}
        </div>

        <footer className="flex items-center gap-3 border-t border-zinc-800 bg-void px-6 py-3.5">
          {error ? (
            <span className="flex min-w-0 flex-1 items-center gap-1.5 text-xs text-rose-300">
              <AlertTriangle className="size-3.5 shrink-0" />
              <span className="truncate" title={error}>
                {error}
              </span>
            </span>
          ) : (
            <span className="flex min-w-0 flex-1 items-center gap-1.5 text-[11px] leading-snug text-zinc-500">
              <span>
                <b className="font-medium text-zinc-300">Salvar</b>: vale na próxima vez que o WSL ligar.{" "}
                <b className="font-medium text-zinc-300">Salvar e reiniciar</b>: vale agora, mas fecha os terminais e para os projetos.
              </span>
              <InfoTip align="right">
                {snapshot?.exists
                  ? "Só as opções que você mudou são gravadas no arquivo; antes da primeira mudança, o app guarda uma cópia do original (.wslconfig.symbiosis.bak)."
                  : "O arquivo ainda não existe: o app cria só com as opções que você mudar."}{" "}
                Local: <span className="font-mono">{snapshot?.path}</span>
              </InfoTip>
            </span>
          )}
          <button type="button" onClick={() => save(false)} disabled={!changed || !!saving} className={`${BTN_GHOST} h-8 shrink-0`}>
            {saving === "save" ? <Loader2 className="size-3.5 animate-spin" /> : <Check className="size-3.5" />}
            Salvar
          </button>
          <button type="button" onClick={() => save(true)} disabled={!changed || !!saving} className={`${BTN_PRIMARY} h-8 shrink-0 px-4`}>
            {saving === "restart" ? <Loader2 className="size-3.5 animate-spin" /> : <RotateCw className="size-3.5" />}
            Salvar e reiniciar WSL
          </button>
        </footer>

        {confirmReclaim && RECLAIM_WARNINGS[confirmReclaim] && (
          <div className="absolute inset-0 z-10 flex items-center justify-center bg-black/60 p-6" onMouseDown={() => setConfirmReclaim(null)}>
            <div
              role="alertdialog"
              aria-modal
              aria-label={RECLAIM_WARNINGS[confirmReclaim].title}
              onMouseDown={(e) => e.stopPropagation()}
              className="w-[min(480px,100%)] overflow-hidden rounded-xl border border-zinc-700 bg-zinc-950 shadow-2xl shadow-black"
            >
              <div className="flex items-start gap-3 px-5 pt-5">
                <AlertTriangle className="mt-0.5 size-4 shrink-0 text-amber-400" />
                <div className="min-w-0">
                  <h3 className="text-sm font-semibold text-zinc-100">{RECLAIM_WARNINGS[confirmReclaim].title}</h3>
                  <div className="mt-2 space-y-2 text-xs leading-relaxed text-zinc-400 [&_b]:font-semibold [&_b]:text-zinc-200">
                    {RECLAIM_WARNINGS[confirmReclaim].body}
                  </div>
                </div>
              </div>
              <div className="mt-5 flex flex-wrap items-center justify-end gap-2 border-t border-zinc-800 bg-void px-5 py-3">
                <button
                  type="button"
                  onClick={() => {
                    set("autoMemoryReclaim", confirmReclaim);
                    setConfirmReclaim(null);
                  }}
                  className={`${BTN_GHOST} h-8`}
                >
                  {RECLAIM_WARNINGS[confirmReclaim].confirm}
                </button>
                <button
                  type="button"
                  autoFocus
                  onClick={() => {
                    set("autoMemoryReclaim", "gradual");
                    setConfirmReclaim(null);
                  }}
                  className={`${BTN_PRIMARY} h-8 px-4`}
                >
                  Usar Gradual (recomendado)
                </button>
              </div>
            </div>
          </div>
        )}
      </div>
    </div>,
    document.body,
  );
}

function Section({ index, title, children }: { index: string; title: string; children: ReactNode }) {
  return (
    <section>
      <div className="mb-2.5 flex items-baseline gap-2.5">
        <span className="font-mono text-[11px] text-zinc-600">{index}</span>
        <h3 className="text-[11px] font-semibold tracking-wider text-zinc-300 uppercase">{title}</h3>
      </div>
      {children}
    </section>
  );
}

/** A slider with a "WSL default" state (`null`): dragging it picks a value. */
function Limit(p: {
  label: string;
  description: string;
  info?: ReactNode;
  value: number | null;
  min: number;
  max: number;
  /** Where the thumb rests while on the default. */
  fallback: number;
  format: (v: number) => string;
  /** What WSL uses without the key, shown in the header while on the default ("7.8 GB"). */
  defaultValue: string;
  defaultLabel: string;
  scale: string;
  onChange: (v: number | null) => void;
}) {
  const isDefault = p.value === null;
  const shown = Math.min(p.max, Math.max(p.min, p.value ?? p.fallback));
  const pct = p.max > p.min ? ((shown - p.min) / (p.max - p.min)) * 100 : 100;
  return (
    <div className="rounded-lg border border-zinc-800 bg-zinc-900 px-4 py-3.5">
      <div className="flex items-center gap-2">
        <span className="text-sm font-medium text-zinc-100">{p.label}</span>
        {p.info && <InfoTip>{p.info}</InfoTip>}
        <span className={`ml-auto font-mono text-sm tabular-nums ${isDefault ? "text-zinc-400" : "text-zinc-100"}`}>
          {isDefault ? `Padrão (${p.defaultValue})` : p.format(shown)}
        </span>
        <span className="font-mono text-[11px] text-zinc-600">{p.scale}</span>
      </div>
      <p className="mt-1 text-xs leading-relaxed text-zinc-500">{p.description}</p>
      <div className="relative mt-3 h-4">
        <div className="absolute inset-x-0 top-1/2 h-1 -translate-y-1/2 rounded-full bg-zinc-800" />
        <div
          className={`absolute top-1/2 left-0 h-1 -translate-y-1/2 rounded-full ${isDefault ? "bg-zinc-600" : "bg-zinc-100"}`}
          style={{ width: `${pct}%` }}
        />
        <div
          className={`pointer-events-none absolute top-1/2 size-3.5 -translate-x-1/2 -translate-y-1/2 rounded-full border-2 border-zinc-950 ${
            isDefault ? "bg-zinc-500" : "bg-zinc-100"
          }`}
          style={{ left: `${pct}%` }}
        />
        <input
          type="range"
          aria-label={p.label}
          min={p.min}
          max={p.max}
          step={1}
          value={shown}
          onChange={(e) => p.onChange(Number(e.target.value))}
          className="absolute inset-0 w-full cursor-pointer opacity-0"
        />
      </div>
      <div className="mt-2 flex items-center gap-2 text-[11px]">
        <span className="text-zinc-500">{p.defaultLabel}</span>
        {!isDefault && (
          <button type="button" onClick={() => p.onChange(null)} className="ml-auto text-zinc-400 underline-offset-2 hover:text-zinc-100 hover:underline">
            Voltar ao padrão
          </button>
        )}
      </div>
    </div>
  );
}

interface Option {
  value: string;
  label: string;
  tag?: string;
  /** What changes for the user with this option selected. */
  help: string;
  /** WSL already behaves like this without the key: picking it removes the key from the file. */
  isDefault?: boolean;
}

/**
 * One setting: plain-language title/description, pill options and, under them, what the selected
 * option means in practice. The default option is labeled "(padrão)" instead of a separate pill.
 */
function Setting(p: {
  title?: string;
  description?: string;
  info?: ReactNode;
  value: string | null;
  onChange: (v: string | null) => void;
  options: Option[];
  /** For settings whose default varies by WSL version: no pill is "the default". */
  unset?: { help: string; reset: string };
  /** Shown instead of the options when the setting doesn't apply. */
  disabled?: string;
}) {
  const defaultOption = p.options.find((o) => o.isDefault);
  const current = p.value ?? defaultOption?.value ?? null;
  const selected = p.options.find((o) => o.value === current);
  return (
    <div className="rounded-lg border border-zinc-800 bg-zinc-900 px-4 py-3.5">
      {(p.title || p.description) && (
        <div className="mb-3">
          {p.title && (
            <div className="flex items-center gap-2">
              <span className="text-sm font-medium text-zinc-100">{p.title}</span>
              {p.info && <InfoTip>{p.info}</InfoTip>}
            </div>
          )}
          {p.description && (
            <p className={`text-xs leading-relaxed text-zinc-500 ${p.title ? "mt-1" : ""}`}>
              {p.description} {!p.title && p.info && <InfoTip>{p.info}</InfoTip>}
            </p>
          )}
        </div>
      )}
      {p.disabled ? (
        <p className="flex items-start gap-2 text-xs leading-relaxed text-zinc-500">
          <CornerDownRight className="mt-0.5 size-3.5 shrink-0 text-zinc-600" />
          {p.disabled}
        </p>
      ) : (
        <>
          <div className="inline-flex flex-wrap gap-1 rounded-full border border-zinc-800 bg-zinc-950 p-0.5" role="radiogroup">
            {p.options.map((o) => {
              const active = o.value === current;
              return (
                <button
                  key={o.value}
                  type="button"
                  role="radio"
                  aria-checked={active}
                  onClick={() => p.onChange(o.isDefault ? null : o.value)}
                  className={`inline-flex items-center gap-1.5 rounded-full px-3 py-1 text-xs whitespace-nowrap transition-colors ${
                    active ? "bg-zinc-100 font-semibold text-void" : "text-zinc-400 hover:text-zinc-100"
                  }`}
                >
                  {o.label}
                  {o.isDefault && <span className={active ? "font-normal text-zinc-600" : "text-zinc-600"}>(padrão)</span>}
                  {o.tag && <span className="text-[10px] font-medium tracking-wider text-zinc-500 uppercase">{o.tag}</span>}
                </button>
              );
            })}
          </div>
          <p className="mt-2.5 flex items-start gap-2 text-xs leading-relaxed text-zinc-400">
            <CornerDownRight className="mt-0.5 size-3.5 shrink-0 text-zinc-600" />
            <span className="flex-1">
              {selected ? selected.help : (p.unset?.help ?? `Valor escrito à mão no seu arquivo (${p.value}): continua como está até você escolher outro.`)}
            </span>
            {p.unset && p.value !== null && (
              <button
                type="button"
                onClick={() => p.onChange(null)}
                className="shrink-0 text-[11px] text-zinc-500 underline-offset-2 hover:text-zinc-100 hover:underline"
              >
                {p.unset.reset}
              </button>
            )}
          </p>
        </>
      )}
    </div>
  );
}

/** A yes/no setting as two named options, with WSL's default marked and stored as "no key". */
function boolSetting(
  value: boolean | null,
  wslDefault: boolean,
  onChange: (v: boolean | null) => void,
  help: { on: string; off: string },
) {
  return {
    value: value === null ? null : String(value),
    onChange: (v: string | null) => onChange(v === null ? null : v === "true"),
    options: [
      { value: "true", label: "Ligado", isDefault: wslDefault, help: help.on },
      { value: "false", label: "Desligado", isDefault: !wslDefault, help: help.off },
    ],
  };
}
