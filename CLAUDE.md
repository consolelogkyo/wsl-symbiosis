# WSL Symbiosis — Regras de Ouro

Aplicativo desktop nativo para Windows que cria uma simbiose visual completa entre o Windows e o WSL2.
Estas regras valem para **toda** interação futura com o código. Leia antes de alterar qualquer arquivo.

## Stack

| Camada    | Tecnologia                                                         |
| --------- | ------------------------------------------------------------------ |
| Shell     | **Tauri v2** (backend em Rust, `src-tauri/`)                       |
| UI        | **React 19** + **TypeScript** + **Vite** (`src/`)                  |
| Estilo    | **Tailwind CSS v4** (via `@tailwindcss/vite`, sem `tailwind.config`) |
| Ícones    | **lucide-react**                                                   |
| Terminal  | **@xterm/xterm** + `@xterm/addon-fit`                              |
| Async Rust| `tokio` (runtime do próprio Tauri)                                 |

Não introduza outras libs de UI (MUI, Chakra, shadcn com Radix pesado etc.) sem necessidade explícita.

## Comandos

```bash
npm run tauri dev        # app completo em modo dev (Vite na porta 1420)
npm run build            # tsc + vite build (frontend apenas)
cd src-tauri && cargo check
```

Antes de finalizar qualquer tarefa: `cargo check` **e** `npm run build` precisam passar com zero erros.

## Estrutura

