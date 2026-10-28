#!/usr/bin/env node
// Builds a signed release for the auto-updater and prepares everything for GitHub Releases:
//   1. loads the signing key from .tauri-keys/ (never committed, see .gitignore);
//   2. runs `npm run tauri build`;
//   3. writes dist-release/ with the installer and the latest.json the app reads;
//   4. prints what to upload and where.
// Usage: npm run release:build
import { spawnSync } from "node:child_process";
import { copyFileSync, existsSync, mkdirSync, readFileSync, rmSync, statSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const KEY_DIR = join(ROOT, ".tauri-keys");
const KEY_FILE = join(KEY_DIR, "symbiosis.key");
const PASSWORD_FILE = join(KEY_DIR, "password.txt");
const CONF_FILE = join(ROOT, "src-tauri", "tauri.conf.json");
const NSIS_DIR = join(ROOT, "src-tauri", "target", "release", "bundle", "nsis");
const OUT_DIR = join(ROOT, "dist-release");
const CHANGELOG = join(ROOT, "docs", "CHANGELOG.md");

function fail(message) {
  console.error(`\n✖ ${message}\n`);
  process.exit(1);
}

// ── 1. Signing key ──────────────────────────────────────────────────────────
if (!existsSync(KEY_FILE)) {
  fail(
    `Chave de assinatura não encontrada em ${KEY_FILE}.\n` +
      "  Sem ela o updater recusa o pacote. Restaure o backup da chave (e da senha) em .tauri-keys/.\n" +
      "  Gerar uma chave nova invalida as instalações existentes: elas só aceitam a chave pública antiga.",
  );
}
const env = { ...process.env, TAURI_SIGNING_PRIVATE_KEY: readFileSync(KEY_FILE, "utf8").trim() };
if (env.TAURI_SIGNING_PRIVATE_KEY_PASSWORD === undefined) {
  env.TAURI_SIGNING_PRIVATE_KEY_PASSWORD = existsSync(PASSWORD_FILE) ? readFileSync(PASSWORD_FILE, "utf8").trim() : "";
}

const conf = JSON.parse(readFileSync(CONF_FILE, "utf8"));
const version = conf.version;
const endpoint = conf.plugins?.updater?.endpoints?.[0] ?? "";
const repo = /github\.com\/([^/]+\/[^/]+)\/releases/.exec(endpoint)?.[1];
if (!version) fail("tauri.conf.json sem \"version\".");
if (!repo) fail(`Endpoint do updater não aponta para um repositório do GitHub: ${endpoint || "(vazio)"}`);
if (!conf.bundle?.createUpdaterArtifacts) fail("bundle.createUpdaterArtifacts precisa estar ativo no tauri.conf.json.");

// ── 2. Build ────────────────────────────────────────────────────────────────
console.log(`\n▶ Gerando WSL Symbiosis v${version} assinado (npm run tauri build)…\n`);
const startedAt = Date.now();
const build = spawnSync("npm", ["run", "tauri", "build"], { cwd: ROOT, env, stdio: "inherit", shell: true });
if (build.status !== 0) fail(`tauri build falhou (código ${build.status}).`);

// ── 3. dist-release/ ────────────────────────────────────────────────────────
const installerName = `${conf.productName}_${version}_x64-setup.exe`;
const installer = join(NSIS_DIR, installerName);
const signature = `${installer}.sig`;
if (!existsSync(installer)) fail(`Instalador não encontrado: ${installer}`);
if (!existsSync(signature)) fail(`Assinatura não encontrada: ${signature} (a chave foi carregada?)`);
if (statSync(signature).mtimeMs < startedAt - 1000) fail(`${signature} é de um build anterior.`);

// GitHub turns spaces in asset names into dots, so the published name must not have any:
// the URL in latest.json has to match the asset exactly.
const assetName = installerName.replace(/\s+/g, "-");
const tag = `v${version}`;
const url = `https://github.com/${repo}/releases/download/${tag}/${encodeURIComponent(assetName)}`;
const notes = releaseNotes(version);
const platform = { signature: readFileSync(signature, "utf8").trim(), url };
const latest = {
  version,
  notes,
  pub_date: new Date().toISOString().replace(/\.\d{3}Z$/, "Z"),
  // The updater tries "<os>-<arch>-<installer>" first, then "<os>-<arch>".
  platforms: { "windows-x86_64-nsis": platform, "windows-x86_64": platform },
};

rmSync(OUT_DIR, { recursive: true, force: true });
mkdirSync(OUT_DIR, { recursive: true });
copyFileSync(installer, join(OUT_DIR, assetName));
writeFileSync(join(OUT_DIR, "latest.json"), `${JSON.stringify(latest, null, 2)}\n`);
writeFileSync(join(OUT_DIR, "release-notes.md"), `${notes}\n`);

// ── 4. Next steps ───────────────────────────────────────────────────────────
const size = (statSync(installer).size / 1024 / 1024).toFixed(2);
console.log(`
✔ Release v${version} pronta em dist-release/

  ${assetName}  (${size} MB, assinado)
  latest.json                             (o app lê este arquivo)
  release-notes.md                        (texto para a descrição da release)

Publicar no GitHub:
  1. Abra https://github.com/${repo}/releases/new
  2. Tag: ${tag}  (crie a tag nesse campo, a partir da branch principal)
  3. Título: WSL Symbiosis ${tag}
  4. Descrição: cole o conteúdo de dist-release/release-notes.md
  5. Anexe os DOIS arquivos de dist-release/:
       • ${assetName}
       • latest.json
  6. Deixe "Set as the latest release" marcado e NÃO marque "pre-release"
     (o updater lê .../releases/latest/download/latest.json, que ignora pre-releases e rascunhos).
  7. Publish release.

Os apps instalados encontram a versão nova na próxima abertura (ou em Configurações → Verificar atualizações).
`);

/** Section "## <version> ..." of docs/CHANGELOG.md, without the heading. */
function releaseNotes(v) {
  if (!existsSync(CHANGELOG)) return `WSL Symbiosis v${v}`;
  const text = readFileSync(CHANGELOG, "utf8").replace(/\r\n/g, "\n");
  const escaped = v.replace(/\./g, "\\.");
  const match = new RegExp(`^## ${escaped}\\b[^\\n]*\\n([\\s\\S]*?)(?=^## |(?![\\s\\S]))`, "m").exec(text);
  const body = match?.[1].trim();
  return body || `WSL Symbiosis v${v}`;
}
