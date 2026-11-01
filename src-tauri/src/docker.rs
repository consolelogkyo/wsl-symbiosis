//! Docker: finds a working engine (native `dockerd` inside the distro, or Docker Desktop on
//! Windows), starts it, lists containers (with live usage) and creates 1-click databases.
//! The table explorer that talks to those databases lives in `database.rs`.
//!
//! Rules:
//! - never start a distro as a side effect: the distro's own Docker is only looked for when it
//!   is already running (CLAUDE.md 3.1);
//! - secrets (database passwords) never travel as command-line arguments: Docker Desktop's
//!   `docker.exe` reads them from its environment (`-e NAME` without a value), the distro's
//!   `docker` gets them through stdin (see [`NATIVE_DOCKER`]);
//! - databases are published on 127.0.0.1 only: a weak local password is never exposed to the
//!   network.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::{AppHandle, Emitter, State};
use tokio::process::Command;
use tokio::sync::Mutex;

use crate::wsl::{self, Captured};

const LINUX_PATH: &str = "export PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin\n";
const PROBE_TIMEOUT: Duration = Duration::from_secs(15);
const LIST_TIMEOUT: Duration = Duration::from_secs(30);
const ACTION_TIMEOUT: Duration = Duration::from_secs(90);
const PULL_TIMEOUT: Duration = Duration::from_secs(20 * 60);
/// Docker Desktop takes 20–60 s on a cold start; the first one after an update can take longer.
const ENGINE_START_TIMEOUT: Duration = Duration::from_secs(150);
/// MySQL's first start (initializing the data folder) takes ~20–40 s.
const READY_TIMEOUT: Duration = Duration::from_secs(150);

/// Label on every container created here ("Seus bancos").
pub const MANAGED_LABEL: &str = "com.wslsymbiosis.managed";
const KIND_LABEL: &str = "com.wslsymbiosis.kind";

/// The distro's `docker`, run as root (socket access without the docker group). The first
/// argument says how many `NAME=value` lines precede the payload on stdin: they are exported,
/// so `docker run -e NAME` / `docker exec -e NAME` pass them on without an argument.
const NATIVE_DOCKER: &str = r#"n=$1; shift
while [ "$n" -gt 0 ]; do IFS= read -r kv || exit 97; export "$kv"; n=$((n-1)); done
exec docker "$@""#;

/// `@@none` (no dockerd here) · `@@ready <version>` · `@@stopped`.
const NATIVE_PROBE: &str = r#"command -v dockerd >/dev/null 2>&1 || { echo @@none; exit 0; }
if v=$(docker version --format '{{.Server.Version}}' 2>/dev/null) && [ -n "$v" ]; then echo "@@ready $v"; else echo @@stopped; fi"#;

const NATIVE_START: &str = "(service docker start || systemctl start docker) 2>&1";

// ---------------------------------------------------------------------------
// Engine discovery
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DockerSource {
    /// `dockerd` installed inside the distro (Docker Engine).
    WslNative,
    /// Docker Desktop on Windows (`docker.exe`).
    DockerDesktop,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EngineState {
    Ready,
    Stopped,
    NotInstalled,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DockerStatus {
    pub state: EngineState,
    /// Where the engine is (ready) or would be started from (stopped).
    pub source: Option<DockerSource>,
    pub version: Option<String>,
    pub desktop_installed: bool,
    pub native_installed: bool,
    /// False when the distro was stopped: its own Docker (if any) wasn't looked for.
    pub distro_checked: bool,
    /// `start_docker_engine` is waiting for the engine.
    pub starting: bool,
}

/// How to reach a working engine.
#[derive(Debug, Clone)]
pub(crate) struct Engine {
    pub source: DockerSource,
    distro: String,
    desktop_cli: Option<PathBuf>,
}

fn program_files() -> PathBuf {
    std::env::var_os("ProgramFiles").map(PathBuf::from).unwrap_or_else(|| r"C:\Program Files".into())
}

/// Docker Desktop's CLI, else a `docker.exe` on the Windows PATH.
fn desktop_cli() -> Option<PathBuf> {
    let bundled = program_files().join(r"Docker\Docker\resources\bin\docker.exe");
    if bundled.is_file() {
        return Some(bundled);
    }
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).map(|dir| dir.join("docker.exe")).find(|p| p.is_file())
}

fn desktop_app() -> Option<PathBuf> {
    Some(program_files().join(r"Docker\Docker\Docker Desktop.exe")).filter(|p| p.is_file())
}

impl Engine {
    fn desktop(cli: PathBuf) -> Self {
        Engine { source: DockerSource::DockerDesktop, distro: String::new(), desktop_cli: Some(cli) }
    }

    fn native(distro: &str) -> Self {
        Engine { source: DockerSource::WslNative, distro: distro.to_string(), desktop_cli: None }
    }

    fn command(&self, args: &[&str], secret_count: usize) -> Command {
        match (self.source, &self.desktop_cli) {
            (DockerSource::DockerDesktop, Some(cli)) => {
                let mut cmd = wsl::hidden_command(cli);
                cmd.args(args).kill_on_drop(true);
                cmd
            }
            _ => {
                let count = secret_count.to_string();
                let mut all = vec![count.as_str()];
                all.extend_from_slice(args);
                wsl::sh_in_distro(&self.distro, Some("root"), &format!("{LINUX_PATH}{NATIVE_DOCKER}"), &all)
            }
        }
    }

    /// Runs `docker <args>`. `secrets` become environment variables of the docker client (pass
    /// `-e NAME` in `args` to hand them to the container); `stdin` is fed after them.
    pub(crate) async fn run(
        &self,
        args: &[&str],
        timeout: Duration,
        stdin: Option<&[u8]>,
        secrets: &[(&str, &str)],
    ) -> Result<Captured, String> {
        for (name, value) in secrets {
            if value.contains(['\n', '\r', '\0']) {
                return Err(format!("Valor inválido para {name}"));
            }
        }
        let mut cmd = self.command(args, secrets.len());
        let mut input = Vec::new();
        match self.source {
            DockerSource::DockerDesktop => {
                for (name, value) in secrets {
                    cmd.env(name, value);
                }
            }
            DockerSource::WslNative => {
                for (name, value) in secrets {
                    input.extend_from_slice(format!("{name}={value}\n").as_bytes());
                }
            }
        }
        if let Some(data) = stdin {
            input.extend_from_slice(data);
        }
        let feed = stdin.is_some() || !input.is_empty();
        wsl::run_capture(cmd, timeout, feed.then_some(input.as_slice()), "docker").await
    }

    /// stdout of a successful `docker <args>`, or the explained error.
    pub(crate) async fn text(&self, args: &[&str], timeout: Duration) -> Result<String, String> {
        let out = self.run(args, timeout, None, &[]).await?;
        if out.success() {
            Ok(wsl::decode_output(&out.stdout))
        } else {
            Err(explain_docker_error(&out.error_text()))
        }
    }
}

/// First line in plain Portuguese, then Docker's own message.
pub(crate) fn explain_docker_error(raw: &str) -> String {
    let raw = raw.trim();
    let l = raw.to_ascii_lowercase();
    let has = |needles: &[&str]| needles.iter().any(|n| l.contains(n));
    let hint = if has(&[
        "cannot connect to the docker daemon",
        "failed to connect to the docker api",
        "is the docker daemon running",
        "dockerdesktoplinuxengine",
        "docker_engine",
    ]) {
        "O Docker está desligado. Ligue-o nesta tela e tente de novo."
    } else if has(&["port is already allocated", "address already in use", "ports are not available", "bind: an attempt was made"]) {
        "Essa porta já está em uso por outro programa ou banco. Escolha outra porta."
    } else if has(&["is already in use by container"]) {
        "Já existe um container com esse nome. Escolha outro nome."
    } else if has(&["no such container"]) {
        "Esse container não existe mais (talvez tenha sido removido fora do app). Atualize a lista."
    } else if has(&["is not running", "is restarting"]) {
        "O container está parado. Ligue-o primeiro."
    } else if has(&["pull access denied", "manifest unknown", "not found: manifest"]) {
        "Essa imagem não foi encontrada no Docker Hub."
    } else if has(&["tls handshake", "dial tcp", "i/o timeout", "no such host", "network is unreachable", "connection refused"]) {
        "Não foi possível baixar a imagem. Verifique a sua internet (só a primeira vez precisa dela)."
    } else if has(&["no space left"]) {
        "Falta espaço em disco para o Docker."
    } else if has(&["permission denied"]) && l.contains("docker.sock") {
        "Sem permissão para falar com o Docker da distro."
    } else if has(&["volume is in use"]) {
        "Os dados ainda estão ligados a outro container, então foram mantidos."
    } else {
        return raw.to_string();
    };
    if raw.is_empty() { hint.to_string() } else { format!("{hint}\n{raw}") }
}

async fn probe_native(distro: &str) -> (bool, Option<String>, bool) {
    // (checked, version when ready, installed)
    if wsl::validate_distro_name(distro).is_err() || crate::provisioner::is_system_distro(distro) {
        return (false, None, false);
    }
    if !wsl::is_running(distro).await.unwrap_or(false) {
        return (false, None, false);
    }
    let cmd = wsl::sh_in_distro(distro, Some("root"), &format!("{LINUX_PATH}{NATIVE_PROBE}"), &[]);
    match wsl::run(cmd, PROBE_TIMEOUT).await {
        Ok(text) => {
            let line = text.lines().find(|l| l.starts_with("@@")).unwrap_or("@@none").trim();
            match line.strip_prefix("@@ready ") {
                Some(version) => (true, Some(version.trim().to_string()), true),
                None => (true, None, line == "@@stopped"),
            }
        }
        Err(_) => (true, None, false),
    }
}

async fn probe_desktop(cli: &Option<PathBuf>) -> Option<String> {
    let cli = cli.as_ref()?;
    let engine = Engine::desktop(cli.clone());
    let text = engine.text(&["version", "--format", "{{.Server.Version}}"], PROBE_TIMEOUT).await.ok()?;
    Some(text.trim().to_string()).filter(|v| !v.is_empty())
}

/// Looks for both engines (in parallel). The distro's own engine wins when it answers.
async fn detect(distro: &str, starting: bool) -> (DockerStatus, Option<Engine>) {
    let cli = desktop_cli();
    let ((checked, native_version, native_installed), desktop_version) = tokio::join!(probe_native(distro), probe_desktop(&cli));
    let mut status = DockerStatus {
        state: EngineState::NotInstalled,
        source: None,
        version: None,
        desktop_installed: cli.is_some(),
        native_installed,
        distro_checked: checked,
        starting,
    };
    let engine = if let Some(version) = native_version {
        status.state = EngineState::Ready;
        status.source = Some(DockerSource::WslNative);
        status.version = Some(version);
        Some(Engine::native(distro))
    } else if let (Some(version), Some(cli)) = (desktop_version, cli) {
        status.state = EngineState::Ready;
        status.source = Some(DockerSource::DockerDesktop);
        status.version = Some(version);
        Some(Engine::desktop(cli))
    } else {
        if native_installed {
            status.state = EngineState::Stopped;
            status.source = Some(DockerSource::WslNative);
        } else if status.desktop_installed {
            status.state = EngineState::Stopped;
            status.source = Some(DockerSource::DockerDesktop);
        }
        None
    };
    (status, engine)
}

/// The engine last found working, so each action doesn't probe both again.
#[derive(Default)]
pub struct DockerManager {
    engine: Mutex<Option<Engine>>,
    starting: AtomicBool,
    creating: AtomicBool,
}

impl DockerManager {
    pub(crate) async fn engine(&self, distro: &str) -> Result<Engine, String> {
        let mut cached = self.engine.lock().await;
        if let Some(engine) = cached.as_ref() {
            if engine.source == DockerSource::DockerDesktop || engine.distro == distro {
                return Ok(engine.clone());
            }
        }
        let (status, engine) = detect(distro, self.starting.load(Ordering::SeqCst)).await;
        *cached = engine.clone();
        engine.ok_or_else(|| match status.state {
            EngineState::NotInstalled => "O Docker não foi encontrado neste computador.".to_string(),
            _ => "O Docker está desligado. Ligue-o na tela Bancos & Containers.".to_string(),
        })
    }

    async fn refresh(&self, distro: &str) -> DockerStatus {
        let (status, engine) = detect(distro, self.starting.load(Ordering::SeqCst)).await;
        *self.engine.lock().await = engine;
        status
    }
}

/// Clears a flag when dropped (also on early returns and errors).
struct FlagGuard<'a>(&'a AtomicBool);

