import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { siMariadb, siMongodb, siMysql, siPostgresql, siRedis } from "simple-icons";
import type { SimpleIcon } from "./brands";

// Mirrors of src-tauri/src/docker.rs (engine, containers, 1-click databases) and
// src-tauri/src/database.rs (table explorer).

export type DockerSource = "wsl_native" | "docker_desktop";
export type EngineState = "ready" | "stopped" | "not_installed";

export interface DockerStatus {
  state: EngineState;
  source: DockerSource | null;
  version: string | null;
  desktopInstalled: boolean;
  nativeInstalled: boolean;
  /** False when the distro was stopped: its own Docker wasn't looked for. */
  distroChecked: boolean;
  starting: boolean;
}

export type DbKind = "postgres" | "mysql" | "mariadb" | "redis" | "mongo";

export interface PortMapping {
  hostIp: string;
  hostPort: number;
  containerPort: number;
  protocol: string;
}

export interface ContainerStats {
  cpuPercent: number;
  memoryUsed: number;
  memoryLimit: number;
}

export interface DatabaseInfo {
  kind: DbKind;
  user: string | null;
  password: string | null;
  database: string | null;
  hostPort: number | null;
  connectionUrl: string | null;
  envSnippet: string | null;
  explorer: boolean;
}

export interface ContainerInfo {
  id: string;
  shortId: string;
  name: string;
  image: string;
  /** running · exited · created · paused · restarting · dead */
  state: string;
  status: string;
  ports: PortMapping[];
  createdAt: string;
  managed: boolean;
  composeProject: string | null;
  volumes: string[];
  stats: ContainerStats | null;
  database: DatabaseInfo | null;
}

export interface LogLine {
  time: string;
  text: string;
  error: boolean;
}

export interface RemoveResult {
  removedVolumes: string[];
  keptVolumes: string[];
}

export interface DatabaseDefaults {
  name: string;
  port: number;
  image: string;
  database: string | null;
  user: string | null;
  password: string | null;
}

export interface NewDatabase {
  kind: DbKind;
  name: string;
  database: string | null;
  user: string | null;
  password: string | null;
  port: number;
}

export interface CreatedDatabase {
  id: string;
  name: string;
  ready: boolean;
  database: DatabaseInfo | null;
}

export interface DatabaseProgress {
  name: string;
  stage: "pull" | "create" | "wait";
  detail: string | null;
  /** Pull only: share of the image layers downloaded and extracted (0–100). */
  percent: number | null;
}

export const getDockerStatus = (distro: string) => invoke<DockerStatus>("get_docker_status", { distro });
export const startDockerEngine = (distro: string) => invoke<DockerStatus>("start_docker_engine", { distro });
/** Docker Desktop: `docker desktop stop` (clean quit). Docker of the distro: systemctl/service stop. */
export const stopDockerEngine = (distro: string) => invoke<DockerStatus>("stop_docker_engine", { distro });
export const listContainers = (distro: string) => invoke<ContainerInfo[]>("list_containers", { distro });
export const startContainer = (distro: string, id: string) => invoke<void>("start_container", { distro, id });
export const stopContainer = (distro: string, id: string) => invoke<void>("stop_container", { distro, id });
export const restartContainer = (distro: string, id: string) => invoke<void>("restart_container", { distro, id });
export const removeContainer = (distro: string, id: string, removeVolume: boolean) =>
  invoke<RemoveResult>("remove_container", { distro, id, removeVolume });
export const getContainerLogs = (distro: string, id: string, tail: number) => invoke<LogLine[]>("get_container_logs", { distro, id, tail });
export const getDatabaseDefaults = (distro: string, kind: DbKind) => invoke<DatabaseDefaults>("get_database_defaults", { distro, kind });
export const createDatabaseContainer = (distro: string, request: NewDatabase) =>
  invoke<CreatedDatabase>("create_database_container", { distro, request });
export const onDatabaseProgress = (cb: (p: DatabaseProgress) => void) => listen<DatabaseProgress>("database-progress", (e) => cb(e.payload));

// ---------------------------------------------------------------------------
// Table explorer
// ---------------------------------------------------------------------------

