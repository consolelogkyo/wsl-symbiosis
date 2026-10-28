//! Git in the Explorer: a repository's branches, switching branch, fetch and pull.
//!
//! Reading branches is local and fast: plain `sh` as the default user. Switch, fetch and
//! pull run in the user's login shell (`packages::run_user_script`), because hooks (husky
//! runs `npm install` on post-merge) need nvm, and ssh/credential setup may live in the rc
//! files. Nothing may prompt: there is no terminal behind these processes, so prompts are
//! disabled (`GIT_TERMINAL_PROMPT=0`, ssh `BatchMode`) and fail with a clear message instead.
//!
//! Pull is always `--ff-only`: it never creates merge commits or conflicts. A diverged
//! branch is reported and left for the user to resolve in the terminal.

use std::time::Duration;

use serde::Serialize;
use tauri::State;

use crate::explorer::{normalize_linux_path, validate_entry_name};
use crate::packages::run_user_script;
use crate::settings::{GitAuth, SettingsStore};
use crate::wsl::{ensure_running, run, sh_in_distro};

const READ_TIMEOUT: Duration = Duration::from_secs(20);
/// Checkout hooks may run a while.
const SWITCH_TIMEOUT: Duration = Duration::from_secs(120);
/// Network + post-merge hooks (an `npm install` after pulling a lockfile change).
const NETWORK_TIMEOUT: Duration = Duration::from_secs(300);
/// Big repositories take a while to download.
const CLONE_TIMEOUT: Duration = Duration::from_secs(900);