```
src/                      # frontend React
  lib/wsl.ts              # tipos + wrappers de invoke() para os comandos Tauri
  lib/format.ts           # formatBytes, percent, level() → cores emerald/amber/rose
  lib/explorer.ts         # tipos + invoke() do Explorador e helpers de caminho (espelham explorer.rs)
  lib/packages.ts         # tipos + invoke() de Pacotes & Ambiente, validação de package spec
  lib/terminal.ts         # invoke() do PTY, roteador único de eventos (com buffer), SNIPPETS
  lib/settings.ts         # get/save_settings, check_tool_path (espelham settings.rs)
  lib/shortcuts.ts        # atalhos configuráveis: defaults, parse/match de teclado e mouse, fixos
  lib/lockdown.ts         # webview como app nativo: sem menu de contexto nem atalhos do navegador
                          # (reload, imprimir, ver código, devtools, voltar); o terminal fica de fora
  lib/ui.ts               # receitas de classe Venom: BTN_PRIMARY/GHOST/DANGER, CARD, LABEL, FIELD
  lib/brands.ts           # logos oficiais (simple-icons, embutidos) por ferramenta/stack
  hooks/useWsl.ts         # estado global de distros (polling 5s) + ações start/shutdown + distro padrão
  hooks/useMetrics.ts     # telemetria no App (3s no Dashboard, 5s fora; alimenta o painel AO VIVO)
  hooks/useExplorer.ts    # uma aba do Explorador: pasta atual, histórico, ocultos
  hooks/useTools.ts       # detect_tools compartilhado; reloadTools() após salvar caminhos
  hooks/useEnvironment.ts # scan_environment com cache por distro; portas atualizadas à parte
  hooks/useActivity.ts    # tarefas (eventos) + portas em escuta (5s) + toasts; ActivityContext no App
  lib/projects.ts         # invoke() do runner de projetos (espelha projects.rs)
  lib/git.ts              # invoke() de branches/switch/fetch/pull (espelha git.rs), formatStat
  lib/menu.ts             # placeMenu + closeOnOutside dos menus flutuantes (scripts, branches)
  hooks/useGitQueue.ts    # fila de git pull (1 repo por vez), GitQueueContext no App, toast no fim
  lib/updater.ts          # check/download/install do plugin updater, mensagens de erro em pt-BR
  hooks/useUpdater.ts     # estado do auto-update (UpdaterContext no App): checagem silenciosa, progresso
  components/UpdateDialog.tsx  # novidades + Baixar e instalar (pill na Titlebar, card nas Configurações)
  lib/setup.ts            # tipos/invoke do provisioner.rs, MODULE_DEFS, assessModule (estado de cada módulo)
  views/SetupWizardView.tsx    # setup Zero-to-Hero que toma a janela inteira (5 etapas, progresso ao vivo)
  components/ToolSettings.tsx  # IdeGrid + ToolPathList (Configurações e setup); hooks/useToolPaths.ts
  components/             # Titlebar (janela sem bordas), Sidebar (+ AO VIVO), Logo, DistroSelect,
                          # StatusDot, SettingsDialog (abas Sistema & IDEs / Git / Atalhos; o 1º uso agora é o setup),
                          # ShortcutsSettings (gravador de atalhos, gestos de mouse, fixos),
                          # InfoTip (tooltip "i"), BrandIcon (logo oficial embutido),
                          # RunningPanel (sidebar EM EXECUÇÃO), Toaster, TaskLogDrawer,
                          # GitQueuePanel (progresso da fila de pull, empilhado com os toasts)
  components/explorer/GitBranchChip.tsx   # chip da branch (substitui o selo GIT) + menu de troca/pull
  components/explorer/ProjectActions.tsx  # Rodar ▾ / Parar / Instalar / limpar, por linha
  components/dashboard/   # MeterCard (barra segmentada), ProcessTable (busca + kill)
  components/explorer/    # ExplorerPane (1 aba), ExplorerToolbar, EntryRow, OpenInButtons, ProjectBadges, FileEditor
  components/packages/    # ToolGrid, NvmCard, NpmGlobals, PortsTable
  components/terminal/    # TerminalPane (1 xterm + 1 sessão PTY por aba)
  views/index.ts          # NAV_ITEMS — as 4 telas (dashboard, explorer, packages, terminal)
  views/*View.tsx         # uma tela por item de navegação
src-tauri/src/
  lib.rs                  # registro de comandos/estado do Tauri
  wsl.rs                  # helpers de wsl.exe (wsl_command, run, sh_in_distro, is_running) + distros
  metrics.rs              # telemetria host (sysinfo + NtQuerySystemInformation) e guest (/proc, df, ps)
  explorer.rs             # arquivos (listar/criar/renomear/excluir/ler/gravar) + abrir em IDEs
  packages.rs             # inventário de ferramentas, nvm, npm globais, portas em escuta
  terminal.rs             # sessões PTY (ConPTY via portable-pty) → eventos terminal-output/exit
  settings.rs             # settings.json em %APPDATA%\com.wslsymbiosis.app (caminhos de IDEs, distro/IDE padrão, atalhos)
  projects.rs             # rodar/parar projetos Node e instalar node_modules (ProjectRunner, evento project-task)
  git.rs                  # branches, git switch, fetch e pull --ff-only (mensagens de erro em pt-BR)
  updater.rs              # prepare_for_update: fecha terminais e para projetos antes do instalador
  provisioner.rs          # setup: inspeção, habilitar WSL, instalar Ubuntu, módulos (evento provision-progress)
scripts/build-release.mjs # npm run release:build → instalador assinado + latest.json em dist-release/
src-tauri/icons/
  app-icon.svg            # fonte do ícone; regenerar com `npx tauri icon src-tauri/icons/app-icon.svg`
                          # e depois `cargo clean -p wsl-symbiosis --release` (o build script não
                          # rastreia o icon.ico: sem isso o .exe continua com o ícone antigo)
src-tauri/examples/
  pty_no_window.rs        # binário GUI que prova que o PTY não abre janelas de console
```

## Regras críticas — Windows / Rust

### 1. NUNCA piscar janelas de console
Todo `std::process::Command` ou `tokio::process::Command` que chamar `wsl.exe`, `powershell.exe`
(ou qualquer executável de console) **DEVE** usar `CREATE_NO_WINDOW`:

```rust
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

// std::process::Command
use std::os::windows::process::CommandExt;
std::process::Command::new("wsl.exe").creation_flags(CREATE_NO_WINDOW);

// tokio::process::Command (método inerente no Windows)
tokio::process::Command::new("wsl.exe").creation_flags(CREATE_NO_WINDOW);
```

Não crie `Command::new("wsl.exe")` solto: use o helper `wsl::wsl_command()` em `src-tauri/src/wsl.rs`,
que já aplica a flag e o `WSL_UTF8=1`.

### 2. Encoding do WSL (UTF-16LE)
`wsl.exe --list --verbose` (e outros subcomandos de gerenciamento) escrevem no stdout em **UTF-16LE**,
com bytes nulos intercalados. Regras:
- Sempre definir `WSL_UTF8=1` no ambiente do processo antes de executar.
- Mesmo assim, decodificar com `wsl::decode_output()`, que detecta UTF-16LE (BOM ou bytes nulos)
  e faz fallback para UTF-8 — versões antigas do WSL ignoram `WSL_UTF8`.