export interface DbTableSummary {
  /** What the other commands take: `name`, or `schema.name` outside the default schema. */
  id: string;
  name: string;
  schema: string | null;
  /** table · view · (Redis) string · hash · list · set · zset · stream */
  kind: string;
  rows: number | null;
  ttl: number | null;
}

export interface DbColumn {
  name: string;
  dataType: string;
  isNullable: boolean;
  defaultValue: string | null;
  isPrimaryKey: boolean;
  isIdentity: boolean;
}

export interface DbTableData {
  columns: DbColumn[];
  rows: Record<string, unknown>[];
  total: number | null;
  limit: number;
  offset: number;
  kind: string | null;
  ttl: number | null;
}

export interface DbQueryResult {
  columns: string[];
  rows: (string | null)[][];
  rowsAffected: number | null;
  statements: number;
  truncated: boolean;
}

export type ColumnType =
  | "identity"
  | "uuid"
  | "text"
  | "varchar"
  | "int"
  | "bigint"
  | "numeric"
  | "float"
  | "bool"
  | "date"
  | "timestamp"
  | "json";

export type ColumnDefault =
  | { kind: "value"; value: string }
  | { kind: "now" }
  | { kind: "random_uuid" }
  | { kind: "true" }
  | { kind: "false" };

export interface NewColumnDef {
  name: string;
  type: ColumnType;
  nullable: boolean;
  primaryKey: boolean;
  unique: boolean;
  default: ColumnDefault | null;
}

export const dbListTables = (distro: string, containerId: string) => invoke<DbTableSummary[]>("db_list_tables", { distro, containerId });
export const dbQueryTable = (distro: string, containerId: string, tableName: string, limit: number, offset: number) =>
  invoke<DbTableData>("db_query_table", { distro, containerId, tableName, limit, offset });
export const dbCreateTable = (distro: string, containerId: string, tableName: string, columns: NewColumnDef[]) =>
  invoke<void>("db_create_table", { distro, containerId, tableName, columns });
export const dbInsertRow = (distro: string, containerId: string, tableName: string, values: Record<string, unknown>) =>
  invoke<void>("db_insert_row", { distro, containerId, tableName, values });
export const dbDeleteRow = (distro: string, containerId: string, tableName: string, pkColumn: string, pkValue: string) =>
  invoke<void>("db_delete_row", { distro, containerId, tableName, pkColumn, pkValue });
export const dbExecuteSql = (distro: string, containerId: string, sql: string) =>
  invoke<DbQueryResult>("db_execute_sql", { distro, containerId, sql });

// ---------------------------------------------------------------------------
// Presets and plain-language copy
// ---------------------------------------------------------------------------

export interface DbPreset {
  kind: DbKind;
  title: string;
  icon: SimpleIcon;
  tagline: string;
  /** What the person will do with it, in their words. */
  goodFor: string;
  /** The form asks for these. */
  fields: { database: boolean; user: boolean; password: "required" | "optional" };
  envName: string;
}

export const DB_PRESETS: DbPreset[] = [
  {
    kind: "postgres",
    title: "PostgreSQL",
    icon: siPostgresql,
    tagline: "O mesmo banco usado pelo Supabase.",
    goodFor: "Ideal para Prisma, Drizzle, Next.js e NestJS.",
    fields: { database: true, user: true, password: "required" },
    envName: "DATABASE_URL",
  },
  {
    kind: "mysql",
    title: "MySQL",
    icon: siMysql,
    tagline: "Banco relacional clássico.",
    goodFor: "Muito usado com Laravel, WordPress, PHP e Prisma.",
    fields: { database: true, user: true, password: "required" },
    envName: "DATABASE_URL",
  },
  {
    kind: "redis",
    title: "Redis",
    icon: siRedis,
    tagline: "Banco ultrarrápido em memória.",
    goodFor: "Para cache, filas (BullMQ), sessões e contadores.",
    fields: { database: false, user: false, password: "optional" },
    envName: "REDIS_URL",
  },
  {
    kind: "mongo",
    title: "MongoDB",
    icon: siMongodb,
    tagline: "Banco NoSQL de documentos JSON.",
    goodFor: "Para Mongoose e projetos que guardam objetos como estão.",
    fields: { database: true, user: true, password: "required" },
    envName: "MONGODB_URI",
  },
];