// ---------------------------------------------------------------------------
// Types (mirrored in src/lib/git.ts)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LocalBranch {
    pub name: String,
    /// `origin/dev`
    pub upstream: Option<String>,
    pub ahead: u32,
    pub behind: u32,
    /// The upstream was deleted on the remote.
    pub gone: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GitBranches {
    /// Checked-out branch; `None` when detached.
    pub current: Option<String>,
    /// Short commit when detached.
    pub detached: Option<String>,
    pub local: Vec<LocalBranch>,
    /// Remote branches without a local branch of the same name (`origin/prod`).
    pub remote: Vec<String>,
    /// Modified/staged tracked files (what can block a switch or a pull).
    pub changes: u32,
    pub untracked: u32,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PullResult {
    pub branch: String,
    /// New commits pulled; 0 = already up to date.
    pub commits: u32,
    /// `3 files changed, 10 insertions(+), 2 deletions(-)`
    pub stat: String,
    /// An HTTPS remote was reached over SSH (auth mode `ssh`, or the `auto` fallback).
    pub via_ssh: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CloneResult {
    /// The new repository folder.
    pub path: String,
    pub name: String,
    pub via_ssh: bool,
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

/// Network remotes only: `https://`, `http://`, `ssh://`, `git://` or scp-like
/// `user@host:org/repo.git`. Local paths, `file://` and `ext::` (runs a command) are refused.
pub fn validate_clone_url(url: &str) -> Result<(), String> {
    let invalid = || Err("Link inválido: use o endereço HTTPS (https://...) ou SSH (git@...:...) do repositório".to_string());
    if url.is_empty() || url.len() > 2048 || url.starts_with('-') || url.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return invalid();
    }
    // Compared as bytes: `&url[..n]` panics (and `panic = "abort"` kills the app) when byte n
    // falls inside a multi-byte character, e.g. `httpséx://`.
    let with_scheme = ["https://", "http://", "ssh://", "git://"]
        .iter()
        .any(|s| url.len() > s.len() && url.as_bytes()[..s.len()].eq_ignore_ascii_case(s.as_bytes()));
    let scp_like = match url.split_once(':') {
        Some((user_host, repo)) => {
            let (user, host) = user_host.split_once('@').unwrap_or(("", user_host));
            !host.is_empty()
                && host.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
                && user.chars().all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
                && !user_host.contains('/')
                && !repo.is_empty()
                && !repo.starts_with("//")
                && !user_host.contains("::")
        }
        None => false,
    };
    if with_scheme || (scp_like && url.contains('@')) { Ok(()) } else { invalid() }
}

/// What `git clone` names the folder: the last path segment without `.git`.
pub fn repo_name_from_url(url: &str) -> Option<String> {
    let trimmed = url.trim_end_matches('/');
    let last = trimmed.rsplit(['/', ':']).next()?;
    let name = last.strip_suffix(".git").unwrap_or(last);
    (!name.is_empty() && validate_entry_name(name).is_ok() && name != "." && name != "..").then(|| name.to_string())
}

/// Branch names from the UI (`dev`, `origin/HISTORIA/FLEET-2098`), following
/// `git check-ref-format` closely enough that nothing reaches git as an option or a revision
/// expression.
pub fn validate_branch_name(name: &str) -> Result<(), String> {
    let invalid = name.is_empty()
        || name.len() > 255
        || name.starts_with('-')
        || name.starts_with('/')
        || name.ends_with('/')
        || name.ends_with('.')
        || name.ends_with(".lock")
        || name.contains("..")
        || name.contains("//")
        || name.contains("@{")
        || name == "@"
        || name.chars().any(|c| c.is_control() || c.is_whitespace() || "~^:?*[\\".contains(c))
        || name.split('/').any(|part| part.starts_with('.'));
    if invalid { Err(format!("Nome de branch inválido: {name:?}")) } else { Ok(()) }
}

// ---------------------------------------------------------------------------
// Scripts
// ---------------------------------------------------------------------------

const ENTER_REPO: &str = r#"cd -- "$1" 2>/dev/null || { echo "Pasta não encontrada: $1"; exit 3; }
command -v git >/dev/null 2>&1 || { echo "@@error no-git"; exit 4; }
git rev-parse --git-dir >/dev/null 2>&1 || { echo "@@error not-repo"; exit 5; }
"#;

/// No prompts (there is no terminal), English messages (parsed below), ssh in batch mode
/// unless the user configured their own ssh command.
const NO_PROMPTS: &str = r#"export GIT_TERMINAL_PROMPT=0 GCM_INTERACTIVE=never LC_MESSAGES=C
[ -n "$GIT_SSH_COMMAND" ] || [ -n "$(git config core.sshCommand)" ] || export GIT_SSH_COMMAND="ssh -o BatchMode=yes -o ConnectTimeout=20"
"#;

/// Inside a repository: `auth` from `$2`, `url` = the current branch's remote (for [`NET`]).
const REMOTE_URL: &str = r#"auth=$2
br=$(git symbolic-ref -q --short HEAD)
rm_name=$(git config "branch.$br.remote" 2>/dev/null) || rm_name=origin
url=$(git remote get-url "$rm_name" 2>/dev/null)
"#;

/// `net <git args>`: runs a network command honoring `$auth` (see `settings::GitAuth`) for
/// the remote `$url`. For an HTTPS URL, `ssh_cfg` rewrites `https://host/` to `git@host:`
/// through `-c url.<ssh>.insteadOf`, for that one command: the repository's config is never
/// touched (a clone keeps the HTTPS URL as its origin). `@@via ssh` tells the caller SSH was
/// used. zsh-safe: no `path`/`status` variables (special in zsh), no word splitting.
const NET: &str = r#"ssh_cfg=
case $url in https://*)
  rest=${url#https://}; hp=${rest%%/*}; h=${hp##*@}; h=${h%%:*}
  ssh_cfg="url.git@$h:.insteadOf=https://$hp/" ;;
esac
net() {
  if [ "$auth" = ssh ] && [ -n "$ssh_cfg" ]; then
    echo "@@via ssh"; git -c "$ssh_cfg" "$@"; return $?
  fi
  if [ "$auth" = auto ] && [ -n "$ssh_cfg" ]; then
    out=$(git "$@" 2>&1); st=$?
    if [ $st -ne 0 ] && printf '%s' "$out" | grep -qiE 'could not read (username|password)|terminal prompts disabled|authentication failed|invalid username or password'; then
      echo "@@via ssh"; git -c "$ssh_cfg" "$@"; return $?
    fi
    [ -n "$out" ] && printf '%s\n' "$out"
    return $st
  fi
  git "$@"
}
"#;

const BRANCHES: &str = r#"echo @@head
git symbolic-ref -q --short HEAD || { printf 'detached '; git rev-parse --short HEAD 2>/dev/null; }
echo @@local
git for-each-ref --format='%(refname:short)%09%(upstream:short)%09%(upstream:track,nobracket)' refs/heads
echo @@remote
git for-each-ref --format='%(refname:short)%09%(symref)' refs/remotes
echo @@status
git status --porcelain=v1 2>/dev/null
exit 0"#;

/// `$2` = branch, `$3` = `remote` to create a local branch tracking `$2` (`origin/dev`).
const SWITCH: &str = r#"if [ "$3" = remote ]; then git switch --track "$2"; else git switch "$2"; fi"#;

const FETCH: &str = "net fetch --all --prune --quiet";

/// `$1` = parent folder, `$2` = URL, `$3` = new folder name, `$4` = auth mode. A failed clone
/// removes its folder, so the SSH retry starts clean.
const CLONE: &str = r#"cd -- "$1" 2>/dev/null || { echo "Pasta não encontrada: $1"; exit 3; }
command -v git >/dev/null 2>&1 || { echo "@@error no-git"; exit 4; }
[ -e "$3" ] && { echo "@@error exists"; exit 6; }
url=$2; auth=$4
"#;

const PULL: &str = r#"b=$(git symbolic-ref -q --short HEAD) || { echo "@@error detached"; exit 10; }
git rev-parse -q --verify '@{u}' >/dev/null 2>&1 || { echo "@@error no-upstream"; exit 11; }
before=$(git rev-parse HEAD)
net pull --ff-only --no-rebase --quiet || exit $?
after=$(git rev-parse HEAD)
echo "@@branch $b"
if [ "$before" != "$after" ]; then
  echo "@@commits $(git rev-list --count "$before..$after")"
  echo "@@stat $(git diff --shortstat "$before" "$after")"
fi"#;

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

fn parse_track(track: &str) -> (u32, u32, bool) {
    let mut ahead = 0;
    let mut behind = 0;
    for part in track.split(',').map(str::trim) {
        if let Some(n) = part.strip_prefix("ahead ") {
            ahead = n.parse().unwrap_or(0);
        } else if let Some(n) = part.strip_prefix("behind ") {
            behind = n.parse().unwrap_or(0);
        }
    }
    (ahead, behind, track.trim() == "gone")
}

pub fn parse_branches(text: &str) -> GitBranches {
    let mut section = "";
    let mut out = GitBranches { current: None, detached: None, local: Vec::new(), remote: Vec::new(), changes: 0, untracked: 0 };
    let mut remotes: Vec<String> = Vec::new();

    for line in text.lines() {
        if let Some(name) = line.strip_prefix("@@") {
            section = name.trim();
            continue;
        }
        match section {
            "head" if !line.trim().is_empty() => match line.trim().strip_prefix("detached ") {
                Some(commit) => out.detached = Some(commit.to_string()),
                None => out.current = Some(line.trim().to_string()),
            },
            "local" => {
                let mut f = line.split('\t');
                let Some(name) = f.next().filter(|n| !n.is_empty()) else { continue };
                let upstream = f.next().filter(|u| !u.is_empty()).map(str::to_string);
                let (ahead, behind, gone) = parse_track(f.next().unwrap_or(""));
                out.local.push(LocalBranch { name: name.to_string(), upstream, ahead, behind, gone });
            }
            "remote" => {
                let mut f = line.split('\t');
                let name = f.next().unwrap_or("");
                let symref = f.next().unwrap_or("");
                // `origin/HEAD` is a pointer, not a branch.
                if name.contains('/') && symref.is_empty() {
                    remotes.push(name.to_string());
                }
            }
            "status" if line.len() > 2 => {
                if line.starts_with("??") {
                    out.untracked += 1;
                } else {
                    out.changes += 1;
                }
            }
            _ => {}
        }
    }

    out.remote = remotes
        .into_iter()
        .filter(|r| {
            let short = r.split_once('/').map(|(_, b)| b).unwrap_or(r);
            !out.local.iter().any(|l| l.name == short)
        })
        .collect();
    out
}

pub fn parse_pull(text: &str) -> Result<PullResult, String> {
    let mut result = PullResult { branch: String::new(), commits: 0, stat: String::new(), via_ssh: false };
    for line in text.lines() {
        if line.trim() == "@@via ssh" {
            result.via_ssh = true;
        } else if let Some(b) = line.strip_prefix("@@branch ") {
            result.branch = b.trim().to_string();
        } else if let Some(n) = line.strip_prefix("@@commits ") {
            result.commits = n.trim().parse().unwrap_or(0);
        } else if let Some(s) = line.strip_prefix("@@stat ") {
            result.stat = s.trim().to_string();
        }
    }
    if result.branch.is_empty() {
        return Err("O git não informou o resultado do pull".into());
    }
    Ok(result)
}

/// Turns git's output into a short Portuguese explanation (first line) followed by the raw
/// output, which the UI shows as detail.
pub fn explain_error(raw: &str) -> String {
    let raw = raw.trim();
    let has = |needle: &str| raw.contains(needle);
    let reason = if has("@@error no-git") {
        Some("O git não está instalado nesta distro")
    } else if has("@@error not-repo") {
        Some("A pasta não é um repositório git")
    } else if has("@@error exists") || has("already exists and is not an empty directory") {
        Some("Já existe uma pasta com esse nome aqui: escolha outro nome")
    } else if has("Repository not found") || (has("repository '") && has("not found")) {
        Some("Repositório não encontrado (link errado ou sem acesso)")
    } else if has("@@error detached") {
        Some("HEAD destacado (detached): troque para uma branch antes de atualizar")
    } else if has("@@error no-upstream") {
        Some("A branch atual não tem uma branch remota (upstream) configurada")
    } else if has("would be overwritten") {
        Some("Alterações locais seriam sobrescritas: faça commit ou stash antes")
    } else if has("Not possible to fast-forward") || has("diverging branches") || has("have diverged") {
        Some("A branch local divergiu do remoto: resolva com merge ou rebase no terminal")
    } else if has("unmerged files") || has("not concluded your merge") || has("resolve your current index") {
        Some("Há um merge em andamento ou conflitos não resolvidos")
    } else if has("Permission denied (publickey)") || has("Host key verification failed") {
        Some("Sem acesso ao remoto: confira a chave SSH (ssh -T git@github.com no terminal)")
    } else if has("could not read Username") || has("terminal prompts disabled") || has("Authentication failed") {
        Some("O remoto pediu usuário e senha (HTTPS sem credencial salva): ative \"tentar via SSH\" em Configurações → Git ou use um credential helper")
    } else if has("Could not resolve host") || has("timed out") || has("Network is unreachable") || has("Connection refused") {
        Some("Sem conexão com o servidor do repositório")
    } else if has("Could not read from remote repository") {
        Some("Não foi possível ler o repositório remoto (acesso ou endereço)")
    } else if has("already exists") {
        Some("Já existe uma branch local com esse nome")
    } else if has("Tempo esgotado") {
        Some("O git demorou demais e foi interrompido")
    } else {
        None
    };
    // The HTTPS remote was retried (or forced) over SSH: say so, the SSH error is what's left.
    let reason = match reason {
        Some(r) if has("@@via ssh") => Some(format!("{r} (tentado via SSH)")),
        r => r.map(str::to_string),
    };
    let detail = raw.lines().filter(|l| !l.starts_with("@@")).collect::<Vec<_>>().join("\n");
    let detail = detail.trim();
    match (reason, detail.lines().last()) {
        (Some(r), None) => r,
        (Some(r), Some(_)) => format!("{r}\n{detail}"),
        (None, None) => "Falha no git".into(),
        // Unknown error: git's last line (usually the `fatal:`) as the summary.
        (None, Some(last)) if detail.lines().count() == 1 => last.to_string(),
        (None, Some(last)) => format!("{last}\n{detail}"),
    }
}

// ---------------------------------------------------------------------------
// Operations
// ---------------------------------------------------------------------------

async fn prepare(distro: &str, path: &str) -> Result<String, String> {
    let path = normalize_linux_path(path)?;
    ensure_running(distro).await?;
    Ok(path)
}

pub(crate) async fn branches(distro: &str, path: &str) -> Result<GitBranches, String> {
    let path = prepare(distro, path).await?;
    let script = format!("{ENTER_REPO}{BRANCHES}");
    let text = run(sh_in_distro(distro, None, &script, &[&path]), READ_TIMEOUT)
        .await
        .map_err(|e| explain_error(&e))?;
    Ok(parse_branches(&text))
}

pub(crate) async fn switch(distro: &str, path: &str, branch: &str, remote: bool) -> Result<(), String> {
    validate_branch_name(branch)?;
    let path = prepare(distro, path).await?;
    let script = format!("{ENTER_REPO}{NO_PROMPTS}{SWITCH}");
    let kind = if remote { "remote" } else { "local" };
    run_user_script(distro, &script, &[&path, branch, kind], SWITCH_TIMEOUT)
        .await
        .map(|_| ())
        .map_err(|e| explain_error(&e))
}

pub(crate) async fn fetch(distro: &str, path: &str, auth: GitAuth) -> Result<(), String> {
    let path = prepare(distro, path).await?;
    let script = format!("{ENTER_REPO}{NO_PROMPTS}{REMOTE_URL}{NET}{FETCH}");
    run_user_script(distro, &script, &[&path, auth.as_str()], NETWORK_TIMEOUT)
        .await
        .map(|_| ())
        .map_err(|e| explain_error(&e))
}

pub(crate) async fn pull(distro: &str, path: &str, auth: GitAuth) -> Result<PullResult, String> {
    let path = prepare(distro, path).await?;
    let script = format!("{ENTER_REPO}{NO_PROMPTS}{REMOTE_URL}{NET}{PULL}");
    let text = run_user_script(distro, &script, &[&path, auth.as_str()], NETWORK_TIMEOUT)
        .await
        .map_err(|e| explain_error(&e))?;
    parse_pull(&text)
}

/// Clones `url` into `<parent>/<name>` (name defaults to the repository's, like `git clone`).
/// Returns the new folder's path.
pub(crate) async fn clone(distro: &str, parent: &str, url: &str, name: Option<&str>, auth: GitAuth) -> Result<CloneResult, String> {
    let url = url.trim();
    validate_clone_url(url)?;
    let name = match name.map(str::trim).filter(|n| !n.is_empty()) {
        Some(n) => n.to_string(),
        None => repo_name_from_url(url).ok_or("Não deu para tirar o nome da pasta do link: informe um nome")?,
    };
    validate_entry_name(&name)?;
    let parent = prepare(distro, parent).await?;
    let script = format!("{CLONE}{NO_PROMPTS}{NET}net clone --quiet -- \"$url\" \"$3\"");
    let text = run_user_script(distro, &script, &[&parent, url, &name, auth.as_str()], CLONE_TIMEOUT)
        .await
        .map_err(|e| explain_error(&e))?;
    let path = if parent == "/" { format!("/{name}") } else { format!("{parent}/{name}") };
    Ok(CloneResult { path, name, via_ssh: text.lines().any(|l| l.trim() == "@@via ssh") })
}

// ---------------------------------------------------------------------------
// Tauri commands
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn git_branches(distro: String, path: String) -> Result<GitBranches, String> {
    branches(&distro, &path).await
}

#[tauri::command]
pub async fn git_switch(distro: String, path: String, branch: String, remote: bool) -> Result<(), String> {
    switch(&distro, &path, &branch, remote).await
}

#[tauri::command]
pub async fn git_fetch(settings: State<'_, SettingsStore>, distro: String, path: String) -> Result<(), String> {
    fetch(&distro, &path, settings.git_auth()).await
}

#[tauri::command]
pub async fn git_clone(
    settings: State<'_, SettingsStore>,
    distro: String,
    parent: String,
    url: String,
    name: Option<String>,
) -> Result<CloneResult, String> {
    clone(&distro, &parent, &url, name.as_deref(), settings.git_auth()).await
}

#[tauri::command]
pub async fn git_pull(settings: State<'_, SettingsStore>, distro: String, path: String) -> Result<PullResult, String> {
    pull(&distro, &path, settings.git_auth()).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_branch_names() {
        for ok in ["dev", "main", "HISTORIA/FLEET-2098", "origin/feature/x_y-1", "release-1.2"] {
            assert!(validate_branch_name(ok).is_ok(), "{ok}");
        }
        for bad in ["", "-f", "--orphan", "a..b", "a b", "x~1", "x^", "a:b", "@", "x@{u}", "/x", "x/", "x.lock", "a/.hidden", "a\nb", "a\\b"] {
            assert!(validate_branch_name(bad).is_err(), "{bad:?}");
        }
    }

    /// Security audit (0.1.3): multi-byte characters right at every scheme's cut position must be
    /// rejected, never panic.
    #[test]
    fn clone_url_validation_never_panics_on_utf8() {
        let schemes = ["http://", "https://", "ssh://", "git://", "file://", "ext::"];
        let tricky = ["é", "ã", "ç", "😀", "€", "\u{301}"];
        for scheme in schemes {
            for ch in tricky {
                for cut in 0..=scheme.len() {
                    // The multi-byte char starts at every byte position up to the scheme's length.
                    let url = format!("{}{ch}{}", &scheme[..cut], "host/repo");
                    let _ = validate_clone_url(&url);
                    let _ = repo_name_from_url(&url);
                    // ...and with ASCII padding so the length check passes first.
                    let padded = format!("{ch}{scheme}github.com/a/b");
                    assert!(validate_clone_url(&padded).is_err(), "{padded}");
                }
            }
        }
        // 7 ASCII letters followed by `é`: the original crash (byte 7 is inside `é`).
        assert!(validate_clone_url("abcdefgé/repo").is_err());
        assert!(validate_clone_url("httpséx://github.com/a/b").is_err());
        assert!(validate_clone_url("https://github.com/organização/repositório.git").is_ok());
        assert_eq!(repo_name_from_url("https://github.com/org/repositório.git").as_deref(), Some("repositório"));
    }

    #[test]
    fn validates_clone_urls_and_names() {
        for ok in [
            "https://github.com/agoravai-arca/aaa.git",
            "HTTPS://gitlab.com/group/sub/repo",
            "git@github.com:consultoria-fleeting/api-financial.git",
            "ssh://git@host.example:2222/org/repo.git",
            "git://example.org/repo",
        ] {
            assert!(validate_clone_url(ok).is_ok(), "{ok}");
        }
        for bad in [
            "",
            "--upload-pack=touch /tmp/x",
            "ext::sh -c touch% /tmp/x",
            "file:///etc",
            "/home/kyo/repo",
            "../repo",
            "C:\\repo",
            "https://github.com/a b",
            "github.com/org/repo",
            "host:repo",
        ] {
            assert!(validate_clone_url(bad).is_err(), "{bad:?}");
        }
        assert_eq!(repo_name_from_url("https://github.com/agoravai-arca/aaa.git").as_deref(), Some("aaa"));
        assert_eq!(repo_name_from_url("git@github.com:org/api-financial.git").as_deref(), Some("api-financial"));
        assert_eq!(repo_name_from_url("https://gitlab.com/group/repo/").as_deref(), Some("repo"));
        assert_eq!(repo_name_from_url("git@host:repo.git").as_deref(), Some("repo"));
        assert_eq!(repo_name_from_url("https://host/.git"), None);
        assert!(explain_error("@@error exists").starts_with("Já existe uma pasta"));
        assert!(explain_error("ERROR: Repository not found.\nfatal: Could not read from remote repository.").starts_with("Repositório não encontrado"));
    }

    const BRANCHES_OUT: &str = "@@head
dev
@@local
dev\torigin/dev\tahead 2, behind 3
HISTORIA/FLEET-2037\torigin/HISTORIA/FLEET-2037\tgone
local-only\t\t
@@remote
origin\trefs/remotes/origin/dev
origin/dev\t
origin/prod\t
origin/HISTORIA/FLEET-2037\t
@@status
 M src/main.ts
A  new.ts
?? notes.md
";

    #[test]
    fn parses_branches() {
        let b = parse_branches(BRANCHES_OUT);
        assert_eq!(b.current.as_deref(), Some("dev"));
        assert_eq!(b.detached, None);
        assert_eq!(b.local.len(), 3);
        assert_eq!(b.local[0], LocalBranch { name: "dev".into(), upstream: Some("origin/dev".into()), ahead: 2, behind: 3, gone: false });
        assert!(b.local[1].gone);
        assert_eq!(b.local[2].upstream, None);
        assert_eq!(b.remote, ["origin/prod"], "origin/HEAD and branches already local are hidden");
        assert_eq!((b.changes, b.untracked), (2, 1));
    }

    #[test]
    fn parses_detached_head() {
        let b = parse_branches("@@head\ndetached 3f9c2a1\n@@local\n@@remote\n@@status\n");
        assert_eq!(b.current, None);
        assert_eq!(b.detached.as_deref(), Some("3f9c2a1"));
    }

    #[test]
    fn parses_pull() {
        let up = parse_pull("hook noise\n@@branch dev\n").unwrap();
        assert_eq!((up.branch.as_str(), up.commits), ("dev", 0));
        let new = parse_pull("@@branch dev\n@@commits 4\n@@stat 3 files changed, 10 insertions(+)\n").unwrap();
        assert_eq!(new.commits, 4);
        assert_eq!(new.stat, "3 files changed, 10 insertions(+)");
        assert!(parse_pull("garbage").is_err());
    }

    #[test]
    fn explains_errors() {
        let e = explain_error("fatal: Not possible to fast-forward, aborting.");
        assert!(e.starts_with("A branch local divergiu"), "{e}");
        assert!(e.ends_with("aborting."), "raw output kept as detail");
        assert!(explain_error("git@github.com: Permission denied (publickey).\nfatal: Could not read from remote repository.")
            .starts_with("Sem acesso ao remoto"));
        assert_eq!(explain_error("@@error no-upstream"), "A branch atual não tem uma branch remota (upstream) configurada");
        assert_eq!(explain_error("fatal: something new\n"), "fatal: something new");
        let ssh = explain_error("@@via ssh\ngit@github.com: Permission denied (publickey).");
        assert!(ssh.starts_with("Sem acesso ao remoto") && ssh.contains("(tentado via SSH)\n"), "{ssh}");
        assert!(!ssh.contains("@@"), "markers never reach the UI");
        assert!(parse_pull("@@via ssh\n@@branch main\n").unwrap().via_ssh);
    }

    /// Real git in a real distro: a bare "remote" in /tmp, two clones, push from one and pull
    /// in the other through the user's login shell. `WSL_TEST_DISTRO=Ubuntu cargo test -- --ignored live_git`
    #[tokio::test]
    #[ignore]
    async fn live_git_branches_switch_pull() {
        let distro = std::env::var("WSL_TEST_DISTRO").expect("set WSL_TEST_DISTRO");
        let root = format!("/tmp/wsl-symbiosis-git-{}", std::process::id());
        let setup = r#"set -e
r=$1; rm -rf "$r"; mkdir -p "$r"; cd "$r"
c() { git -c user.name=t -c user.email=t@t -c commit.gpgsign=false "$@"; }
git init -q --bare -b main remote.git
git clone -q remote.git a 2>/dev/null; cd a
echo 1 > f; git add f; c commit -qm one; git push -q origin main
git switch -qc dev; echo 2 > g; git add g; c commit -qm two; git push -q origin dev
git switch -q main
cd ..; git clone -q remote.git 'b proj'
cd a; echo 3 >> f; c commit -qam three; echo 4 >> f; c commit -qam four; git push -q origin main"#;
        run(sh_in_distro(&distro, None, setup, &[&root]), Duration::from_secs(60)).await.expect("setup");
        let b = format!("{root}/b proj");

        // Listing reads the branch from .git/HEAD.
        let listing = crate::explorer::list(&distro, &root, false).await.expect("list");
        let entry = listing.entries.iter().find(|e| e.name == "b proj").expect("b proj listed");
        assert_eq!(entry.git_branch.as_deref(), Some("main"));

        let before = branches(&distro, &b).await.expect("branches");
        println!("{before:#?}");
        assert_eq!(before.current.as_deref(), Some("main"));
        assert_eq!(before.remote, ["origin/dev"]);

        let pulled = pull(&distro, &b, GitAuth::Auto).await.expect("pull");
        println!("{pulled:?}");
        assert_eq!((pulled.branch.as_str(), pulled.commits), ("main", 2));
        assert!(pulled.stat.contains("1 file changed"), "{}", pulled.stat);
        assert!(!pulled.via_ssh, "a local remote is not HTTPS");
        assert_eq!(pull(&distro, &b, GitAuth::Ssh).await.expect("pull again").commits, 0, "already up to date");

        switch(&distro, &b, "origin/dev", true).await.expect("switch to remote branch");
        let after = branches(&distro, &b).await.expect("branches after");
        assert_eq!(after.current.as_deref(), Some("dev"));
        assert_eq!(after.local.iter().find(|l| l.name == "dev").unwrap().upstream.as_deref(), Some("origin/dev"));
        switch(&distro, &b, "main", false).await.expect("switch back");
        assert!(switch(&distro, &b, "nope", false).await.is_err());

        // A local-only branch has no upstream: a clear message, not a prompt or a hang.
        run(sh_in_distro(&distro, None, r#"cd "$1" && git switch -qc solo"#, &[&b]), Duration::from_secs(20)).await.unwrap();
        let err = pull(&distro, &b, GitAuth::Auto).await.unwrap_err();
        assert!(err.starts_with("A branch atual não tem"), "{err}");

        run(sh_in_distro(&distro, None, r#"rm -rf "$1""#, &[&root]), Duration::from_secs(20)).await.unwrap();
    }

    /// A private GitHub repository cloned over HTTPS: without a credential helper git asks
    /// for a username (prompt disabled → error), and `auto` retries over SSH with the user's
    /// key. Needs network + a key GitHub accepts.
    /// `WSL_TEST_DISTRO=Ubuntu WSL_TEST_HTTPS_REPO=https://github.com/org/private.git cargo test --lib live_git_https -- --ignored`
    #[tokio::test]
    #[ignore]
    async fn live_git_https_falls_back_to_ssh() {
        let distro = std::env::var("WSL_TEST_DISTRO").expect("set WSL_TEST_DISTRO");
        let https = std::env::var("WSL_TEST_HTTPS_REPO").expect("set WSL_TEST_HTTPS_REPO");
        let root = format!("/tmp/wsl-symbiosis-https-{}", std::process::id());
        // Clone over SSH (works), then point origin at the HTTPS URL, like the user's repo.
        let setup = r#"set -e
rm -rf "$1"; mkdir -p "$1"; cd "$1"
ssh_url=$(printf '%s' "$2" | sed -E 's#^https://([^/]+)/#git@\1:#')
GIT_SSH_COMMAND="ssh -o BatchMode=yes" git clone -q --depth 5 "$ssh_url" repo
cd repo; git remote set-url origin "$2""#;
        run(sh_in_distro(&distro, None, setup, &[&root, &https]), Duration::from_secs(120)).await.expect("setup");
        let repo = format!("{root}/repo");

        let err = pull(&distro, &repo, GitAuth::Remote).await.unwrap_err();
        println!("remote: {err}");
        assert!(err.starts_with("O remoto pediu usuário e senha"), "{err}");

        let auto = pull(&distro, &repo, GitAuth::Auto).await.expect("auto falls back to ssh");
        println!("auto: {auto:?}");
        assert!(auto.via_ssh);
        let forced = pull(&distro, &repo, GitAuth::Ssh).await.expect("ssh");
        assert!(forced.via_ssh);
        fetch(&distro, &repo, GitAuth::Auto).await.expect("fetch falls back too");

        let url = run(sh_in_distro(&distro, None, r#"git -C "$1" remote get-url origin"#, &[&repo]), Duration::from_secs(20))
            .await
            .unwrap();
        assert_eq!(url.trim(), https, "the repository's remote is never rewritten");

        // Clone: HTTPS asks for a password → SSH; the clone keeps the HTTPS origin.
        let cloned = clone(&distro, &root, &https, None, GitAuth::Auto).await.expect("clone via fallback");
        println!("clone: {cloned:?}");
        assert!(cloned.via_ssh);
        assert_eq!(cloned.path, format!("{root}/{}", repo_name_from_url(&https).unwrap()));
        let origin = run(sh_in_distro(&distro, None, r#"git -C "$1" remote get-url origin"#, &[&cloned.path]), Duration::from_secs(20))
            .await
            .unwrap();
        assert_eq!(origin.trim(), https);
        let again = clone(&distro, &root, &https, None, GitAuth::Auto).await.unwrap_err();
        assert!(again.starts_with("Já existe uma pasta"), "{again}");
        let ssh_url = https.replacen("https://github.com/", "git@github.com:", 1);
        let named = clone(&distro, &root, &ssh_url, Some("meu nome"), GitAuth::Remote).await.expect("clone over ssh with a name");
        assert!(named.path.ends_with("/meu nome") && !named.via_ssh);
        let err = clone(&distro, &root, &https, Some("x"), GitAuth::Remote).await.unwrap_err();
        assert!(err.starts_with("O remoto pediu usuário e senha"), "{err}");
        let listing = crate::explorer::list(&distro, &root, false).await.unwrap();
        assert!(!listing.entries.iter().any(|e| e.name == "x"), "a failed clone leaves nothing behind");

        run(sh_in_distro(&distro, None, r#"rm -rf "$1""#, &[&root]), Duration::from_secs(20)).await.unwrap();
    }
}