- A saída é **localizada** conforme o idioma do Windows (ex.: `Versão do WSL:`). Nunca dependa
  de rótulos traduzidos; faça parse por posição/forma (ex.: último token = versão).

### 3. Shell Linux com ambiente do usuário
Para ler ferramentas do usuário (nvm, node, yarn, pyenv, cargo...), rode no **shell de login do
usuário** em modo interativo, e não em `bash -ic` fixo: o shell padrão do usuário é **zsh**, e o nvm
é carregado no `~/.zshrc`, que o `bash -ic` nunca lê. Use `packages::user_shell_command` (LAUNCHER):
`zsh -ic`/`bash -ic` conforme `/etc/passwd`, senão `bash -ic`, senão `sh -c` (busybox).
- rc files imprimem lixo (powerline, oh-my-zsh): delimite a saída com `@@section`; para ações, use
  `run_action` (`@@out ... @@status N`).
- O script precisa rodar igual em **zsh e sh**: nada de `for x in $VAR` (o zsh não faz word
  splitting) e `exit` dentro de pipeline só em `( )` (o zsh roda o último elemento do pipeline
  no shell atual, então `{ ...; exit; }` derruba o shell inteiro).
- O PRELUDE remove do PATH os drives do Windows, identificados pelo tipo de montagem (9p
  `aname=drvfs`; são `/mnt/c` no Ubuntu e `/mnt/host/c` no docker-desktop). Isso evita ferramentas
  do Windows (o npm do Windows instalaria pacotes no Windows) e varreduras lentas pelo 9P
  (3,4 s → 0,1 s).

Comandos que não dependem do ambiente do usuário podem usar `wsl.exe -d <distro> --exec <bin>`.
Passe argumentos como args separados (`.arg()`), nunca concatenando input do usuário em string de shell
sem escapar. Nomes de distro vindos do frontend devem ser validados (`wsl::validate_distro_name`).

### 3.1 Nunca ligar uma distro por efeito colateral
Qualquer `wsl.exe -d <distro> ...` **inicia** a distro se ela estiver parada. Comandos de leitura
(métricas, listagens) devem checar `wsl::is_running()` antes e devolver "sem dados" quando parada.

### 3.2 Scripts portáveis (procps **e** busybox)
O usuário tem `Ubuntu` (procps) e `docker-desktop` (busybox). Scripts dentro da distro:
- Use `wsl::sh_in_distro(distro, user, script, args)` → `--exec sh -c <script> sh <args...>`.
  Valores entram como `$1`, `$2`; nunca interpole input no texto do script.
- Prefira `/proc/*` a ferramentas; `df -kP`; detecte o `ps` (`ps --version | grep procps`).
  busybox não tem `%CPU` e abrevia tamanhos (`12m`, `2g`).
- Sempre `"$1"` (com aspas) para valores que podem ter espaços; o escape do Rust na linha de
  comando do Windows preserva as aspas até o `sh` (ver 3.4).

### 3.3 Métricas da VM
- Memória, CPU e load do guest vêm do **kernel único da VM WSL2**: são iguais para todas as distros.
  Disco (`/`) e processos são por distro. `drop_caches` afeta a VM inteira.
- `vmmemWSL` é um processo protegido: `OpenProcess` falha sem admin (sysinfo devolve 0).
  Memória e CPU dele vêm de `NtQuerySystemInformation` (`metrics::nt`).

### 3.4 Arquivos da distro (Explorador)
- Caminhos do frontend passam por `explorer::normalize_linux_path` (absoluto, sem controle,
  `.`/`..` resolvidos) e nomes novos por `validate_entry_name`.
- Exclusão/renomeação recusam `explorer::is_protected` (`/`, todo diretório de 1º nível,
  `/home/<user>`, `/mnt/<drive>`). Operações rodam como o usuário padrão, nunca root.
- Leitura usa `wsl::run_raw` (bytes crus) para detectar binário; gravação manda o conteúdo
  por stdin (`cat > "$1"`) após converter CRLF → LF.