export const DB_ICONS: Record<DbKind, SimpleIcon> = {
  postgres: siPostgresql,
  mysql: siMysql,
  mariadb: siMariadb,
  redis: siRedis,
  mongo: siMongodb,
};

export const DB_TITLES: Record<DbKind, string> = {
  postgres: "PostgreSQL",
  mysql: "MySQL",
  mariadb: "MariaDB",
  redis: "Redis",
  mongo: "MongoDB",
};

export const CONTAINER_STATE_LABEL: Record<string, string> = {
  running: "Em execução",
  exited: "Parado",
  created: "Criado (nunca ligou)",
  paused: "Pausado",
  restarting: "Reiniciando",
  dead: "Com defeito",
  removing: "Removendo",
};

/** Friendly column types for the "Nova tabela" form, in the order shown. */
export const COLUMN_TYPES: { type: ColumnType; label: string; hint: string; sql: string }[] = [
  { type: "text", label: "Texto", hint: "Qualquer tamanho: nomes, descrições", sql: "text" },
  { type: "varchar", label: "Texto curto", hint: "Até 255 letras: e-mail, código, slug", sql: "varchar(255)" },
  { type: "int", label: "Número inteiro", hint: "1, 42, -7", sql: "int4" },
  { type: "bigint", label: "Número inteiro grande", hint: "Para valores acima de 2 bilhões", sql: "int8" },
  { type: "numeric", label: "Número com casas decimais", hint: "Preços e valores exatos: 19,90", sql: "numeric" },
  { type: "float", label: "Número decimal aproximado", hint: "Medidas e médias: 3,14159", sql: "float8" },
  { type: "bool", label: "Sim / Não", hint: "Verdadeiro ou falso: ativo, pago", sql: "bool" },
  { type: "date", label: "Data", hint: "Só o dia: 2026-11-01", sql: "date" },
  { type: "timestamp", label: "Data e hora", hint: "Com horário e fuso", sql: "timestamptz" },
  { type: "uuid", label: "UUID", hint: "Identificador único aleatório", sql: "uuid" },
  { type: "json", label: "JSON", hint: "Objetos e listas: {\"a\": 1}", sql: "jsonb" },
  { type: "identity", label: "Número automático", hint: "1, 2, 3… preenchido pelo banco (só na chave primária)", sql: "int8 identity" },
];

/** Mirrors `validate_container_name` (docker.rs). */
export function containerNameError(name: string, taken: string[]): string | null {
  if (!name) return "Dê um nome ao banco";
  if (!/^[A-Za-z0-9][A-Za-z0-9_.-]{1,59}$/.test(name)) return "Use letras sem acento, números, hífen, ponto ou _ (sem espaços)";
  if (taken.includes(name)) return "Já existe um container com esse nome";
  return null;
}

/** Mirrors `validate_db_identifier` (docker.rs). */
export function dbIdentifierError(value: string, what: string): string | null {
  if (!value) return `${what}: obrigatório`;
  if (!/^[A-Za-z_][A-Za-z0-9_]{0,62}$/.test(value)) return `${what}: use letras sem acento, números e _, começando por letra`;
  return null;
}

/** Mirrors `validate_password` (docker.rs). */
export function passwordError(value: string, required: boolean): string | null {
  if (!value) return required ? "Defina uma senha" : null;
  if (value.length > 128 || !/^[\x21-\x7e]+$/.test(value) || /["'\\$`]/.test(value)) return "Sem espaços, aspas, \\, $ ou crase";
  return null;
}

/** Mirrors `validate_new_identifier` (database.rs). */
export function newIdentifierError(value: string): string | null {
  if (!value) return "Obrigatório";
  if (!/^[A-Za-z_][A-Za-z0-9_]{0,62}$/.test(value)) return "Letras sem acento, números e _, começando por letra";
  return null;
}

/** A strong password with only characters that are safe in URLs, shells and .env files. */
export function generatePassword(length = 20): string {
  const alphabet = "ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz23456789";
  const bytes = crypto.getRandomValues(new Uint8Array(length));
  return Array.from(bytes, (b) => alphabet[b % alphabet.length]).join("");
}

/** The host port a database answers on (published binding). */
export function databasePort(c: ContainerInfo): number | null {
  return c.database?.hostPort ?? c.ports[0]?.hostPort ?? null;
}