impl Drop for FlagGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

fn take_flag<'a>(flag: &'a AtomicBool, busy: &str) -> Result<FlagGuard<'a>, String> {
    if flag.swap(true, Ordering::SeqCst) {
        Err(busy.to_string())
    } else {
        Ok(FlagGuard(flag))
    }
}

#[tauri::command]
pub async fn get_docker_status(distro: String, docker: State<'_, DockerManager>) -> Result<DockerStatus, String> {
    Ok(docker.refresh(&distro).await)
}

#[tauri::command]
pub async fn start_docker_engine(distro: String, docker: State<'_, DockerManager>) -> Result<DockerStatus, String> {
    start_engine(&docker, &distro).await
}

pub(crate) async fn start_engine(docker: &DockerManager, distro: &str) -> Result<DockerStatus, String> {
    let _guard = take_flag(&docker.starting, "O Docker já está sendo ligado. Aguarde.")?;
    let status = docker.refresh(distro).await;
    match (status.state, status.source) {
        (EngineState::Ready, _) => return Ok(status),
        (EngineState::NotInstalled, _) | (_, None) => return Err("O Docker não foi encontrado neste computador.".into()),
        (EngineState::Stopped, Some(DockerSource::WslNative)) => {
            let cmd = wsl::sh_in_distro(distro, Some("root"), &format!("{LINUX_PATH}{NATIVE_START}"), &[]);
            if let Err(e) = wsl::run(cmd, Duration::from_secs(60)).await {
                return Err(format!("Não foi possível ligar o Docker da distro {distro}.\n{e}"));
            }
        }
        (EngineState::Stopped, Some(DockerSource::DockerDesktop)) => {
            let app = desktop_app().ok_or("O Docker Desktop não foi encontrado em Arquivos de Programas.")?;
            launch_detached(&app)?;
        }
    }
    let launched = std::time::SystemTime::now();
    let started = Instant::now();
    let deadline = started + ENGINE_START_TIMEOUT;
    let desktop = status.source == Some(DockerSource::DockerDesktop);
    loop {
        tokio::time::sleep(Duration::from_secs(2)).await;
        let status = docker.refresh(distro).await;
        if status.state == EngineState::Ready {
            return Ok(DockerStatus { starting: false, ..status });
        }
        if desktop {
            // Docker Desktop shows its own error window and never answers: stop waiting right away.
            if let Some(error) = desktop_crash_since(launched) {
                return Err(explain_desktop_crash(&error));
            }
            // Closed (its Quit button, or it died) before getting ready. Give it a moment to appear.
            if started.elapsed() > Duration::from_secs(15) && !tokio::task::spawn_blocking(desktop_running).await.unwrap_or(true) {
                return Err("O Docker Desktop fechou antes de ficar pronto. Abra-o pelo menu Iniciar para ver o motivo e tente de novo.".into());
            }
        }
        if Instant::now() >= deadline {
            return Err(match status.source {
                Some(DockerSource::DockerDesktop) => "O Docker Desktop ainda não respondeu. Se ele abriu uma janela (termos de uso, login ou atualização), conclua por lá e tente de novo.".into(),
                _ => "O Docker da distro não respondeu a tempo. Tente de novo em instantes.".into(),
            });
        }
    }
}

#[tauri::command]
pub async fn stop_docker_engine(app: AppHandle, distro: String, docker: State<'_, DockerManager>) -> Result<DockerStatus, String> {
    let _guard = take_flag(&docker.starting, "O Docker já está sendo ligado ou desligado. Aguarde.")?;
    let status = docker.refresh(&distro).await;
    if status.state == EngineState::Ready && status.source == Some(DockerSource::WslNative) {
        // docker.socket too: with socket activation, the next `docker` call would start it again.
        let cmd = wsl::sh_in_distro(&distro, Some("root"), &format!("{LINUX_PATH}(systemctl stop docker.socket docker || service docker stop) 2>&1"), &[]);
        wsl::run(cmd, Duration::from_secs(60)).await.map_err(|e| format!("Não foi possível desligar o Docker da distro {distro}.\n{e}"))?;
    } else {
        stop_desktop_if_running().await?;
    }
    // The docker-desktop distro stopped with it: the Titlebar and the tray refresh.
    let _ = app.emit("wsl-changed", ());
    Ok(docker.refresh(&distro).await)
}

/// `docker desktop stop` (a clean quit) when Docker Desktop is running. True when it was.
pub(crate) async fn stop_desktop_if_running() -> Result<bool, String> {
    if !tokio::task::spawn_blocking(desktop_running).await.unwrap_or(false) {
        return Ok(false);
    }
    let cli = desktop_cli().ok_or("O docker.exe do Docker Desktop não foi encontrado.")?;
    Engine::desktop(cli)
        .text(&["desktop", "stop"], Duration::from_secs(120))
        .await
        .map_err(|e| format!("Não foi possível desligar o Docker Desktop.\n{e}"))?;
    Ok(true)
}

/// Docker Desktop or its backend is running.
fn desktop_running() -> bool {
    let mut sys = sysinfo::System::new();
    sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
    sys.processes().values().any(|p| {
        let name = p.name().to_string_lossy();
        name.eq_ignore_ascii_case("Docker Desktop.exe") || name.eq_ignore_ascii_case("com.docker.backend.exe")
    })
}