- Aspas duplas em scripts `sh -c` funcionam (testado com nomes contendo `' " $ ; \ ç`).
- **Abas** (v0.1.1): `ExplorerView` hospeda as abas; cada uma é um `ExplorerPane` com seu próprio
  `useExplorer`. As abas são **por distro** (a barra mostra as da distro selecionada; trocar e
  voltar restaura) e cada distro mantém ao menos uma. Todas ficam montadas (ocultas), e o próprio
  `ExplorerView` fica montado no `App` após a 1ª visita, como o Terminal. "Nova aba" abre na
  pasta atual; o gesto de "abrir em nova aba" abre em segundo plano. Ponto emerald na aba =
  projeto rodando naquela pasta.
- **Atalhos configuráveis** (`lib/shortcuts.ts`, Configurações → Atalhos): o `settings.json` guarda
  só o que difere do padrão (`""` = desativado), assim mudanças futuras de padrão chegam ao
  usuário. Teclado: `"Ctrl+Shift+Tab"` (Ctrl, Alt, Shift; letras pelo `e.code`, igual em ABNT2 e
  US); exige Ctrl/Alt ou F1–F12 e recusa combinações reservadas (Ctrl+C/V/X/A/Z/Y/S). Mouse:
  gestos `ctrl,shift,alt,middle`. Conflitos de teclado bloqueiam o Salvar. Atalhos são ignorados
  com `[aria-modal]` aberto (o `FileEditor` é um diálogo: fechar a aba descartaria edições).
  Nada de atalhos de aba no Terminal: Ctrl+T/Ctrl+W são do shell (fzf, apagar palavra).

### 3.5 Abrir em ferramentas do Windows
- Resolva IDEs pelo caminho de instalação, **não pelo PATH**: nesta máquina `code` no PATH é o
  shim do Cursor. Ordem/locais em `explorer::Tool::launcher`.
- `.cmd` são executados pelo `std` do Rust (escapa args de batch); sempre via `wsl::hidden_command`.
- IDEs: `<cli> --remote wsl+<distro> <linux_path>`. Explorer: `\\wsl.localhost\<distro>\...`.
- Caminhos personalizados (`settings.rs`, tela de Diagnóstico) têm prioridade: `Tool::launcher(custom)`
  usa o caminho salvo se o arquivo ainda existir, senão cai na detecção automática (`auto_launcher`).
  Explorer não aceita caminho personalizado. `normalize_tool_path` tira aspas, expande `%VAR%` e
  exige `.exe/.cmd/.bat` existente.
- `defaultIde` (settings) define o botão em destaque. Sem ele, vale a primeira IDE instalada
  (`primaryIde` em `OpenInButtons.tsx`).
- Ícone oficial: `icon_source` usa o próprio `.exe`, ou, para shims `.cmd`, o maior `.exe` até 4
  pastas acima (ignora `unins*`; o `code-tunnel.exe` é menor que o `Code.exe`). Aliases de 0 byte
  (`WindowsApps\wt.exe`) não têm ícone.

### 3.6 Terminal (PTY)
- **Única exceção à regra 1**: o `wsl.exe` interativo roda dentro de um pseudoconsole (ConPTY via
  `portable-pty`), que o anexa ao PTY; ele nunca ganha janela própria. Provado pelo exemplo
  `pty_no_window` (binário GUI, 3 sessões, 0 janelas novas). Não use CREATE_NO_WINDOW ali.
- O terminal **pode** iniciar uma distro parada (o usuário pediu um shell); sondas em background não.
- O ConPTY só entrega EOF ao leitor depois que o pseudoconsole é fechado: o thread "waiter" espera o
  filho, aguarda o leitor drenar, derruba a sessão e só então emite `terminal-exit`.
- O ConPTY pede a posição do cursor (`ESC[6n`) e espera resposta: o xterm.js responde sozinho; em
  testes, responda `ESC[1;1R`.
- Frontend: o `TerminalView` fica montado (oculto com `hidden`) depois da 1ª visita, e cada aba
  mantém seu xterm. Ao reexibir, chame `fit()` **e** `term.refresh()` (linhas renderizadas ocultas
  usam métricas erradas). Efeitos de "abrir aba" precisam ser idempotentes (StrictMode roda 2×).

### 3.7 Projetos (rodar / parar / instalar)
- A tarefa roda no shell de login do usuário (nvm!) via `packages::user_shell_command`: o script
  imprime `@@pid $$` e faz `exec <pm> run <script>`, então esse PID é a raiz da árvore no Linux.
