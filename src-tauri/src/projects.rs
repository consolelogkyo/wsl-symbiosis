//! Run / stop Node projects from the Explorer and install their dependencies.
//!
//! A task is `<pm> run <script>` or `<pm> install` started in the user's login shell (so nvm's
//! Node is on PATH), inside the project folder. The shell prints its PID and `exec`s the
//! package manager, so that PID is the root of the project's process tree in Linux. Stopping
//! signals that whole tree (dev servers spawn watchers, workers, esbuild...), then kills the
//! hidden wsl.exe that carried it.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

/// Where task updates go: the `project-task` event in the app, a no-op/collector in tests.
pub type TaskSink = Arc<dyn Fn(&TaskInfo) + Send + Sync>;
use tokio::io::{AsyncRead, BufReader};
use tokio::sync::oneshot;

use crate::explorer::normalize_linux_path;
use crate::lines::{BoundedLines, MAX_LOG_LINE_BYTES};
use crate::packages::{strip_ansi, user_shell_command};
use crate::wsl::{ensure_running, run, sh_in_distro, validate_distro_name};

const INSPECT_TIMEOUT: Duration = Duration::from_secs(15);
const STOP_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_TASKS: usize = 12;
const MAX_LOG_LINES: usize = 1500;
const MAX_URLS: usize = 6;
/// Scripts tried, in order, when the user just clicks "Rodar".
const PREFERRED_SCRIPTS: [&str; 5] = ["dev", "start:dev", "develop", "serve", "start"];

