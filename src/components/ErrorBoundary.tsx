import { Component, type ErrorInfo, type ReactNode } from "react";
import { AlertTriangle, RotateCw } from "lucide-react";
import { BTN_PRIMARY } from "../lib/ui";

interface Props {
  /** Changing it (another screen) clears the error. */
  resetKey: string;
  children: ReactNode;
}

interface State {
  error: Error | null;
  key: string;
}

/** A screen that throws while rendering shows this instead of taking the whole window down
 *  (React unmounts everything on an uncaught render error: the window would go black). */
export class ErrorBoundary extends Component<Props, State> {
  state: State = { error: null, key: this.props.resetKey };

  static getDerivedStateFromError(error: Error): Partial<State> {
    return { error };
  }

  static getDerivedStateFromProps(props: Props, state: State): Partial<State> | null {
    return props.resetKey !== state.key ? { error: null, key: props.resetKey } : null;
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    console.error("[ErrorBoundary]", error, info.componentStack);
  }

  render() {
    const { error } = this.state;
    if (!error) return this.props.children;
    return (
      <div className="flex h-full flex-col items-center justify-center gap-3 p-8 text-center">
        <AlertTriangle className="size-5 text-rose-400" />
        <p className="text-sm font-medium text-zinc-100">Algo deu errado nesta tela</p>
        <p className="max-w-md text-xs leading-relaxed text-zinc-500">
          O resto do app continua funcionando. Tente de novo; se repetir, mande o detalhe abaixo para o suporte.
        </p>
        <pre className="max-w-xl overflow-auto rounded-md border border-zinc-800 bg-void px-3 py-2 text-left font-mono text-[11px] whitespace-pre-wrap text-rose-300/90">
          {error.message}
        </pre>
        <button type="button" onClick={() => this.setState({ error: null })} className={`${BTN_PRIMARY} mt-1`}>
          <RotateCw className="size-3.5" />
          Tentar de novo
        </button>
      </div>
    );
  }
}