- **O WSL mata os processos de um `wsl.exe` quando ele termina** (até com `setsid`). Por isso o
  `wsl.exe` portador fica vivo durante toda a tarefa. Matar só o `wsl.exe` também derruba a árvore,
  mas o caminho limpo é o `KILL_TREE_SCRIPT`: percorre `/proc/<pid>/task/*/children`, manda TERM
  e, ~5 s depois, KILL no que sobrou.
- Gerenciador de pacotes: `packageManager` do package.json > lockfile (pnpm/yarn/bun) > npm.
  Script padrão: dev > start:dev > develop > serve > start. Nomes de script são validados e vão
  como argumento posicional.
- Ao sair do app (`RunEvent::Exit`), `stop_all_blocking` para tudo; antes do `wsl --shutdown`,
  `mark_all_stopping` faz as tarefas aparecerem como "paradas" e não como "falhou".
- Portas: o `PORTS_SCRIPT` (root) também traz usuário, `cwd` e comando de cada PID. Uma porta
  pertence a um projeto quando o `cwd` do processo está dentro da pasta. `isUserPort` ignora
  daemons (cwd `/` ou root).

### 3.7.1 Git (branch, troca, pull em lote)
- A branch da listagem vem de **ler `.git/HEAD`** no `list_script` (só builtins; `.git` pode ser
  um arquivo `gitdir:` em worktrees/submódulos). Nada de rodar `git` por linha.
- Listar branches roda em `sh` simples (rápido). `switch`, `fetch` e `pull` rodam no shell de
  login (`packages::run_user_script`): hooks como o post-merge do husky chamam `npm install` e
  precisam do nvm.
- **Nada pode pedir input** (não há terminal): `GIT_TERMINAL_PROMPT=0`, `GCM_INTERACTIVE=never` e
  `ssh -o BatchMode=yes`, a menos que o usuário tenha `GIT_SSH_COMMAND`/`core.sshCommand`.
  `LC_MESSAGES=C` deixa as mensagens em inglês para o `explain_error` traduzir (1ª linha =
  explicação em pt-BR, resto = saída do git).
- Remotos HTTPS sem credencial: `settings.gitAuth` (Configurações → Git). `auto` (padrão) tenta o
  remoto e, se pedir usuário/senha, repete via SSH; `ssh` sempre SSH; `remote` nunca troca. A troca
  é só no comando (`git -c url.git@host:.insteadOf=https://host/`, função `net` do script): o
  remoto salvo no repositório **nunca** é alterado. `@@via ssh` → `PullResult.viaSsh`.
- **Clone** (`git_clone`, `CloneDialog`): só links de rede (`https/http/ssh/git://` ou
  `user@host:path`; nunca caminho local, `file://` ou `ext::`), validados em `validate_clone_url`
  e espelhados em `isCloneUrl`. A pasta segue o `git clone` (`repo_name_from_url`) ou o nome
  digitado (`validate_entry_name`). Usa o mesmo `net` (fallback SSH); o clone guarda o link
  digitado como `origin`. Clone que falha não deixa pasta. "Continuar em segundo plano" fecha a
  janela e avisa por toast no fim.
- Pull é sempre `--ff-only --no-rebase`: nunca cria merge nem conflito; branch divergida, sem
  upstream ou HEAD destacado viram erro explicado. Nomes de branch passam por
  `validate_branch_name` (sem `-` inicial, `..`, `@{`, espaços...).
- A fila (`useGitQueue`) fica no `App`: um repo por vez, continua ao trocar de tela, toast de
  resumo no fim. A seleção do lote é lembrada por pasta (`localStorage`, conveniência local).
- Teste real: `WSL_TEST_DISTRO=Ubuntu cargo test --lib live_git -- --ignored` (repo bare em /tmp);
  com `WSL_TEST_HTTPS_REPO=https://github.com/<org>/<privado>.git` testa também o fallback SSH.

### 3.7.2 Auto-update (GitHub Releases)
- `tauri-plugin-updater` lê `https://github.com/consolelogkyo/wsl-symbiosis/releases/latest/download/latest.json`
  (`plugins.updater` no `tauri.conf.json`, com a chave pública). Repositório precisa ser público.
- **Chave privada + senha em `.tauri-keys/` (fora do git).** Perder qualquer uma das duas = as
  instalações existentes nunca mais aceitam update (só aceitam a chave pública embutida).
- Release: subir a versão nos 3 arquivos (package.json, tauri.conf.json, Cargo.toml), escrever a
  seção `## X.Y.Z` do `docs/CHANGELOG.md` (vira as notas da release), `npm run release:build`,
  anexar o `.exe` e o `latest.json` de `dist-release/` na release `vX.Y.Z` (não pre-release).