// ---------------------------------------------------------------------------
// Types (mirrored in src/lib/projects.ts)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ProjectScript {
    pub name: String,
    pub command: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ProjectInfo {
    /// npm, pnpm, yarn or bun.
    pub package_manager: String,
    pub scripts: Vec<ProjectScript>,
    /// What "Rodar" runs: dev > start:dev > develop > serve > start.
    pub suggested: Option<String>,
    pub has_node_modules: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskKind {
    Run,
    Install,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    /// Spawned; the login shell hasn't reported the PID yet.
    Starting,
    Running,
    Stopping,
    /// Stopped by the user.
    Stopped,
    /// Finished on its own with exit code 0 (an install that completed).
    Exited,
    /// Finished on its own with a non-zero code, or couldn't start.
    Failed,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskInfo {
    pub id: String,
    pub distro: String,
    pub path: String,
    /// Folder name, for display.
    pub name: String,
    pub kind: TaskKind,
    /// e.g. `pnpm run dev`
    pub command: String,
    pub status: TaskStatus,
    pub exit_code: Option<i32>,
    /// Linux PID of the package manager (root of the process tree).
    pub pid: Option<u32>,
    /// Local URLs printed by the dev server (`http://localhost:5173/`).
    pub urls: Vec<String>,
    /// Unix time in ms.
    pub started_at: u64,
}

impl TaskInfo {
    pub(crate) fn active(&self) -> bool {
        matches!(self.status, TaskStatus::Starting | TaskStatus::Running | TaskStatus::Stopping)
    }
}

struct Task {
    info: Mutex<TaskInfo>,
    log: Mutex<VecDeque<String>>,
    kill: Mutex<Option<oneshot::Sender<()>>>,
    stop_requested: AtomicBool,
}

#[derive(Default)]
pub struct ProjectRunner {
    tasks: Mutex<HashMap<String, Arc<Task>>>,
    counter: AtomicU64,
}

// ---------------------------------------------------------------------------
// Inspect
// ---------------------------------------------------------------------------

const INSPECT_SCRIPT: &str = r#"cd "$1" 2>/dev/null || { echo "pasta inexistente: $1" >&2; exit 1; }
printf '@@locks'; for f in pnpm-lock.yaml yarn.lock bun.lockb bun.lock package-lock.json; do [ -e "$f" ] && printf ' %s' "$f"; done; printf '\n'
[ -d node_modules ] && echo @@node_modules
echo @@package
cat package.json 2>/dev/null"#;

pub fn parse_inspect(text: &str) -> Result<ProjectInfo, String> {
    let (head, json) = text.split_once("@@package").ok_or("Resposta inesperada ao ler o projeto")?;
    let value: serde_json::Value =
        serde_json::from_str(json.trim()).map_err(|_| "package.json ausente ou inválido".to_string())?;
    let locks = head
        .lines()
        .find_map(|l| l.trim_end().strip_prefix("@@locks"))
        .unwrap_or("")
        .split_whitespace()
        .collect::<Vec<_>>();

    // `"packageManager": "pnpm@9.1.0"` (corepack) wins over lockfiles.
    let declared = value
        .get("packageManager")
        .and_then(|v| v.as_str())
        .and_then(|s| s.split('@').next())
        .filter(|pm| ["npm", "pnpm", "yarn", "bun"].contains(pm));
    let package_manager = declared
        .or_else(|| {
            [("pnpm-lock.yaml", "pnpm"), ("yarn.lock", "yarn"), ("bun.lockb", "bun"), ("bun.lock", "bun")]
                .iter()
                .find(|(lock, _)| locks.contains(lock))
                .map(|(_, pm)| *pm)
        })
        .unwrap_or("npm")
        .to_string();

    let scripts: Vec<ProjectScript> = value
        .get("scripts")
        .and_then(|s| s.as_object())
        .map(|obj| {
            obj.iter()
                .filter(|(name, _)| valid_script_name(name))
                .map(|(name, cmd)| ProjectScript { name: name.clone(), command: cmd.as_str().unwrap_or("").to_string() })
                .collect()
        })
        .unwrap_or_default();
    let suggested = PREFERRED_SCRIPTS
        .iter()
        .find(|p| scripts.iter().any(|s| s.name == **p))
        .map(|s| s.to_string());

    Ok(ProjectInfo {
        package_manager,
        scripts,
        suggested,
        has_node_modules: head.lines().any(|l| l.trim() == "@@node_modules"),
    })
}

fn valid_script_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 100
        && !name.starts_with('-')
        && name.chars().all(|c| c.is_ascii_alphanumeric() || ":_-.@/+".contains(c))
}

pub(crate) async fn inspect(distro: &str, path: &str) -> Result<ProjectInfo, String> {
    validate_distro_name(distro)?;
    let path = normalize_linux_path(path)?;
    ensure_running(distro).await?;
    let text = run(sh_in_distro(distro, None, INSPECT_SCRIPT, &[&path]), INSPECT_TIMEOUT).await?;
    parse_inspect(&text)
}

// ---------------------------------------------------------------------------
// Log parsing
// ---------------------------------------------------------------------------

/// Local URLs in a dev-server line: `Local: http://localhost:5173/`,
/// `ready on http://0.0.0.0:3000` (shown as localhost).
pub fn find_local_urls(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = line;
    while let Some(i) = rest.find("http") {
        let candidate = &rest[i..];
        let end = candidate
            .find(|c: char| c.is_whitespace() || "\"'`<>)]|,".contains(c))
            .unwrap_or(candidate.len());
        let url = candidate[..end].trim_end_matches(['.', ';']);
        rest = &candidate[end.max(4)..];
        let Some(after) = url.strip_prefix("http://").or_else(|| url.strip_prefix("https://")) else { continue };
        let scheme = if url.starts_with("https") { "https" } else { "http" };
        let (host, tail) = after.split_once(':').unwrap_or((after, ""));
        let port_ok = tail.split('/').next().is_some_and(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()));
        let host = match host {
            "localhost" | "127.0.0.1" => "localhost",
            "0.0.0.0" | "[::]" | "[::1]" => "localhost",
            _ => continue,
        };
        if port_ok {
            out.push(format!("{scheme}://{host}:{tail}"));
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Start / stop
// ---------------------------------------------------------------------------

/// Prints its PID, then `exec`s the package manager so that PID is the tree's root.
const RUN_SCRIPT: &str = r#"cd "$1" || exit 1
shift
export BROWSER=none
printf '@@pid %s\n' "$$"
exec "$@""#;

/// Signals a process and all its descendants (TERM, then KILL whatever is left after ~5s).
const KILL_TREE_SCRIPT: &str = r#"all=
walk() { all="$all $1"; for c in $(cat /proc/"$1"/task/*/children 2>/dev/null); do walk "$c"; done; }
[ -d "/proc/$1" ] || exit 0
walk "$1"
kill -TERM $all 2>/dev/null
i=0
while [ $i -lt 10 ]; do
  alive=
  for p in $all; do
    [ -d "/proc/$p" ] && ! grep -q '^State:.*Z' "/proc/$p/status" 2>/dev/null && alive="$alive $p"
  done
  [ -z "$alive" ] && exit 0
  sleep 0.5; i=$((i + 1))
done
kill -KILL $alive 2>/dev/null
exit 0"#;

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

fn emit(sink: &TaskSink, task: &Task) {
    let info = task.info.lock().unwrap().clone();
    sink(&info);
}

pub(crate) fn app_sink(app: AppHandle) -> TaskSink {
    Arc::new(move |info: &TaskInfo| {
        let _ = app.emit("project-task", info);
    })
}

fn push_log(task: &Task, line: String) {
    let mut log = task.log.lock().unwrap();
    if log.len() >= MAX_LOG_LINES {
        log.pop_front();
    }
    log.push_back(line);
}

const MAX_EARLY_LINES: usize = 200;
const MAX_EARLY_BYTES: usize = 64 * 1024;

/// rc-file noise printed before `@@pid`, kept only to explain a shell that died early. Bounded in
/// lines and bytes; the last lines are the ones kept (that's where the error is).
#[derive(Default)]
struct EarlyLog {
    lines: VecDeque<String>,
    bytes: usize,
}

impl EarlyLog {
    fn push(&mut self, line: String) {
        self.bytes += line.len();
        self.lines.push_back(line);
        while self.lines.len() > MAX_EARLY_LINES || self.bytes > MAX_EARLY_BYTES {
            match self.lines.pop_front() {
                Some(old) => self.bytes -= old.len(),
                None => break,
            }
        }
    }
}

/// Reads one output stream into the task log. Lines before `@@pid` are rc-file noise, kept
/// aside and only shown if the shell dies before reaching the project (e.g. `cd` failed).
/// Each line is capped (`lines::BoundedLines`): a dev server printing megabytes without a
/// newline can't grow the app's memory.
async fn pump(sink: TaskSink, task: Arc<Task>, stream: impl AsyncRead + Unpin, early: Arc<Mutex<EarlyLog>>) {
    let mut lines = BoundedLines::new(BufReader::new(stream), MAX_LOG_LINE_BYTES);
    while let Ok(Some(raw)) = lines.next_line().await {
        let line = strip_ansi(&raw);
        if let Some(pid) = line.trim().strip_prefix("@@pid ") {
            let mut info = task.info.lock().unwrap();
            info.pid = pid.trim().parse().ok();
            if info.status == TaskStatus::Starting {
                info.status = TaskStatus::Running;
            }
            drop(info);
            emit(&sink, &task);
            continue;
        }
        if task.info.lock().unwrap().pid.is_none() {
            early.lock().unwrap().push(line);
            continue;
        }
        let urls = find_local_urls(&line);
        push_log(&task, line);
        if !urls.is_empty() {
            let mut info = task.info.lock().unwrap();
            let before = info.urls.len();
            for url in urls {
                if !info.urls.contains(&url) && info.urls.len() < MAX_URLS {
                    info.urls.push(url);
                }
            }
            let changed = info.urls.len() != before;
            drop(info);
            if changed {
                emit(&sink, &task);
            }
        }
    }
}

impl ProjectRunner {
    fn get(&self, id: &str) -> Result<Arc<Task>, String> {
        self.tasks.lock().unwrap().get(id).cloned().ok_or_else(|| format!("Tarefa não encontrada: {id}"))
    }

    pub fn list(&self) -> Vec<TaskInfo> {
        let mut all: Vec<TaskInfo> = self.tasks.lock().unwrap().values().map(|t| t.info.lock().unwrap().clone()).collect();
        all.sort_by_key(|t| t.started_at);
        all
    }

    pub async fn start(
        &self,
        sink: TaskSink,
        distro: &str,
        path: &str,
        kind: TaskKind,
        script: Option<&str>,
    ) -> Result<TaskInfo, String> {
        let path = normalize_linux_path(path)?;
        let project = inspect(distro, &path).await?;
        let pm = project.package_manager.clone();
        let argv: Vec<String> = match kind {
            TaskKind::Install => vec![pm.clone(), "install".into()],
            TaskKind::Run => {
                let script = script.map(str::to_string).or(project.suggested.clone()).ok_or(
                    "Nenhum script de execução (dev/start) no package.json: escolha um script",
                )?;
                if !project.scripts.iter().any(|s| s.name == script) {
                    return Err(format!("Script inexistente no package.json: {script}"));
                }
                if !project.has_node_modules {
                    return Err("Instale as dependências (node_modules) antes de rodar o projeto".into());
                }
                vec![pm.clone(), "run".into(), script]
            }
        };

        {
            let tasks = self.tasks.lock().unwrap();
            if tasks.values().any(|t| {
                let i = t.info.lock().unwrap();
                i.active() && i.distro == distro && i.path == path
            }) {
                return Err("Já existe uma tarefa ativa neste projeto".into());
            }
            if tasks.values().filter(|t| t.info.lock().unwrap().active()).count() >= MAX_TASKS {
                return Err(format!("Limite de {MAX_TASKS} tarefas simultâneas"));
            }
        }

        let mut args: Vec<&str> = vec![&path];
        args.extend(argv.iter().map(String::as_str));
        let mut child = user_shell_command(distro, RUN_SCRIPT, &args)
            .spawn()
            .map_err(|e| format!("Falha ao iniciar: {e}"))?;

        let id = format!("task-{}", self.counter.fetch_add(1, Ordering::Relaxed) + 1);
        let (kill_tx, kill_rx) = oneshot::channel();
        let task = Arc::new(Task {
            info: Mutex::new(TaskInfo {
                id: id.clone(),
                distro: distro.to_string(),
                name: path.rsplit('/').next().unwrap_or(&path).to_string(),
                path: path.clone(),
                kind,
                command: argv.join(" "),
                status: TaskStatus::Starting,
                exit_code: None,
                pid: None,
                urls: Vec::new(),
                started_at: now_ms(),
            }),
            log: Mutex::new(VecDeque::new()),
            kill: Mutex::new(Some(kill_tx)),
            stop_requested: AtomicBool::new(false),
        });
        push_log(&task, format!("$ {}", argv.join(" ")));
        self.tasks.lock().unwrap().insert(id, Arc::clone(&task));

        let early = Arc::new(Mutex::new(EarlyLog::default()));
        let readers = [
            child.stdout.take().map(|s| tauri::async_runtime::spawn(pump(Arc::clone(&sink), Arc::clone(&task), s, Arc::clone(&early)))),
            child.stderr.take().map(|s| tauri::async_runtime::spawn(pump(Arc::clone(&sink), Arc::clone(&task), s, Arc::clone(&early)))),
        ];
        let waiter_task = Arc::clone(&task);
        let waiter_sink = Arc::clone(&sink);
        tauri::async_runtime::spawn(async move {
            let status = tokio::select! {
                status = child.wait() => status,
                _ = kill_rx => {
                    let _ = child.start_kill();
                    child.wait().await
                }
            };
            for reader in readers.into_iter().flatten() {
                let _ = reader.await;
            }
            let code = status.ok().and_then(|s| s.code());
            let stopped = waiter_task.stop_requested.load(Ordering::SeqCst);
            {
                let mut info = waiter_task.info.lock().unwrap();
                if info.pid.is_none() {
                    // Died before reaching the project: show what the shell said.
                    for line in early.lock().unwrap().lines.drain(..) {
                        push_log(&waiter_task, line);
                    }
                }
                info.exit_code = code;
                info.status = if stopped {
                    TaskStatus::Stopped
                } else if code == Some(0) {
                    TaskStatus::Exited
                } else {
                    TaskStatus::Failed
                };
            }
            push_log(&waiter_task, match code {
                Some(c) => format!("[processo encerrado · código {c}]"),
                None => "[processo encerrado]".into(),
            });
            emit(&waiter_sink, &waiter_task);
        });

        emit(&sink, &task);
        let info = task.info.lock().unwrap().clone();
        Ok(info)
    }

    pub async fn stop(&self, sink: &TaskSink, id: &str) -> Result<(), String> {
        let task = self.get(id)?;
        let (distro, pid) = {
            let mut info = task.info.lock().unwrap();
            if !info.active() {
                return Ok(());
            }
            info.status = TaskStatus::Stopping;
            (info.distro.clone(), info.pid)
        };
        task.stop_requested.store(true, Ordering::SeqCst);
        emit(sink, &task);
        if let Some(pid) = pid {
            let result = kill_tree(&distro, pid, false).await;
            if let Err(e) = &result {
                push_log(&task, format!("[falha ao encerrar a árvore de processos: {e}]"));
            }
        }
        // Whatever is left on the Windows side (or the shell, if it never printed its PID).
        if let Some(kill) = task.kill.lock().unwrap().take() {
            let _ = kill.send(());
        }
        Ok(())
    }

    /// Before `wsl --shutdown`: the VM takes every task down with it.
    pub fn mark_all_stopping(&self) {
        for task in self.tasks.lock().unwrap().values() {
            if task.info.lock().unwrap().active() {
                task.stop_requested.store(true, Ordering::SeqCst);
            }
        }
    }

    /// App exit: stop every active task, waiting at most a few seconds in total. Tasks already
    /// being stopped are skipped, so a second call (quit, then `RunEvent::Exit`) is a no-op and
    /// never boots a VM that `wsl --shutdown` just took down.
    pub fn stop_all_blocking(&self) {
        let targets: Vec<(String, Option<u32>, Arc<Task>)> = self
            .tasks
            .lock()
            .unwrap()
            .values()
            .filter(|t| t.info.lock().unwrap().active() && !t.stop_requested.load(Ordering::SeqCst))
            .map(|t| {
                let i = t.info.lock().unwrap();
                (i.distro.clone(), i.pid, Arc::clone(t))
            })
            .collect();
        if targets.is_empty() {
            return;
        }
        tauri::async_runtime::block_on(async move {
            let mut set = tokio::task::JoinSet::new();
            for (distro, pid, task) in targets {
                task.stop_requested.store(true, Ordering::SeqCst);
                set.spawn(async move {
                    if let Some(pid) = pid {
                        let _ = kill_tree(&distro, pid, false).await;
                    }
                    if let Some(kill) = task.kill.lock().unwrap().take() {
                        let _ = kill.send(());
                    }
                });
            }
            let _ = tokio::time::timeout(Duration::from_secs(8), async { while set.join_next().await.is_some() {} }).await;
        });
    }

    pub fn log(&self, id: &str) -> Result<Vec<String>, String> {
        Ok(self.get(id)?.log.lock().unwrap().iter().cloned().collect())
    }

    pub fn dismiss(&self, id: &str) -> Result<(), String> {
        let mut tasks = self.tasks.lock().unwrap();
        if tasks.get(id).is_some_and(|t| t.info.lock().unwrap().active()) {
            return Err("Pare a tarefa antes de removê-la".into());
        }
        tasks.remove(id);
        Ok(())
    }
}

/// TERM → KILL on `pid` and its descendants. `as_root` for processes the app didn't start
/// (any listening port); tasks started by the app run as the user, who owns them.
pub(crate) async fn kill_tree(distro: &str, pid: u32, as_root: bool) -> Result<(), String> {
    validate_distro_name(distro)?;
    if pid <= 1 {
        return Err("PID inválido".into());
    }
    let user = if as_root { Some("root") } else { None };
    run(sh_in_distro(distro, user, KILL_TREE_SCRIPT, &[&pid.to_string()]), STOP_TIMEOUT).await.map(|_| ())
}

// ---------------------------------------------------------------------------
// Tauri commands
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn inspect_project(distro: String, path: String) -> Result<ProjectInfo, String> {
    inspect(&distro, &path).await
}

#[tauri::command]
pub async fn start_project(
    app: AppHandle,
    runner: State<'_, ProjectRunner>,
    distro: String,
    path: String,
    kind: String,
    script: Option<String>,
) -> Result<TaskInfo, String> {
    let kind = match kind.as_str() {
        "run" => TaskKind::Run,
        "install" => TaskKind::Install,
        other => return Err(format!("Tipo de tarefa inválido: {other}")),
    };
    runner.start(app_sink(app), &distro, &path, kind, script.as_deref()).await
}

#[tauri::command]
pub async fn stop_project(app: AppHandle, runner: State<'_, ProjectRunner>, id: String) -> Result<(), String> {
    runner.stop(&app_sink(app), &id).await
}

#[tauri::command]
pub fn list_project_tasks(runner: State<'_, ProjectRunner>) -> Vec<TaskInfo> {
    runner.list()
}

#[tauri::command]
pub fn get_project_log(runner: State<'_, ProjectRunner>, id: String) -> Result<Vec<String>, String> {
    runner.log(&id)
}

#[tauri::command]
pub fn dismiss_project_task(runner: State<'_, ProjectRunner>, id: String) -> Result<(), String> {
    runner.dismiss(&id)
}

/// "Parar" on a listening port the app didn't start: its whole process tree, as root.
#[tauri::command]
pub async fn stop_process_tree(distro: String, pid: u32) -> Result<(), String> {
    ensure_running(&distro).await?;
    kill_tree(&distro, pid, true).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncBufReadExt;

    #[test]
    fn early_log_is_bounded() {
        let mut early = EarlyLog::default();
        for i in 0..1000 {
            early.push(format!("ruído {i}"));
        }
        assert_eq!(early.lines.len(), MAX_EARLY_LINES);
        assert_eq!(early.lines.back().map(String::as_str), Some("ruído 999"), "the last lines are kept");
        let mut big = EarlyLog::default();
        for _ in 0..20 {
            big.push("x".repeat(MAX_LOG_LINE_BYTES));
        }
        assert!(big.bytes <= MAX_EARLY_BYTES && big.bytes == big.lines.iter().map(String::len).sum::<usize>());
    }

    #[test]
    fn inspects_package_json() {
        let text = "zsh noise\n@@locks pnpm-lock.yaml package-lock.json\n@@node_modules\n@@package\n{\"name\":\"web\",\"scripts\":{\"build\":\"vite build\",\"dev\":\"vite\",\"start\":\"node .\",\"bad name;rm\":\"x\"}}";
        let p = parse_inspect(text).unwrap();
        assert_eq!(p.package_manager, "pnpm");
        assert!(p.has_node_modules);
        assert_eq!(p.suggested.as_deref(), Some("dev"));
        assert_eq!(p.scripts.len(), 3, "invalid script names are dropped");

        let text = "@@locks\n@@package\n{\"packageManager\":\"yarn@4.1.0\",\"scripts\":{\"start:dev\":\"nest start --watch\",\"start\":\"nest start\"}}";
        let p = parse_inspect(text).unwrap();
        assert_eq!((p.package_manager.as_str(), p.suggested.as_deref(), p.has_node_modules), ("yarn", Some("start:dev"), false));

        let p = parse_inspect("@@locks\n@@package\n{\"name\":\"lib\"}").unwrap();
        assert_eq!((p.package_manager.as_str(), p.suggested, p.scripts.len()), ("npm", None, 0));
        assert!(parse_inspect("@@locks\n@@package\n").is_err());
    }

    #[test]
    fn finds_local_urls() {
        assert_eq!(find_local_urls("  ➜  Local:   http://localhost:5173/"), ["http://localhost:5173/"]);
        assert_eq!(find_local_urls("- ready started server on 0.0.0.0:3000, url: http://0.0.0.0:3000"), ["http://localhost:3000"]);
        assert_eq!(find_local_urls("Network: http://172.20.1.5:5173/ (use --host)"), Vec::<String>::new());
        assert_eq!(find_local_urls("see https://nextjs.org/docs"), Vec::<String>::new());
        assert_eq!(find_local_urls("api at \"http://127.0.0.1:3333/api\"."), ["http://localhost:3333/api"]);
    }

    #[test]
    fn validates_script_names() {
        for ok in ["dev", "start:dev", "build-prod", "test.unit", "@x/y"] {
            assert!(valid_script_name(ok), "{ok}");
        }
        for bad in ["", "-c", "a b", "x;rm", "$(id)"] {
            assert!(!valid_script_name(bad), "{bad}");
        }
    }

    /// End to end through the real runner: a throwaway Node project in /tmp whose `dev` script
    /// is an HTTP server (child of npm, like real dev servers). Run → URL in the log and a
    /// listening port owned by that folder → stop → port and processes gone.
    #[tokio::test]
    #[ignore]
    async fn live_run_and_stop_project() {
        let distro = std::env::var("WSL_TEST_DISTRO").expect("set WSL_TEST_DISTRO");
        let dir = format!("/tmp/wsls-demo-{}", std::process::id());
        let setup = r#"mkdir -p "$1/node_modules" && cd "$1" &&
printf '%s' '{"name":"demo","scripts":{"dev":"node server.js","build":"echo ok"}}' > package.json &&
printf '%s' 'require("http").createServer((q,s)=>s.end("ok")).listen(48731,()=>console.log("  Local:   http://localhost:48731/"))' > server.js"#;
        run(sh_in_distro(&distro, None, setup, &[&dir]), STOP_TIMEOUT).await.expect("setup");

        let events: Arc<Mutex<Vec<TaskInfo>>> = Arc::default();
        let collected = Arc::clone(&events);
        let sink: TaskSink = Arc::new(move |t: &TaskInfo| collected.lock().unwrap().push(t.clone()));
        let runner = ProjectRunner::default();
        let task = runner.start(Arc::clone(&sink), &distro, &dir, TaskKind::Run, None).await.expect("start");
        assert_eq!(task.command, "npm run dev");

        // Wait for the URL printed by the server.
        let deadline = std::time::Instant::now() + Duration::from_secs(40);
        let running = loop {
            let info = runner.list().into_iter().find(|t| t.id == task.id).unwrap();
            if !info.urls.is_empty() || std::time::Instant::now() > deadline {
                break info;
            }
            tokio::time::sleep(Duration::from_millis(300)).await;
        };
        println!("running: {running:#?}\nlog: {:#?}", runner.log(&task.id).unwrap());
        assert_eq!(running.status, TaskStatus::Running);
        assert_eq!(running.urls, ["http://localhost:48731/"]);
        let ports = crate::packages::listening_ports(&distro).await.unwrap();
        let port = ports.iter().find(|p| p.port == 48731).expect("port listening");
        assert_eq!(port.cwd.as_deref(), Some(dir.as_str()), "port is attributed to the project folder");

        runner.stop(&sink, &task.id).await.expect("stop");
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        let done = loop {
            let info = runner.list().into_iter().find(|t| t.id == task.id).unwrap();
            if !info.active() || std::time::Instant::now() > deadline {
                break info;
            }
            tokio::time::sleep(Duration::from_millis(300)).await;
        };
        assert_eq!(done.status, TaskStatus::Stopped);
        let ports = crate::packages::listening_ports(&distro).await.unwrap();
        assert!(!ports.iter().any(|p| p.port == 48731), "port must be free after stop");
        let statuses: Vec<TaskStatus> = events.lock().unwrap().iter().map(|t| t.status).collect();
        println!("event statuses: {statuses:?}");
        run(sh_in_distro(&distro, None, r#"rm -rf "$1""#, &[&dir]), STOP_TIMEOUT).await.unwrap();
    }

    // Live: WSL_TEST_DISTRO=Ubuntu cargo test --lib -- --ignored --nocapture projects::tests::live_
    // Starts a tiny node HTTP server through the real runner path, finds its URL, stops it and
    // checks the whole tree is gone.
    #[tokio::test]
    #[ignore]
    async fn live_kill_tree() {
        let distro = std::env::var("WSL_TEST_DISTRO").expect("set WSL_TEST_DISTRO");
        // Same shape as a real task: a long-lived wsl.exe carrying a parent shell with two
        // children (npm → node → esbuild). WSL kills a wsl.exe's processes when it exits, so
        // the carrier stays alive until kill_tree (a separate wsl.exe call) does its job.
        let mut carrier = sh_in_distro(&distro, None, "sleep 300 & sleep 300 & echo $$; wait", &[]).spawn().unwrap();
        let mut out = BufReader::new(carrier.stdout.take().unwrap()).lines();
        let pid: u32 = out.next_line().await.unwrap().unwrap().trim().parse().unwrap();
        let count = format!("sleep 0.5; cat /proc/{pid}/task/*/children");
        let children = run(sh_in_distro(&distro, None, &count, &[]), STOP_TIMEOUT).await.unwrap();
        println!("pid {pid} children: {children}");
        assert_eq!(children.split_whitespace().count(), 2);
        kill_tree(&distro, pid, false).await.unwrap();
        let check = format!("for p in {pid} {}; do [ -d /proc/$p ] && echo alive $p; done; exit 0", children.trim());
        let left = run(sh_in_distro(&distro, None, &check, &[]), STOP_TIMEOUT).await.unwrap();
        assert!(left.trim().is_empty(), "left: {left}");
        // With its tree gone, the carrier wsl.exe exits on its own.
        let status = tokio::time::timeout(Duration::from_secs(10), carrier.wait()).await.expect("carrier exit");
        println!("carrier exited: {status:?}");
    }
}