/// When Docker Desktop's backend crashes it writes `%LOCALAPPDATA%\Docker\backend.error.json`
/// (and shows its own error window). The error, when that file is newer than `since`.
fn desktop_crash_since(since: std::time::SystemTime) -> Option<String> {
    let path = PathBuf::from(std::env::var_os("LOCALAPPDATA")?).join(r"Docker\backend.error.json");
    if std::fs::metadata(&path).ok()?.modified().ok()? < since {
        return None;
    }
    let v: Value = serde_json::from_str(&std::fs::read_to_string(&path).ok()?).ok()?;
    Some(desktop_error_text(&v))
}

/// The innermost `description` of Docker Desktop's nested error JSON, else the top `message`.
pub fn desktop_error_text(v: &Value) -> String {
    fn deepest(v: &Value) -> Option<String> {
        for key in ["originalError", "error", "wrappedError"] {
            if let Some(found) = v.get(key).and_then(deepest) {
                return Some(found);
            }
        }
        v.get("description").and_then(Value::as_str).map(String::from)
    }
    deepest(v)
        .or_else(|| v.get("message").and_then(Value::as_str).map(String::from))
        .unwrap_or_else(|| "erro desconhecido".into())
}

pub fn explain_desktop_crash(error: &str) -> String {
    // Known Docker Desktop bug: a socket file left by a session that didn't close cleanly can't be
    // replaced (seen with 4.67 on `run\dockerInference`).
    let hint = if error.contains(r"\Docker\run\") && error.contains("cannot be accessed") {
        let file = error
            .split_whitespace()
            .find(|w| w.contains(r"\Docker\run\"))
            .map(|w| w.trim_end_matches([':', '.']).trim_start_matches("unix://"))
            .unwrap_or(r"%LOCALAPPDATA%\Docker\run\dockerInference");
        format!(
            "\n\nÉ um problema conhecido do Docker Desktop: sobrou um arquivo de uma sessão que não fechou direito. \
             Clique em Quit na janela de erro do Docker, apague o arquivo abaixo e tente de novo:\n{file}"
        )
    } else {
        "\n\nA janela de erro do Docker Desktop tem mais detalhes e o botão Quit para fechá-lo.".to_string()
    };
    format!("O Docker Desktop encontrou um erro e não ligou.\n{error}{hint}")
}

/// Starts a Windows GUI program that outlives this app (no pipes, no console).
fn launch_detached(path: &std::path::Path) -> Result<(), String> {
    let mut cmd = std::process::Command::new(path);
    cmd.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd.spawn().map(drop).map_err(|e| format!("Não foi possível abrir o Docker Desktop: {e}"))
}

// ---------------------------------------------------------------------------
// Containers
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Hash)]
#[serde(rename_all = "snake_case")]
pub enum DbKind {
    Postgres,
    Mysql,
    Mariadb,
    Redis,
    Mongo,
}

impl DbKind {
    pub fn internal_port(self) -> u16 {
        match self {
            DbKind::Postgres => 5432,
            DbKind::Mysql | DbKind::Mariadb => 3306,
            DbKind::Redis => 6379,
            DbKind::Mongo => 27017,
        }
    }

    fn label(self) -> &'static str {
        match self {
            DbKind::Postgres => "postgres",
            DbKind::Mysql => "mysql",
            DbKind::Mariadb => "mariadb",
            DbKind::Redis => "redis",
            DbKind::Mongo => "mongo",
        }
    }

    fn from_label(s: &str) -> Option<Self> {
        [DbKind::Postgres, DbKind::Mysql, DbKind::Mariadb, DbKind::Redis, DbKind::Mongo]
            .into_iter()
            .find(|k| k.label() == s)
    }
}