- O asset publicado não tem espaços (o GitHub troca espaço por ponto e a URL do latest.json quebraria).
- A checagem automática é silenciosa (offline / sem release = nada na tela); erro só aparece em
  ação do usuário. No Windows o `install()` sai do app com `process::exit` (pula `RunEvent::Exit`),
  por isso o frontend chama `prepare_for_update` antes.

### 3.7.3 Setup Zero-to-Hero (provisioner.rs, SetupWizardView)
- 1º uso (`!onboardingCompleted`) ou `wslRebootPending` → `App` entra em `setupMode`: o setup **toma a
  janela inteira** (barra própria com WindowControls; sem Titlebar, Sidebar nem módulos), como o
  onboarding de outras IDEs. 1ª etapa = Diagnóstico (só leitura). Reabrir: Configurações → 06.
- **Quando abre:** `should_open_setup` → `setup_required` (testada): `!onboardingCompleted` ou
  `!setupWizardSeen` (quem vem da 0.1.2 vê uma vez) ou `wslRebootPending` ou nenhuma distro de
  desenvolvimento. `save_settings` nunca limpa `setupWizardSeen`.
- **Distros de sistema** (`is_system_distro` / `isSystemDistro`: docker-desktop*, rancher-desktop*,
  podman-machine*) nunca são alvo do setup nem dos módulos, e o `useWsl` (`preferredDistro`) não as
  escolhe como padrão.
- `inspect_setup_state` **nunca liga a distro**: parada → `linux: null` e a UI oferece "Ligar e analisar".
  Os módulos (ação do usuário) podem ligar.
- Edições de config são funções puras testadas (`set_zsh_theme`, `ensure_plugins`,
  `ensure_completions_fpath`, `ensure_omz_loaded`, `set_wsl_conf_default_user`, `apply_dracula`),
  nunca sed. Backup `<arquivo>.symbiosis.bak` só na 1ª edição (guarda o original).
- `zsh-completions` vai no `fpath` antes do `source $ZSH/oh-my-zsh.sh`, **não** em `plugins=()`
  (README do projeto: como plugin quebra o cache do compinit). `zsh-syntax-highlighting` sempre por último.
- Windows Terminal: `settings.json` é JSONC (comentários/vírgulas finais → `strip_jsonc`); só é
  regravado se algo mudou (comentários se perdem nessa regravação; há backup). Há dois perfis
  "Ubuntu" (pacote da Store visível + WSL oculto): `find_distro_profile` prefere o visível.
  `serde_json` com `preserve_order` mantém a ordem das chaves do usuário.
- Senha do Linux: só por stdin para `chpasswd`; nunca em argumento, log ou settings.
- `enable_wsl_engine`: script .ps1 temporário elevado (`Start-Process -Verb RunAs`), códigos do DISM
  num arquivo (3010 = reiniciar) → `settings.wslRebootPending` (o `save_settings` preserva esse campo).
- Scripts Linux usam PATH só Linux (`LINUX_PATH`), senão o `git.exe` do Windows apareceria.
- Teste real só-leitura: `WSL_TEST_DISTRO=Ubuntu cargo test --lib live_setup -- --ignored`
  (o patch do .zshrc roda num HOME descartável em /tmp).
- E2E numa distro **descartável** (recusa se já existir; remove só a que criou):
  `WSL_E2E_DISTRO=SymbiosisE2E WSL_E2E_ROOTFS=<ubuntu-noble-wsl-amd64-wsl.rootfs.tar.gz>
  cargo test --lib live_setup_e2e -- --ignored --nocapture` (rootfs oficial de
  cloud-images.ubuntu.com/wsl; confira o SHA256).
- Lições do E2E: o catálogo de `wsl --install` muda com a versão do WSL (`list_online_distros`, nada
  de nomes fixos); o instalador só oferece Ubuntu (o Debian do catálogo pode ser EOL com mirrors 404);
  `apt-get update` com um repositório morto não pode abortar; cada módulo garante seu git/curl
  (`ensure_packages`); pacotes antigos da Store só registram a distro com `<distro>.exe install --root`.

### 3.8 Memória do WSL no Windows
- Uma única chamada `NtQuerySystemInformation` por amostra, **sem cache de PID**, porque a VM nasce
  e morre com o `wsl --shutdown`. VM = `vmmemWSL` (ou `vmmem`); os auxiliares (`wslservice`,
  `wslhost`, `wslrelay`, `wsl.exe`) são somados à parte. Nunca use um auxiliar como "a VM".
- Memória = **private working set** (coluna "Memória" do Gerenciador de Tarefas).

### 4. Comandos Tauri
- Todo comando que toca processos externos é `async` e retorna `Result<T, String>`.
- Aplique timeout (`tokio::time::timeout`) — o serviço do WSL pode travar.
- Structs serializadas para o frontend usam `#[serde(rename_all = "camelCase")]`;
  os tipos espelhados ficam em `src/lib/wsl.ts`.

## Design System

**Symbiote / Venom Edition** (Fase 6): preto abissal + branco puro de alto contraste, layout de
utilitário de hardware. Zero gradientes, glows ou acentos coloridos decorativos.

- A escala `zinc` foi **reajustada** em `src/index.css` (`@theme`) para os neutros Venom; use as
  classes zinc normalmente: `950` #09090c conteúdo · `900` #0d0d11 cards · `800` #1a1a20 bordas ·
  `500` #6e6e7a rótulos · `100` #f4f4f6 texto. `bg-void` (#050507) é a moldura (titlebar, sidebar).
- Cards: `bg-zinc-900` + `border-zinc-800` (1px seco), `rounded-lg`, sem sombra.
- Destaque/seleção/primário: **branco** — pill `bg-zinc-100 text-void` (`BTN_PRIMARY`) ou linha
  indicadora branca de 2px (nav ativo, aba ativa). Secundário: pill com borda (`BTN_GHOST`).
- Use as receitas de `src/lib/ui.ts` em vez de reinventar classes de botão/card/campo.
- Rótulos: `text-[11px]` uppercase `tracking-wider` `text-zinc-500` (`LABEL`).
- Cores semânticas **só** em pontos de status (`size-1.5`) e alertas críticos: emerald (ok/running),
  amber (atenção), rose (crítico/destrutivo; destrutivo só aparece no hover ou na confirmação).
- **Exceção intencional, a cor como informação:** no Explorador, as stacks têm cor própria
  (`ProjectBadges.tsx`: chip e ícone da pasta; **sem** tom de fundo na linha). Também têm cor as
  ações de projeto: limpeza amber, Instalar sky, Rodar emerald, Parar rose. Os logos oficiais
  aparecem nas cores das marcas.
- Logos de ferramentas: `simple-icons` (CC0), importados nomeadamente em `src/lib/brands.ts` e
  embutidos no bundle. **Nunca** buscar ícones na rede (o app precisa funcionar offline).
  Use `<BrandIcon>`; `readableBrandColor` clareia marcas pretas (Rust, Bun, Deno).
- Ícones das IDEs: extraídos dos executáveis instalados (`get_tool_icons`, uma execução do
  PowerShell com `ExtractAssociatedIcon`, com cache). Use `<ToolIcon>`; sem ícone, cai no lucide.
  O Explorer fica de fora de propósito: mantém a pasta branca genérica (menos ruído visual).
- Textos auxiliares longos vão em `<InfoTip>` (ícone "i" com tooltip), não fixos na tela.
- Responsividade por **container query** (`@container` + `@max-5xl:` etc.), não por viewport: a
  linha do Explorador abaixo de 64rem esconde as tags, os chips de limpeza/Instalar e o rótulo
  da IDE padrão (vira bolinha branca só com ícone). Barras com rótulos usam `whitespace-nowrap`.
- Menus flutuantes (portal) abrem para o lado com mais espaço e rolam por dentro: o webview não
  desenha fora da janela (ver `placeMenu` em `ProjectActions.tsx`).
- Versão ativa = pill branca; as demais = cinza apagado. Verde nunca indica "selecionado".
- Medidores: barra branca/cromo segmentada (`.meter-cells`); `LEVEL_DOT` ao lado do valor indica o nível.
- Janela sem bordas: `Titlebar.tsx` usa `data-tauri-drag-region` em todo elemento passivo
  (o atributo não vale para filhos) e os botões de janela precisam das permissões `core:window:*`.
- Tipografia: Inter / system-ui; números, caminhos e terminal em JetBrains Mono / Cascadia Code.
- Ícones lucide em `size-4` (16px); transições curtas (`transition-colors`); nada chamativo.