/// `postgres:16-alpine`, `docker.io/library/mysql:8.4`, `localhost:5000/redis@sha256:…` → kind.
pub fn db_kind_from_image(image: &str) -> Option<DbKind> {
    let without_digest = image.split('@').next().unwrap_or(image);
    let last = without_digest.rsplit('/').next().unwrap_or(without_digest);
    let repo = last.split(':').next().unwrap_or(last).to_ascii_lowercase();
    if repo.starts_with("postgres") || repo.starts_with("postgis") || repo.starts_with("pgvector") || repo.starts_with("timescaledb") {
        Some(DbKind::Postgres)
    } else if repo.starts_with("mysql") {
        Some(DbKind::Mysql)
    } else if repo.starts_with("mariadb") {
        Some(DbKind::Mariadb)
    } else if repo.starts_with("redis") || repo.starts_with("valkey") {
        Some(DbKind::Redis)
    } else if repo.starts_with("mongo") {
        Some(DbKind::Mongo)
    } else {
        None
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PortMapping {
    pub host_ip: String,
    pub host_port: u16,
    pub container_port: u16,
    pub protocol: String,
}

/// `docker ps` "Ports": `0.0.0.0:5432->5432/tcp, [::]:5432->5432/tcp, 6379/tcp`. Only published
/// ports are kept, once each (IPv4 and IPv6 bindings collapse).
pub fn parse_ports(s: &str) -> Vec<PortMapping> {
    let mut out: Vec<PortMapping> = Vec::new();
    for item in s.split(',').map(str::trim) {
        let Some((host, container)) = item.split_once("->") else { continue };
        let Some((host_ip, host_port)) = host.rsplit_once(':') else { continue };
        let (container_port, protocol) = container.split_once('/').unwrap_or((container, "tcp"));
        // Ranges ("8000-8001") keep their first port.
        let first = |p: &str| p.split('-').next().and_then(|n| n.parse::<u16>().ok());
        let (Some(host_port), Some(container_port)) = (first(host_port), first(container_port)) else { continue };
        let host_ip = host_ip.trim_matches(['[', ']']).to_string();
        let host_ip = if host_ip.is_empty() || host_ip == "::" { "0.0.0.0".to_string() } else { host_ip };
        if !out.iter().any(|p| p.host_port == host_port && p.container_port == container_port && p.protocol == protocol) {
            out.push(PortMapping { host_ip, host_port, container_port, protocol: protocol.to_string() });
        }
    }
    out
}

/// "a=b,com.docker.compose.project=app" → map (values with commas are cut; only flags are read).
fn parse_labels(s: &str) -> HashMap<String, String> {
    s.split(',')
        .filter_map(|kv| kv.split_once('='))
        .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
        .collect()
}

/// One `docker ps --format '{{json .}}'` line.
#[derive(Debug, Clone, PartialEq)]
pub struct PsRow {
    pub id: String,
    pub name: String,
    pub image: String,
    pub state: String,
    pub status: String,
    pub ports: Vec<PortMapping>,
    pub labels: HashMap<String, String>,
    pub created_at: String,
    pub mounts: Vec<String>,
}

pub fn parse_ps_line(line: &str) -> Option<PsRow> {
    let v: Value = serde_json::from_str(line.trim()).ok()?;
    let s = |key: &str| v.get(key).and_then(Value::as_str).unwrap_or("").to_string();
    let id = s("ID");
    if id.is_empty() {
        return None;
    }
    Some(PsRow {
        id,
        name: s("Names").split(',').next().unwrap_or("").trim_start_matches('/').to_string(),
        image: s("Image"),
        state: s("State").to_ascii_lowercase(),
        status: s("Status"),
        ports: parse_ports(&s("Ports")),
        labels: parse_labels(&s("Labels")),
        created_at: s("CreatedAt"),
        mounts: s("Mounts").split(',').map(str::trim).filter(|m| !m.is_empty()).map(String::from).collect(),
    })
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ContainerStats {
    pub cpu_percent: f64,
    pub memory_used: u64,
    pub memory_limit: u64,
}

/// "18.5MiB", "1.2GB", "0B" → bytes.
pub fn parse_size(s: &str) -> Option<u64> {
    let s = s.trim();
    let split = s.find(|c: char| !(c.is_ascii_digit() || c == '.'))?;
    let (num, unit) = s.split_at(split);
    let n: f64 = num.parse().ok()?;
    let mult: f64 = match unit.trim() {
        "B" => 1.0,
        "KiB" => 1024.0,
        "MiB" => 1024.0 * 1024.0,
        "GiB" => 1024.0 * 1024.0 * 1024.0,
        "TiB" => 1024f64.powi(4),
        "kB" | "KB" => 1e3,
        "MB" => 1e6,
        "GB" => 1e9,
        "TB" => 1e12,
        _ => return None,
    };
    Some((n * mult).round() as u64)
}

/// One `docker stats --no-stream --format '{{json .}}'` line → (container name, stats).
pub fn parse_stats_line(line: &str) -> Option<(String, ContainerStats)> {
    let v: Value = serde_json::from_str(line.trim()).ok()?;
    let s = |key: &str| v.get(key).and_then(Value::as_str).unwrap_or("");
    let name = s("Name").trim_start_matches('/').to_string();
    let cpu = s("CPUPerc").trim_end_matches('%').trim().parse::<f64>().unwrap_or(0.0);
    let (used, limit) = s("MemUsage").split_once('/').unwrap_or(("0B", "0B"));
    if name.is_empty() {
        return None;
    }
    Some((
        name,
        ContainerStats { cpu_percent: cpu, memory_used: parse_size(used).unwrap_or(0), memory_limit: parse_size(limit).unwrap_or(0) },
    ))
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DatabaseInfo {
    pub kind: DbKind,
    pub user: Option<String>,
    pub password: Option<String>,
    pub database: Option<String>,
    pub host_port: Option<u16>,
    /// `DATABASE_URL` style link (needs a published port).
    pub connection_url: Option<String>,
    /// Ready-to-paste `.env` lines.
    pub env_snippet: Option<String>,
    /// The table explorer can open it (Postgres, MySQL/MariaDB, Redis).
    pub explorer: bool,
}

/// Percent-encodes a URL user, password or path segment.
fn url_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn env_quote(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

/// Connection details from the container's own environment (and command, for
/// `redis-server --requirepass`).
pub fn database_info(kind: DbKind, env: &HashMap<String, String>, cmd: &[String], host_port: Option<u16>) -> DatabaseInfo {
    let get = |keys: &[&str]| keys.iter().find_map(|k| env.get(*k).filter(|v| !v.is_empty()).cloned());
    let (user, password, database) = match kind {
        DbKind::Postgres => {
            let user = get(&["POSTGRES_USER"]).unwrap_or_else(|| "postgres".into());
            let db = get(&["POSTGRES_DB"]).unwrap_or_else(|| user.clone());
            (Some(user), get(&["POSTGRES_PASSWORD"]), Some(db))
        }
        DbKind::Mysql | DbKind::Mariadb => {
            let app_user = get(&["MYSQL_USER", "MARIADB_USER"]);
            let (user, password) = match app_user {
                Some(u) => (u, get(&["MYSQL_PASSWORD", "MARIADB_PASSWORD"])),
                None => ("root".to_string(), get(&["MYSQL_ROOT_PASSWORD", "MARIADB_ROOT_PASSWORD"])),
            };
            (Some(user), password, get(&["MYSQL_DATABASE", "MARIADB_DATABASE"]))
        }
        DbKind::Redis => {
            let from_cmd = cmd.iter().position(|a| a == "--requirepass").and_then(|i| cmd.get(i + 1).cloned());
            (None, get(&["REDIS_PASSWORD"]).or(from_cmd), None)
        }
        DbKind::Mongo => (
            get(&["MONGO_INITDB_ROOT_USERNAME"]),
            get(&["MONGO_INITDB_ROOT_PASSWORD"]),
            get(&["MONGO_INITDB_DATABASE"]),
        ),
    };

    let (connection_url, env_snippet) = match host_port {
        None => (None, None),
        Some(port) => {
            let auth = match (&user, &password) {
                (Some(u), Some(p)) => format!("{}:{}@", url_encode(u), url_encode(p)),
                (Some(u), None) => format!("{}@", url_encode(u)),
                (None, Some(p)) => format!(":{}@", url_encode(p)),
                (None, None) => String::new(),
            };
            let path = database.as_deref().map(|d| format!("/{}", url_encode(d))).unwrap_or_default();
            let url = match kind {
                DbKind::Postgres => format!("postgresql://{auth}localhost:{port}{path}"),
                DbKind::Mysql | DbKind::Mariadb => format!("mysql://{auth}localhost:{port}{path}"),
                DbKind::Redis => format!("redis://{auth}localhost:{port}"),
                DbKind::Mongo => {
                    let query = if user.is_some() { "?authSource=admin" } else { "" };
                    let path = if path.is_empty() && !query.is_empty() { "/".to_string() } else { path };
                    format!("mongodb://{auth}localhost:{port}{path}{query}")
                }
            };
            let mut lines = vec![];
            match kind {
                DbKind::Redis => {
                    lines.push(format!("REDIS_URL={}", env_quote(&url)));
                    lines.push("REDIS_HOST=localhost".into());
                    lines.push(format!("REDIS_PORT={port}"));
                    if let Some(p) = &password {
                        lines.push(format!("REDIS_PASSWORD={}", env_quote(p)));
                    }
                }
                DbKind::Mongo => lines.push(format!("MONGODB_URI={}", env_quote(&url))),
                _ => {
                    lines.push(format!("DATABASE_URL={}", env_quote(&url)));
                    lines.push("DB_HOST=localhost".into());
                    lines.push(format!("DB_PORT={port}"));
                    if let Some(u) = &user {
                        lines.push(format!("DB_USER={}", env_quote(u)));
                    }
                    if let Some(p) = &password {
                        lines.push(format!("DB_PASSWORD={}", env_quote(p)));
                    }
                    if let Some(d) = &database {
                        lines.push(format!("DB_NAME={}", env_quote(d)));
                    }
                }
            }
            (Some(url), Some(lines.join("\n")))
        }
    };

    DatabaseInfo {
        kind,
        user,
        password,
        database,
        host_port,
        connection_url,
        env_snippet,
        explorer: !matches!(kind, DbKind::Mongo),
    }
}

/// What the explorer and the cards need from `docker inspect`.
#[derive(Debug, Clone)]
pub(crate) struct Inspected {
    pub id: String,
    pub running: bool,
    pub kind: Option<DbKind>,
    pub env: HashMap<String, String>,
    /// Named volumes mounted in the container.
    pub volumes: Vec<String>,
    pub database: Option<DatabaseInfo>,
}

/// Host port published for `internal` ("5432/tcp"): the binding survives a stop.
fn published_port(v: &Value, internal: u16) -> Option<u16> {
    let key = format!("{internal}/tcp");
    [&v["HostConfig"]["PortBindings"][&key], &v["NetworkSettings"]["Ports"][&key]]
        .into_iter()
        .filter_map(Value::as_array)
        .flatten()
        .find_map(|b| b["HostPort"].as_str().and_then(|p| p.parse::<u16>().ok()).filter(|p| *p != 0))
}

pub(crate) fn parse_inspect(v: &Value) -> Option<Inspected> {
    let id = v["Id"].as_str()?.to_string();
    let env: HashMap<String, String> = v["Config"]["Env"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter_map(|kv| kv.split_once('='))
        .map(|(k, val)| (k.to_string(), val.to_string()))
        .collect();
    let cmd: Vec<String> = v["Config"]["Cmd"].as_array().into_iter().flatten().filter_map(Value::as_str).map(String::from).collect();
    let image = v["Config"]["Image"].as_str().unwrap_or("");
    let kind = v["Config"]["Labels"][KIND_LABEL].as_str().and_then(DbKind::from_label).or_else(|| db_kind_from_image(image));
    let volumes = v["Mounts"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|m| m["Type"].as_str() == Some("volume"))
        .filter_map(|m| m["Name"].as_str().map(String::from))
        .collect();
    let database = kind.map(|k| database_info(k, &env, &cmd, published_port(v, k.internal_port())));
    Some(Inspected {
        id,
        running: v["State"]["Running"].as_bool().unwrap_or(false),
        kind,
        env,
        volumes,
        database,
    })
}

pub(crate) async fn inspect(engine: &Engine, ids: &[&str]) -> Result<Vec<Inspected>, String> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let mut args = vec!["inspect"];
    args.extend_from_slice(ids);
    let text = engine.text(&args, LIST_TIMEOUT).await?;
    let list: Vec<Value> = serde_json::from_str(text.trim()).map_err(|e| format!("Resposta inesperada do docker inspect: {e}"))?;
    Ok(list.iter().filter_map(parse_inspect).collect())
}

/// Container names and ids that reach `docker` as arguments.
pub fn validate_container_ref(s: &str) -> Result<(), String> {
    let ok = !s.is_empty()
        && s.len() <= 128
        && s.chars().next().is_some_and(|c| c.is_ascii_alphanumeric())
        && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'));
    if ok { Ok(()) } else { Err(format!("Container inválido: {s:?}")) }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContainerInfo {
    pub id: String,
    pub short_id: String,
    pub name: String,
    pub image: String,
    /// running · exited · created · paused · restarting · dead
    pub state: String,
    /// Docker's own text ("Up 5 minutes"), shown in tooltips.
    pub status: String,
    pub ports: Vec<PortMapping>,
    pub created_at: String,
    /// Created by this app (1-click database).
    pub managed: bool,
    pub compose_project: Option<String>,
    pub volumes: Vec<String>,
    pub stats: Option<ContainerStats>,
    pub database: Option<DatabaseInfo>,
}

#[tauri::command]
pub async fn list_containers(distro: String, docker: State<'_, DockerManager>) -> Result<Vec<ContainerInfo>, String> {
    let engine = docker.engine(&distro).await?;
    list(&engine).await
}

pub(crate) async fn list(engine: &Engine) -> Result<Vec<ContainerInfo>, String> {
    let text = engine.text(&["ps", "-a", "--no-trunc", "--format", "{{json .}}"], LIST_TIMEOUT).await?;
    let rows: Vec<PsRow> = text.lines().filter_map(parse_ps_line).collect();

    let running: Vec<&str> = rows.iter().filter(|r| r.state == "running").map(|r| r.id.as_str()).collect();
    let databases: Vec<&str> = rows
        .iter()
        .filter(|r| r.labels.contains_key(KIND_LABEL) || db_kind_from_image(&r.image).is_some())
        .map(|r| r.id.as_str())
        .collect();
    let stats = async {
        if running.is_empty() {
            return HashMap::new();
        }
        let mut args = vec!["stats", "--no-stream", "--format", "{{json .}}"];
        args.extend_from_slice(&running);
        // Usage is a nice-to-have: a slow or failing `stats` never hides the list.
        match engine.text(&args, Duration::from_secs(20)).await {
            Ok(text) => text.lines().filter_map(parse_stats_line).collect::<HashMap<_, _>>(),
            Err(_) => HashMap::new(),
        }
    };
    let (mut stats, inspected) = tokio::join!(stats, inspect(engine, &databases));
    let mut inspected: HashMap<String, Inspected> = inspected.unwrap_or_default().into_iter().map(|i| (i.id.clone(), i)).collect();

    let mut out: Vec<ContainerInfo> = rows
        .into_iter()
        .map(|r| {
            let info = inspected.remove(&r.id);
            ContainerInfo {
                short_id: r.id.chars().take(12).collect(),
                managed: r.labels.get(MANAGED_LABEL).is_some_and(|v| v == "true"),
                compose_project: r.labels.get("com.docker.compose.project").cloned(),
                stats: stats.remove(&r.name),
                volumes: info.as_ref().map(|i| i.volumes.clone()).unwrap_or(r.mounts),
                database: info.and_then(|i| i.database),
                id: r.id,
                name: r.name,
                image: r.image,
                state: r.state,
                status: r.status,
                ports: r.ports,
                created_at: r.created_at,
            }
        })
        .collect();
    // Databases first, then running ones, then by name.
    out.sort_by(|a, b| {
        (b.database.is_some(), b.state == "running", &a.name).cmp(&(a.database.is_some(), a.state == "running", &b.name))
    });
    Ok(out)
}

#[tauri::command]
pub async fn start_container(distro: String, id: String, docker: State<'_, DockerManager>) -> Result<(), String> {
    lifecycle(&docker, &distro, "start", &id).await
}

#[tauri::command]
pub async fn stop_container(distro: String, id: String, docker: State<'_, DockerManager>) -> Result<(), String> {
    lifecycle(&docker, &distro, "stop", &id).await
}

#[tauri::command]
pub async fn restart_container(distro: String, id: String, docker: State<'_, DockerManager>) -> Result<(), String> {
    lifecycle(&docker, &distro, "restart", &id).await
}

async fn lifecycle(docker: &DockerManager, distro: &str, action: &str, id: &str) -> Result<(), String> {
    validate_container_ref(id)?;
    let engine = docker.engine(distro).await?;
    lifecycle_engine(&engine, action, id).await
}

pub(crate) async fn lifecycle_engine(engine: &Engine, action: &str, id: &str) -> Result<(), String> {
    engine.text(&[action, id], ACTION_TIMEOUT).await.map(drop)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoveResult {
    pub removed_volumes: Vec<String>,
    /// Volumes that stayed (kept on purpose, or still used by another container).
    pub kept_volumes: Vec<String>,
}

#[tauri::command]
pub async fn remove_container(
    distro: String,
    id: String,
    remove_volume: bool,
    docker: State<'_, DockerManager>,
) -> Result<RemoveResult, String> {
    validate_container_ref(&id)?;
    let engine = docker.engine(&distro).await?;
    remove(&engine, &id, remove_volume).await
}

pub(crate) async fn remove(engine: &Engine, id: &str, remove_volume: bool) -> Result<RemoveResult, String> {
    let volumes = inspect(engine, &[id]).await?.into_iter().next().map(|i| i.volumes).unwrap_or_default();
    // -v also drops anonymous volumes (never reachable again anyway).
    engine.text(&["rm", "-f", "-v", id], ACTION_TIMEOUT).await?;
    let mut result = RemoveResult { removed_volumes: Vec::new(), kept_volumes: Vec::new() };
    for volume in volumes {
        if remove_volume && engine.text(&["volume", "rm", &volume], ACTION_TIMEOUT).await.is_ok() {
            result.removed_volumes.push(volume);
        } else {
            result.kept_volumes.push(volume);
        }
    }
    Ok(result)
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LogLine {
    /// RFC 3339 (UTC), as Docker wrote it.
    pub time: String,
    pub text: String,
    pub error: bool,
}

/// `docker logs --timestamps` keeps stdout and stderr apart: merge them back by time.
pub fn merge_logs(stdout: &str, stderr: &str) -> Vec<LogLine> {
    let parse = |text: &str, error: bool| -> Vec<LogLine> {
        text.lines()
            .map(|line| match line.split_once(' ') {
                Some((time, rest)) if time.len() >= 20 && time.as_bytes()[4] == b'-' => {
                    LogLine { time: time.to_string(), text: rest.to_string(), error }
                }
                _ => LogLine { time: String::new(), text: line.to_string(), error },
            })
            .collect()
    };
    let mut lines = parse(stdout, false);
    lines.extend(parse(stderr, true));
    // Same format and zone on both streams: the text order is the time order.
    lines.sort_by(|a, b| a.time.cmp(&b.time));
    lines
}

#[tauri::command]
pub async fn get_container_logs(
    distro: String,
    id: String,
    tail: usize,
    docker: State<'_, DockerManager>,
) -> Result<Vec<LogLine>, String> {
    validate_container_ref(&id)?;
    let engine = docker.engine(&distro).await?;
    let tail = tail.clamp(10, 2000).to_string();
    let out = engine.run(&["logs", "--tail", &tail, "--timestamps", &id], LIST_TIMEOUT, None, &[]).await?;
    if !out.success() {
        return Err(explain_docker_error(&out.error_text()));
    }
    Ok(merge_logs(&String::from_utf8_lossy(&out.stdout), &String::from_utf8_lossy(&out.stderr)))
}

// ---------------------------------------------------------------------------
// 1-click databases
// ---------------------------------------------------------------------------

struct Preset {
    image: &'static str,
    data_dir: &'static str,
}

fn preset(kind: DbKind) -> Preset {
    match kind {
        DbKind::Postgres => Preset { image: "postgres:16-alpine", data_dir: "/var/lib/postgresql/data" },
        DbKind::Mysql => Preset { image: "mysql:8.4", data_dir: "/var/lib/mysql" },
        DbKind::Mariadb => Preset { image: "mariadb:11", data_dir: "/var/lib/mysql" },
        DbKind::Redis => Preset { image: "redis:7-alpine", data_dir: "/data" },
        DbKind::Mongo => Preset { image: "mongo:7", data_dir: "/data/db" },
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewDatabase {
    pub kind: DbKind,
    /// Container name.
    pub name: String,
    pub database: Option<String>,
    pub user: Option<String>,
    pub password: Option<String>,
    pub port: u16,
}

/// Container names: Docker's own rule, shorter.
pub fn validate_container_name(name: &str) -> Result<(), String> {
    let ok = name.len() >= 2
        && name.len() <= 60
        && name.chars().next().is_some_and(|c| c.is_ascii_alphanumeric())
        && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'));
    if ok {
        Ok(())
    } else {
        Err("Nome inválido: use de 2 a 60 letras sem acento, números, hífen, ponto ou _, começando por letra ou número.".into())
    }
}

/// Database and user names: safe unquoted in every engine and in URLs.
pub fn validate_db_identifier(value: &str, what: &str) -> Result<(), String> {
    let ok = !value.is_empty()
        && value.len() <= 63
        && value.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && value.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    if ok { Ok(()) } else { Err(format!("{what} inválido: use letras sem acento, números e _, começando por letra.")) }
}

/// Passwords end up in URLs and `.env` files: printable ASCII without quotes, spaces, `\`, `$`
/// or backticks (they break shells and `.env` parsers).
pub fn validate_password(value: &str) -> Result<(), String> {
    let ok = !value.is_empty()
        && value.len() <= 128
        && value.chars().all(|c| c.is_ascii_graphic() && !matches!(c, '"' | '\'' | '\\' | '$' | '`'));
    if ok { Ok(()) } else { Err("Senha inválida: use de 1 a 128 caracteres sem espaços, aspas, \\, $ ou crase.".into()) }
}

pub fn volume_name(container: &str) -> String {
    format!("symbiosis_{container}_data")
}

/// `docker run` arguments (secrets travel apart, see [`Engine::run`]).
pub fn run_args(req: &NewDatabase) -> Result<(Vec<String>, Vec<(String, String)>), String> {
    validate_container_name(&req.name)?;
    if req.port < 1024 {
        return Err("Escolha uma porta entre 1024 e 65535.".into());
    }
    let p = preset(req.kind);
    let mut args: Vec<String> = vec![
        "run".into(),
        "-d".into(),
        "--name".into(),
        req.name.clone(),
        "--label".into(),
        format!("{MANAGED_LABEL}=true"),
        "--label".into(),
        format!("{KIND_LABEL}={}", req.kind.label()),
        "--restart".into(),
        "unless-stopped".into(),
        "-p".into(),
        format!("127.0.0.1:{}:{}", req.port, req.kind.internal_port()),
        "-v".into(),
        format!("{}:{}", volume_name(&req.name), p.data_dir),
    ];
    let mut secrets: Vec<(String, String)> = Vec::new();
    let plain = |args: &mut Vec<String>, k: &str, v: &str| {
        args.push("-e".into());
        args.push(format!("{k}={v}"));
    };
    let mut secret = |args: &mut Vec<String>, k: &str, v: &str| {
        args.push("-e".into());
        args.push(k.into());
        secrets.push((k.into(), v.into()));
    };
    let password = req.password.as_deref().filter(|p| !p.is_empty());
    if let Some(pw) = password {
        validate_password(pw)?;
    }
    let db = req.database.as_deref().filter(|d| !d.is_empty());
    let user = req.user.as_deref().filter(|u| !u.is_empty());
    if let Some(d) = db {
        validate_db_identifier(d, "Nome do banco")?;
    }
    if let Some(u) = user {
        validate_db_identifier(u, "Usuário")?;
    }
    let need_password = || password.ok_or_else(|| "Defina uma senha.".to_string());
    let mut command: Vec<String> = Vec::new();
    match req.kind {
        DbKind::Postgres => {
            plain(&mut args, "POSTGRES_USER", user.unwrap_or("postgres"));
            plain(&mut args, "POSTGRES_DB", db.unwrap_or("app_db"));
            secret(&mut args, "POSTGRES_PASSWORD", need_password()?);
        }
        DbKind::Mysql | DbKind::Mariadb => {
            let pw = need_password()?;
            plain(&mut args, "MYSQL_DATABASE", db.unwrap_or("app_db"));
            secret(&mut args, "MYSQL_ROOT_PASSWORD", pw);
            if let Some(u) = user.filter(|u| *u != "root") {
                plain(&mut args, "MYSQL_USER", u);
                secret(&mut args, "MYSQL_PASSWORD", pw);
            }
        }
        DbKind::Redis => {
            // appendonly: writes survive a restart (the default only snapshots now and then).
            command = vec!["redis-server".into(), "--appendonly".into(), "yes".into()];
            if let Some(pw) = password {
                secret(&mut args, "REDIS_PASSWORD", pw);
                command = ["sh", "-c", r#"exec redis-server --appendonly yes --requirepass "$REDIS_PASSWORD""#]
                    .map(String::from)
                    .to_vec();
            }
        }
        DbKind::Mongo => {
            plain(&mut args, "MONGO_INITDB_DATABASE", db.unwrap_or("app_db"));
            plain(&mut args, "MONGO_INITDB_ROOT_USERNAME", user.unwrap_or("admin"));
            secret(&mut args, "MONGO_INITDB_ROOT_PASSWORD", need_password()?);
        }
    }
    args.push(p.image.into());
    args.extend(command);
    Ok((args, secrets))
}

/// Inside the container; exit 0 = accepting connections. Postgres and MySQL are asked over TCP:
/// their first-start init runs a temporary server on the socket only, then restarts.
fn ready_script(kind: DbKind) -> &'static str {
    match kind {
        DbKind::Postgres => r#"pg_isready -q -h 127.0.0.1 -U "${POSTGRES_USER:-postgres}""#,
        DbKind::Mysql | DbKind::Mariadb => {
            r#"c=mysql; command -v mariadb >/dev/null 2>&1 && c=mariadb; "$c" -h127.0.0.1 -uroot --password="${MYSQL_ROOT_PASSWORD:-$MARIADB_ROOT_PASSWORD}" -e 'SELECT 1' >/dev/null 2>&1"#
        }
        DbKind::Redis => r#"[ -n "${REDIS_PASSWORD:-}" ] && export REDISCLI_AUTH="$REDIS_PASSWORD"; [ "$(redis-cli ping 2>/dev/null)" = PONG ]"#,
        DbKind::Mongo => r#"[ "$(mongosh --quiet --eval 'db.adminCommand({ping: 1}).ok' 2>/dev/null)" = 1 ]"#,
    }
}

pub(crate) async fn wait_ready(engine: &Engine, id: &str, kind: DbKind, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        let ok = engine
            .run(&["exec", id, "sh", "-c", ready_script(kind)], Duration::from_secs(20), None, &[])
            .await
            .is_ok_and(|o| o.success());
        if ok {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(1500)).await;
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DatabaseProgress {
    name: String,
    /// pull · create · wait
    stage: &'static str,
    detail: Option<String>,
    /// Pull only: share of the image's layers downloaded and extracted (0–100).
    percent: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreatedDatabase {
    pub id: String,
    pub name: String,
    /// Accepting connections already (false: still starting when the wait ran out).
    pub ready: bool,
    pub database: Option<DatabaseInfo>,
}

#[tauri::command]
pub async fn create_database_container(
    app: AppHandle,
    distro: String,
    request: NewDatabase,
    docker: State<'_, DockerManager>,
) -> Result<CreatedDatabase, String> {
    let _guard = take_flag(&docker.creating, "Já há um banco sendo criado. Aguarde terminar.")?;
    let engine = docker.engine(&distro).await?;
    let progress = |stage: &'static str, detail: Option<String>, percent: Option<f64>| {
        let _ = app.emit("database-progress", DatabaseProgress { name: request.name.clone(), stage, detail, percent });
    };
    create(&engine, &request, &progress).await
}

pub(crate) async fn create(
    engine: &Engine,
    request: &NewDatabase,
    progress: &(dyn Fn(&'static str, Option<String>, Option<f64>) + Sync),
) -> Result<CreatedDatabase, String> {
    let (args, secrets) = run_args(request)?;
    let volume = volume_name(&request.name);
    if engine.text(&["volume", "inspect", &volume], LIST_TIMEOUT).await.is_ok() {
        return Err(format!(
            "Já existem dados guardados de um banco antigo chamado {} (volume {volume}). Escolha outro nome para começar do zero.",
            request.name
        ));
    }
    if !port_is_free(request.port) {
        return Err(format!("A porta {} já está em uso no Windows. Escolha outra porta.", request.port));
    }

    let image = preset(request.kind).image;
    if engine.text(&["image", "inspect", "--format", "{{.Id}}", image], LIST_TIMEOUT).await.is_err() {
        progress("pull", None, Some(0.0));
        pull(engine, image, progress).await?;
    }

    progress("create", None, None);
    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let secret_refs: Vec<(&str, &str)> = secrets.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    let out = engine.run(&arg_refs, ACTION_TIMEOUT, None, &secret_refs).await?;
    if !out.success() {
        return Err(explain_docker_error(&out.error_text()));
    }
    let id = wsl::decode_output(&out.stdout).trim().to_string();
    validate_container_ref(&id)?;

    progress("wait", None, None);
    let ready = wait_ready(engine, &id, request.kind, READY_TIMEOUT).await;
    let database = inspect(engine, &[&id]).await.ok().and_then(|v| v.into_iter().next()).and_then(|i| i.database);
    Ok(CreatedDatabase { id, name: request.name.clone(), ready, database })
}

async fn pull(engine: &Engine, image: &str, progress: &(dyn Fn(&'static str, Option<String>, Option<f64>) + Sync)) -> Result<(), String> {
    use tokio::io::BufReader;
    let mut cmd = engine.command(&["pull", image], 0);
    let mut child = cmd.spawn().map_err(|e| format!("Falha ao executar o docker: {e}"))?;
    let stdout = child.stdout.take().ok_or("Sem saída do docker pull")?;
    let stderr = child.stderr.take().ok_or("Sem saída do docker pull")?;
    let task = async {
        let mut lines = crate::lines::BoundedLines::new(BufReader::new(stdout), crate::lines::MAX_LOG_LINE_BYTES);
        let mut last = Instant::now() - Duration::from_secs(1);
        let mut layers = PullTracker::default();
        let read_out = async {
            while let Ok(Some(line)) = lines.next_line().await {
                layers.feed(&line);
                // One update every ~300 ms is plenty for a status line.
                if last.elapsed() > Duration::from_millis(300) {
                    last = Instant::now();
                    progress("pull", Some(line.trim().to_string()), layers.percent());
                }
            }
        };
        let (_, err) = tokio::join!(read_out, wsl::read_capped(stderr, 64 * 1024));
        let status = child.wait().await.map_err(|e| e.to_string())?;
        if status.success() {
            Ok(())
        } else {
            Err(explain_docker_error(&String::from_utf8_lossy(&err.unwrap_or_default())))
        }
    };
    tokio::time::timeout(PULL_TIMEOUT, task)
        .await
        .map_err(|_| "O download da imagem demorou demais. Verifique a internet e tente de novo.".to_string())?
}

/// Progress of a `docker pull` from its plain (non-TTY) output: one line per layer event,
/// `<12 hex>: Pulling fs layer | Downloading | Download complete | Extracting | Pull complete`.
/// Each layer counts half when downloaded and whole when extracted (or already present).
#[derive(Default)]
pub struct PullTracker {
    layers: HashMap<String, u8>,
}

impl PullTracker {
    pub fn feed(&mut self, line: &str) {
        let Some((id, status)) = line.trim().split_once(": ") else { return };
        if id.len() != 12 || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
            return;
        }
        let s = status.trim();
        let level = if s.starts_with("Pull complete") || s.starts_with("Already exists") {
            2
        } else if s.starts_with("Download complete") || s.starts_with("Verifying Checksum") || s.starts_with("Extracting") {
            1
        } else {
            0
        };
        let entry = self.layers.entry(id.to_string()).or_insert(0);
        *entry = (*entry).max(level);
    }

    /// 0–100, or None before any layer was listed.
    pub fn percent(&self) -> Option<f64> {
        if self.layers.is_empty() {
            return None;
        }
        let done: u32 = self.layers.values().map(|l| *l as u32).sum();
        Some(done as f64 * 100.0 / (self.layers.len() as f64 * 2.0))
    }
}

/// Nothing answers on 127.0.0.1:`port` and it can be bound. (A connect test as well: on Windows
/// a socket listening on 0.0.0.0 doesn't stop a bind on 127.0.0.1.) Only loopback is bound, so
/// the firewall never asks anything.
pub fn port_is_free(port: u16) -> bool {
    use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    if TcpStream::connect_timeout(&addr, Duration::from_millis(250)).is_ok() {
        return false;
    }
    TcpListener::bind(addr).is_ok()
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DatabaseDefaults {
    pub name: String,
    pub port: u16,
    pub image: String,
    pub database: Option<String>,
    pub user: Option<String>,
    pub password: Option<String>,
}

/// `base`, `base-2`, `base-3`… not used by any container (or leftover data volume).
pub fn free_name(base: &str, taken: &HashSet<String>) -> String {
    (1..)
        .map(|i| if i == 1 { base.to_string() } else { format!("{base}-{i}") })
        .find(|n| !taken.contains(n) && !taken.contains(&volume_name(n)))
        .unwrap()
}

/// Suggested form values: a free name and the first free port from the engine's default.
#[tauri::command]
pub async fn get_database_defaults(
    distro: String,
    kind: DbKind,
    docker: State<'_, DockerManager>,
) -> Result<DatabaseDefaults, String> {
    let engine = docker.engine(&distro).await?;
    let containers = list(&engine).await?;
    let volumes = engine.text(&["volume", "ls", "--format", "{{.Name}}"], LIST_TIMEOUT).await.unwrap_or_default();
    let mut taken: HashSet<String> = containers.iter().map(|c| c.name.clone()).collect();
    taken.extend(volumes.lines().map(|l| l.trim().to_string()));
    // Ports bound by any container (stopped ones too: they'd clash when started).
    let used: HashSet<u16> = containers
        .iter()
        .flat_map(|c| c.ports.iter().map(|p| p.host_port).chain(c.database.as_ref().and_then(|d| d.host_port)))
        .collect();
    let base = kind.internal_port();
    let port = tokio::task::spawn_blocking(move || {
        (base..base.saturating_add(100)).find(|p| !used.contains(p) && port_is_free(*p)).unwrap_or(base)
    })
    .await
    .unwrap_or(base);
    let (database, user, password) = match kind {
        DbKind::Postgres => (Some("app_db"), Some("postgres"), Some("postgres")),
        DbKind::Mysql | DbKind::Mariadb => (Some("app_db"), Some("root"), Some("root")),
        DbKind::Redis => (None, None, None),
        DbKind::Mongo => (Some("app_db"), Some("admin"), Some("admin")),
    };
    Ok(DatabaseDefaults {
        name: free_name(&format!("symbiosis-{}", kind.label()), &taken),
        port,
        image: preset(kind).image.to_string(),
        database: database.map(String::from),
        user: user.map(String::from),
        password: password.map(String::from),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ps_lines() {
        let line = r#"{"Command":"\"docker-entrypoint.s…\"","CreatedAt":"2026-11-01 10:00:00 -0300 -03","ID":"3f1c9a7b2d4e5f60718293a4b5c6d7e8f90123456789abcdef0123456789abcd","Image":"postgres:16-alpine","Labels":"com.wslsymbiosis.managed=true,com.wslsymbiosis.kind=postgres","LocalVolumes":"1","Mounts":"symbiosis_pg_data","Names":"symbiosis-postgres","Networks":"bridge","Ports":"127.0.0.1:5432->5432/tcp","RunningFor":"2 minutes ago","Size":"0B","State":"running","Status":"Up 2 minutes"}"#;
        let row = parse_ps_line(line).unwrap();
        assert_eq!(row.name, "symbiosis-postgres");
        assert_eq!(row.state, "running");
        assert_eq!(row.labels.get(MANAGED_LABEL).map(String::as_str), Some("true"));
        assert_eq!(row.mounts, vec!["symbiosis_pg_data"]);
        assert_eq!(
            row.ports,
            vec![PortMapping { host_ip: "127.0.0.1".into(), host_port: 5432, container_port: 5432, protocol: "tcp".into() }]
        );
        assert!(parse_ps_line("not json").is_none());
        assert!(parse_ps_line("{}").is_none());
    }

    #[test]
    fn parses_port_lists() {
        let ports = parse_ports("0.0.0.0:6379->6379/tcp, [::]:6379->6379/tcp, :::6379->6379/tcp, 8080/tcp, 0.0.0.0:8000-8001->8000-8001/udp");
        assert_eq!(ports.len(), 2);
        assert_eq!(ports[0].host_port, 6379);
        assert_eq!(ports[0].host_ip, "0.0.0.0");
        assert_eq!((ports[1].host_port, ports[1].protocol.as_str()), (8000, "udp"));
        assert!(parse_ports("").is_empty());
    }

    #[test]
    fn parses_stats_lines() {
        let line = r#"{"BlockIO":"0B / 0B","CPUPerc":"1.25%","Container":"abc","ID":"abc","MemPerc":"0.30%","MemUsage":"23.5MiB / 7.607GiB","Name":"symbiosis-postgres","NetIO":"1kB / 0B","PIDs":"6"}"#;
        let (name, s) = parse_stats_line(line).unwrap();
        assert_eq!(name, "symbiosis-postgres");
        assert_eq!(s.cpu_percent, 1.25);
        assert_eq!(s.memory_used, (23.5 * 1024.0 * 1024.0) as u64);
        assert_eq!(s.memory_limit, (7.607 * 1024.0 * 1024.0 * 1024.0_f64).round() as u64);
        assert_eq!(parse_size("0B"), Some(0));
        assert_eq!(parse_size("1.5kB"), Some(1500));
        assert_eq!(parse_size("--"), None);
    }

    #[test]
    fn detects_database_images() {
        assert_eq!(db_kind_from_image("postgres:16-alpine"), Some(DbKind::Postgres));
        assert_eq!(db_kind_from_image("docker.io/library/mysql:8.4"), Some(DbKind::Mysql));
        assert_eq!(db_kind_from_image("localhost:5000/mariadb:11"), Some(DbKind::Mariadb));
        assert_eq!(db_kind_from_image("redis@sha256:abcdef"), Some(DbKind::Redis));
        assert_eq!(db_kind_from_image("bitnami/mongodb:7"), Some(DbKind::Mongo));
        assert_eq!(db_kind_from_image("supabase/postgres:15.1"), Some(DbKind::Postgres));
        assert_eq!(db_kind_from_image("nginx:latest"), None);
        assert_eq!(db_kind_from_image("node:20"), None);
    }

    fn env(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn builds_connection_urls() {
        let pg = database_info(
            DbKind::Postgres,
            &env(&[("POSTGRES_USER", "postgres"), ("POSTGRES_PASSWORD", "p@ss:w/rd"), ("POSTGRES_DB", "app_db")]),
            &[],
            Some(5432),
        );
        assert_eq!(pg.connection_url.as_deref(), Some("postgresql://postgres:p%40ss%3Aw%2Frd@localhost:5432/app_db"));
        let snippet = pg.env_snippet.unwrap();
        assert!(snippet.starts_with("DATABASE_URL=\"postgresql://"));
        assert!(snippet.contains("DB_PASSWORD=\"p@ss:w/rd\""));

        // Defaults of the official image: user postgres, database = user.
        let bare = database_info(DbKind::Postgres, &env(&[("POSTGRES_PASSWORD", "x")]), &[], Some(55432));
        assert_eq!(bare.connection_url.as_deref(), Some("postgresql://postgres:x@localhost:55432/postgres"));

        let my = database_info(
            DbKind::Mysql,
            &env(&[("MYSQL_ROOT_PASSWORD", "root"), ("MYSQL_DATABASE", "loja")]),
            &[],
            Some(3307),
        );
        assert_eq!(my.connection_url.as_deref(), Some("mysql://root:root@localhost:3307/loja"));
        let my_app = database_info(
            DbKind::Mariadb,
            &env(&[("MARIADB_ROOT_PASSWORD", "r"), ("MARIADB_USER", "app"), ("MARIADB_PASSWORD", "a1"), ("MARIADB_DATABASE", "loja")]),
            &[],
            Some(3306),
        );
        assert_eq!(my_app.connection_url.as_deref(), Some("mysql://app:a1@localhost:3306/loja"));

        let redis = database_info(DbKind::Redis, &env(&[]), &[], Some(6379));
        assert_eq!(redis.connection_url.as_deref(), Some("redis://localhost:6379"));
        let cmd: Vec<String> = ["redis-server", "--requirepass", "s3cr3t"].map(String::from).to_vec();
        let redis_pw = database_info(DbKind::Redis, &env(&[]), &cmd, Some(6380));
        assert_eq!(redis_pw.connection_url.as_deref(), Some("redis://:s3cr3t@localhost:6380"));
        assert!(redis_pw.env_snippet.unwrap().starts_with("REDIS_URL="));

        let mongo = database_info(
            DbKind::Mongo,
            &env(&[("MONGO_INITDB_ROOT_USERNAME", "admin"), ("MONGO_INITDB_ROOT_PASSWORD", "admin"), ("MONGO_INITDB_DATABASE", "app_db")]),
            &[],
            Some(27017),
        );
        assert_eq!(mongo.connection_url.as_deref(), Some("mongodb://admin:admin@localhost:27017/app_db?authSource=admin"));
        assert!(!mongo.explorer);
        let mongo_open = database_info(DbKind::Mongo, &env(&[]), &[], Some(27017));
        assert_eq!(mongo_open.connection_url.as_deref(), Some("mongodb://localhost:27017"));

        // No published port: nothing to connect to from Windows.
        assert_eq!(database_info(DbKind::Postgres, &env(&[]), &[], None).connection_url, None);
    }

    #[test]
    fn reads_inspect_output() {
        let v: Value = serde_json::from_str(
            r#"{"Id":"abc123","Name":"/meu-pg","State":{"Running":false},
                "Config":{"Image":"postgres:16","Env":["POSTGRES_PASSWORD=x","PATH=/usr/bin"],"Cmd":["postgres"],"Labels":{}},
                "HostConfig":{"PortBindings":{"5432/tcp":[{"HostIp":"127.0.0.1","HostPort":"5433"}]}},
                "NetworkSettings":{"Ports":{}},
                "Mounts":[{"Type":"volume","Name":"pgdata"},{"Type":"bind","Source":"C:\\x"}]}"#,
        )
        .unwrap();
        let i = parse_inspect(&v).unwrap();
        assert!(!i.running);
        assert_eq!(i.kind, Some(DbKind::Postgres));
        assert_eq!(i.volumes, vec!["pgdata"]);
        assert_eq!(i.database.unwrap().host_port, Some(5433));
    }

    #[test]
    fn builds_run_arguments_without_secrets_in_argv() {
        let req = NewDatabase {
            kind: DbKind::Postgres,
            name: "symbiosis-postgres".into(),
            database: Some("app_db".into()),
            user: Some("postgres".into()),
            password: Some("segredo123".into()),
            port: 5432,
        };
        let (args, secrets) = run_args(&req).unwrap();
        assert!(!args.iter().any(|a| a.contains("segredo123")), "password in argv: {args:?}");
        assert_eq!(secrets, vec![("POSTGRES_PASSWORD".to_string(), "segredo123".to_string())]);
        let joined = args.join(" ");
        assert!(joined.contains("-p 127.0.0.1:5432:5432"));
        assert!(joined.contains("-v symbiosis_symbiosis-postgres_data:/var/lib/postgresql/data"));
        assert!(joined.contains("--label com.wslsymbiosis.managed=true"));
        assert!(joined.contains("-e POSTGRES_PASSWORD postgres:16-alpine"));

        let redis = NewDatabase { kind: DbKind::Redis, name: "cache".into(), database: None, user: None, password: Some("pw".into()), port: 6380 };
        let (args, secrets) = run_args(&redis).unwrap();
        assert!(!args.iter().any(|a| a == "pw"));
        assert_eq!(secrets.len(), 1);
        assert_eq!(args.last().unwrap(), r#"exec redis-server --appendonly yes --requirepass "$REDIS_PASSWORD""#);

        let mysql = NewDatabase { kind: DbKind::Mysql, name: "loja".into(), database: Some("loja".into()), user: Some("app".into()), password: Some("pw".into()), port: 3306 };
        let (args, secrets) = run_args(&mysql).unwrap();
        assert!(args.contains(&"MYSQL_USER=app".to_string()));
        assert_eq!(secrets.len(), 2);

        let bad = |f: fn(&mut NewDatabase)| {
            let mut r = req.clone();
            f(&mut r);
            run_args(&r).is_err()
        };
        assert!(bad(|r| r.name = "-rm".into()));
        assert!(bad(|r| r.name = "a b".into()));
        assert!(bad(|r| r.password = Some("has space".into())));
        assert!(bad(|r| r.password = Some("x\"y".into())));
        assert!(bad(|r| r.password = None));
        assert!(bad(|r| r.database = Some("app-db".into())));
        assert!(bad(|r| r.user = Some("1user".into())));
        assert!(bad(|r| r.port = 80));
    }

    #[test]
    fn suggests_free_names() {
        let taken: HashSet<String> = ["symbiosis-postgres", "symbiosis_symbiosis-postgres-2_data"].map(String::from).into();
        assert_eq!(free_name("symbiosis-postgres", &taken), "symbiosis-postgres-3");
        assert_eq!(free_name("symbiosis-redis", &taken), "symbiosis-redis");
    }

    #[test]
    fn merges_log_streams_by_time() {
        let out = "2026-11-01T10:00:01.000000000Z ready\n2026-11-01T10:00:03.000000000Z listening";
        let err = "2026-11-01T10:00:02.000000000Z warning: x";
        let lines = merge_logs(out, err);
        assert_eq!(lines.iter().map(|l| l.text.as_str()).collect::<Vec<_>>(), vec!["ready", "warning: x", "listening"]);
        assert!(lines[1].error);
    }

    #[test]
    fn validates_container_refs() {
        assert!(validate_container_ref("3f1c9a7b2d4e").is_ok());
        assert!(validate_container_ref("symbiosis-postgres").is_ok());
        assert!(validate_container_ref("-f").is_err());
        assert!(validate_container_ref("a;rm").is_err());
        assert!(validate_container_ref("").is_err());
    }

    #[test]
    fn reads_docker_desktop_crash_reports() {
        // Shape of the real backend.error.json (4.67, stale dockerInference socket), trimmed.
        let v: Value = serde_json::from_str(
            r#"{"type":"*fmt.wrapError","error":{"type":"*errors.withStack","error":{"type":"*errors.withMessage","error":{
                "type":"*errordef.dockerDesktopError","component":"services",
                "originalError":{"type":"*errors.fundamental","description":"listening on unix://C:\\Users\\kyo\\AppData\\Local\\Docker\\run\\dockerInference: remove C:\\Users\\kyo\\AppData\\Local\\Docker\\run\\dockerInference: The file cannot be accessed by the system."},
                "wrappedError":{"type":"*fs.PathError","error":{"type":"syscall.Errno","description":"The file cannot be accessed by the system."}}
              },"message":"initializing Inference manager"}},"message":"starting services"}"#,
        )
        .unwrap();
        let text = desktop_error_text(&v);
        assert!(text.starts_with("listening on unix://C:\\Users\\kyo"), "{text}");
        let msg = explain_desktop_crash(&text);
        assert!(msg.starts_with("O Docker Desktop encontrou um erro e não ligou."));
        assert!(msg.contains("problema conhecido"));
        assert!(msg.ends_with(r"C:\Users\kyo\AppData\Local\Docker\run\dockerInference"), "{msg}");

        let other = explain_desktop_crash("WSL integration failed");
        assert!(other.contains("janela de erro do Docker Desktop") && !other.contains("problema conhecido"));
        assert_eq!(desktop_error_text(&serde_json::json!({"message": "starting services"})), "starting services");
    }

    #[test]
    fn tracks_pull_progress_by_layer() {
        let mut t = PullTracker::default();
        assert_eq!(t.percent(), None);
        for line in [
            "16-alpine: Pulling from library/postgres",
            "a1b2c3d4e5f6: Pulling fs layer",
            "0123456789ab: Already exists",
            "fedcba987654: Waiting",
            "Digest: sha256:abc",
        ] {
            t.feed(line);
        }
        assert_eq!(t.percent(), Some(100.0 / 3.0));
        t.feed("a1b2c3d4e5f6: Download complete");
        t.feed("a1b2c3d4e5f6: Pull complete");
        t.feed("fedcba987654: Download complete");
        assert_eq!(t.percent(), Some(500.0 / 6.0));
        t.feed("fedcba987654: Pulling fs layer"); // never goes back
        t.feed("fedcba987654: Pull complete");
        assert_eq!(t.percent(), Some(100.0));
    }

    #[test]
    fn explains_docker_errors() {
        assert!(explain_docker_error("failed to connect to the docker API at npipe:////./pipe/dockerDesktopLinuxEngine").starts_with("O Docker está desligado"));
        assert!(explain_docker_error("Bind for 127.0.0.1:5432 failed: port is already allocated").starts_with("Essa porta"));
        assert_eq!(explain_docker_error("something else"), "something else");
    }
}
