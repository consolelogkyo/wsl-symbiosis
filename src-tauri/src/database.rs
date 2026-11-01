//! Table explorer ("mini Supabase Studio") for databases running in Docker containers.
//!
//! No database drivers: everything runs through `docker exec -i <container>` with the client that
//! ships in the image (psql, mysql/mariadb, redis-cli). SQL goes through **stdin**, never through
//! arguments, and the credentials come from the container's own environment, inside the container
//! (`POSTGRES_USER`, `MYSQL_ROOT_PASSWORD`...).
//!
//! Injection rules:
//! - identifiers are validated ([`validate_identifier`]) **and** always quoted (`"x"` / `` `x` ``);
//! - values never become SQL text: Postgres reads them from a dollar-quoted JSON document
//!   (`json_populate_record`), MySQL from hex literals (`CONVERT(X'..' USING utf8mb4)`).

use std::time::Duration;

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Map, Value};
use tauri::State;

use crate::docker::{self, DbKind, DockerManager, Engine};
use crate::wsl::Captured;

const QUERY_TIMEOUT: Duration = Duration::from_secs(60);
const SQL_TIMEOUT: Duration = Duration::from_secs(180);
/// Rows shown by the SQL editor (the rest is counted, not sent to the screen).
const MAX_RESULT_ROWS: usize = 1000;
const MAX_PAGE: u32 = 500;
const MAX_KEYS: usize = 1000;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DbTableSummary {
    /// What to send back to the other commands: `name`, or `schema.name` outside the default
    /// schema (Redis: the key).
    pub id: String,
    pub name: String,
    pub schema: Option<String>,
    /// table · view (SQL) · string · hash · list · set · zset · stream (Redis)
    pub kind: String,
    pub rows: Option<i64>,
    /// Redis: seconds to expire (-1 = never).
    pub ttl: Option<i64>,
}

fn lenient_bool<'de, D: Deserializer<'de>>(d: D) -> Result<bool, D::Error> {
    // MySQL's JSON_OBJECT turns comparisons into 0/1.
    Ok(match Value::deserialize(d)? {
        Value::Bool(b) => b,
        Value::Number(n) => n.as_i64().unwrap_or(0) != 0,
        Value::String(s) => matches!(s.as_str(), "1" | "true" | "t" | "YES"),
        _ => false,
    })
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DbColumn {
    pub name: String,
    pub data_type: String,
    #[serde(deserialize_with = "lenient_bool")]
    pub is_nullable: bool,
    pub default_value: Option<String>,
    #[serde(deserialize_with = "lenient_bool")]
    pub is_primary_key: bool,
    /// Filled by the database (identity/serial/auto_increment).
    #[serde(deserialize_with = "lenient_bool", default)]
    pub is_identity: bool,
    #[serde(default, skip_serializing)]
    pub pos: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DbTableData {
    pub columns: Vec<DbColumn>,
    /// One JSON object per row (column name → value).
    pub rows: Vec<Value>,
    pub total: Option<i64>,
    pub limit: u32,
    pub offset: u32,
    /// Redis: the key's type and TTL.
    pub kind: Option<String>,
    pub ttl: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DbQueryResult {
    /// Result of the last statement that returned rows (empty when none did).
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Option<String>>>,
    /// Rows returned (SELECT) or changed (INSERT/UPDATE/DELETE) by the last statement.
    pub rows_affected: Option<i64>,
    pub statements: usize,
    /// More rows than [`MAX_RESULT_ROWS`] came back.
    pub truncated: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ColumnType {
    /// Auto-numbered id (identity / auto_increment).
    Identity,
    Uuid,
    Text,
    Varchar,
    Int,
    Bigint,
    Numeric,
    Float,
    Bool,
    Date,
    Timestamp,
    Json,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum ColumnDefault {
    Value(String),
    Now,
    RandomUuid,
    True,
    False,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewColumnDef {
    pub name: String,
    #[serde(rename = "type")]
    pub column_type: ColumnType,
    #[serde(default)]
    pub nullable: bool,
    #[serde(default)]
    pub primary_key: bool,
    #[serde(default)]
    pub unique: bool,
    #[serde(default)]
    pub default: Option<ColumnDefault>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dialect {
    Postgres,
    MySql,
}

// ---------------------------------------------------------------------------
// Identifiers and literals
// ---------------------------------------------------------------------------

/// Names of existing tables and columns: letters (accents too), digits, `_`, `-` and inner
/// spaces, up to 63 bytes. No quotes, dots, backslashes or control characters, so a quoted
/// identifier can never be closed early.
pub fn validate_identifier(name: &str) -> Result<(), String> {
    let ok = !name.is_empty()
        && name.len() <= 63
        && name.trim() == name
        && name.chars().all(|c| c.is_alphanumeric() || matches!(c, '_' | '-' | ' '));
    if ok { Ok(()) } else { Err(format!("Nome inválido: {name:?}")) }
}

/// Names of new tables and columns: `snake_case` friendly, the same in every ORM.
pub fn validate_new_identifier(name: &str) -> Result<(), String> {
    let ok = !name.is_empty()
        && name.len() <= 63
        && name.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    if ok {
        Ok(())
    } else {
        Err(format!("Nome inválido: {name:?}. Use letras sem acento, números e _, começando por letra (ex.: clientes, created_at)."))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableRef {
    pub schema: Option<String>,
    pub name: String,
}

/// `clientes` or `schema.clientes`.
pub fn parse_table_ref(s: &str) -> Result<TableRef, String> {
    let (schema, name) = match s.split_once('.') {
        Some((schema, name)) => (Some(schema), name),
        None => (None, s),
    };
    if let Some(schema) = schema {
        validate_identifier(schema)?;
    }
    validate_identifier(name)?;
    Ok(TableRef { schema: schema.map(String::from), name: name.to_string() })
}

pub fn pg_ident(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}

pub fn my_ident(s: &str) -> String {
    format!("`{}`", s.replace('`', "``"))
}

impl TableRef {
    pub fn pg(&self) -> String {
        format!("{}.{}", pg_ident(self.schema.as_deref().unwrap_or("public")), pg_ident(&self.name))
    }

    pub fn my(&self) -> String {
        match &self.schema {
            Some(s) => format!("{}.{}", my_ident(s), my_ident(&self.name)),
            None => my_ident(&self.name),
        }
    }

    /// information_schema filter: the given schema or the connection's database.
    fn my_schema_expr(&self) -> String {
        // Validated identifiers have no quotes or backslashes: a plain literal is safe.
        self.schema.as_ref().map(|s| format!("'{s}'")).unwrap_or_else(|| "DATABASE()".into())
    }
}

/// Postgres dollar-quoted literal whose tag doesn't occur in the value: nothing inside it is
/// ever interpreted (quotes, backslashes, psql's `:variables` and `\commands`).
pub fn pg_literal(s: &str) -> String {
    let tag = (0..)
        .map(|i| if i == 0 { "$symb$".to_string() } else { format!("$symb{i}$") })
        .find(|t| !s.contains(t.as_str()))
        .unwrap();
    format!("{tag}{s}{tag}")
}

/// MySQL string literal as hex: no quoting or backslash rules involved.
pub fn my_literal(s: &str) -> String {
    if s.is_empty() {
        return "''".into();
    }
    let hex: String = s.bytes().map(|b| format!("{b:02X}")).collect();
    format!("CONVERT(X'{hex}' USING utf8mb4)")
}

fn my_value(v: &Value) -> String {
    match v {
        Value::Null => "NULL".into(),
        Value::Bool(b) => if *b { "TRUE" } else { "FALSE" }.into(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => my_literal(s),
        other => my_literal(&other.to_string()),
    }
}

// ---------------------------------------------------------------------------
// SQL generation (pure, tested)
// ---------------------------------------------------------------------------

fn check_values(values: &Map<String, Value>) -> Result<(), String> {
    if values.len() > 500 {
        return Err("Colunas demais.".into());
    }
    values.keys().try_for_each(|k| validate_identifier(k))
}

/// INSERT that prints how many rows went in.
pub fn insert_sql(dialect: Dialect, t: &TableRef, values: &Map<String, Value>) -> Result<String, String> {
    check_values(values)?;
    Ok(match dialect {
        Dialect::Postgres => {
            let insert = if values.is_empty() {
                format!("INSERT INTO {} DEFAULT VALUES", t.pg())
            } else {
                let cols = values.keys().map(|k| pg_ident(k)).collect::<Vec<_>>().join(", ");
                // The JSON document is the only place the values appear; Postgres converts each
                // one to its column's type.
                let doc = pg_literal(&Value::Object(values.clone()).to_string());
                format!("INSERT INTO {t} ({cols}) SELECT {cols} FROM json_populate_record(NULL::{t}, {doc}::json)", t = t.pg())
            };
            format!("WITH symb_ins AS ({insert} RETURNING 1) SELECT count(*) FROM symb_ins;")
        }
        Dialect::MySql => {
            let cols = values.keys().map(|k| my_ident(k)).collect::<Vec<_>>().join(", ");
            let vals = values.values().map(my_value).collect::<Vec<_>>().join(", ");
            format!("INSERT INTO {} ({cols}) VALUES ({vals});\nSELECT ROW_COUNT();", t.my())
        }
    })
}

/// DELETE of one row by primary key that prints how many rows went away.
pub fn delete_sql(dialect: Dialect, t: &TableRef, pk_column: &str, pk_value: &str) -> Result<String, String> {
    validate_identifier(pk_column)?;
    Ok(match dialect {
        Dialect::Postgres => format!(
            "WITH symb_del AS (DELETE FROM {} WHERE {} = {} RETURNING 1) SELECT count(*) FROM symb_del;",
            t.pg(),
            pg_ident(pk_column),
            pg_literal(pk_value)
        ),
        Dialect::MySql => format!(
            "DELETE FROM {} WHERE {} = {} LIMIT 1;\nSELECT ROW_COUNT();",
            t.my(),
            my_ident(pk_column),
            my_literal(pk_value)
        ),
    })
}

fn column_type_sql(dialect: Dialect, ty: ColumnType) -> &'static str {
    use ColumnType::*;
    match (dialect, ty) {
        (Dialect::Postgres, Identity) => "bigint GENERATED BY DEFAULT AS IDENTITY",
        (Dialect::Postgres, Uuid) => "uuid",
        (Dialect::Postgres, Text) => "text",
        (Dialect::Postgres, Varchar) => "varchar(255)",
        (Dialect::Postgres, Int) => "integer",
        (Dialect::Postgres, Bigint) => "bigint",
        (Dialect::Postgres, Numeric) => "numeric",
        (Dialect::Postgres, Float) => "double precision",
        (Dialect::Postgres, Bool) => "boolean",
        (Dialect::Postgres, Date) => "date",
        (Dialect::Postgres, Timestamp) => "timestamptz",
        (Dialect::Postgres, Json) => "jsonb",
        (Dialect::MySql, Identity) => "BIGINT AUTO_INCREMENT",
        (Dialect::MySql, Uuid) => "CHAR(36)",
        (Dialect::MySql, Text) => "TEXT",
        (Dialect::MySql, Varchar) => "VARCHAR(255)",
        (Dialect::MySql, Int) => "INT",
        (Dialect::MySql, Bigint) => "BIGINT",
        (Dialect::MySql, Numeric) => "DECIMAL(12,2)",
        (Dialect::MySql, Float) => "DOUBLE",
        (Dialect::MySql, Bool) => "BOOLEAN",
        (Dialect::MySql, Date) => "DATE",
        (Dialect::MySql, Timestamp) => "DATETIME(3)",
        (Dialect::MySql, Json) => "JSON",
    }
}

fn default_sql(dialect: Dialect, col: &NewColumnDef) -> Result<Option<String>, String> {
    use ColumnType as T;
    let Some(default) = &col.default else { return Ok(None) };
    let wrong = || Err(format!("O valor padrão escolhido não combina com o tipo da coluna {}.", col.name));
    let sql = match (default, col.column_type, dialect) {
        (_, T::Identity, _) => return Err(format!("A coluna {} já é numerada automaticamente: ela não leva valor padrão.", col.name)),
        (ColumnDefault::Now, T::Timestamp, Dialect::Postgres) => "now()".into(),
        (ColumnDefault::Now, T::Date, Dialect::Postgres) => "CURRENT_DATE".into(),
        (ColumnDefault::Now, T::Timestamp, Dialect::MySql) => "CURRENT_TIMESTAMP(3)".into(),
        (ColumnDefault::Now, T::Date, Dialect::MySql) => "(CURRENT_DATE)".into(),
        (ColumnDefault::RandomUuid, T::Uuid, Dialect::Postgres) => "gen_random_uuid()".into(),
        (ColumnDefault::RandomUuid, T::Uuid, Dialect::MySql) => "(UUID())".into(),
        (ColumnDefault::True, T::Bool, _) => "TRUE".into(),
        (ColumnDefault::False, T::Bool, _) => "FALSE".into(),
        (ColumnDefault::Value(v), _, Dialect::Postgres) => pg_literal(v),
        // An expression default (8.0.13+ / MariaDB 10.2+) works on TEXT and JSON too.
        (ColumnDefault::Value(v), _, Dialect::MySql) => format!("({})", my_literal(v)),
        _ => return wrong(),
    };
    Ok(Some(sql))
}

/// `CREATE TABLE` from the visual form.
pub fn create_table_sql(dialect: Dialect, t: &TableRef, columns: &[NewColumnDef]) -> Result<String, String> {
    validate_new_identifier(&t.name)?;
    if columns.is_empty() {
        return Err("Adicione pelo menos uma coluna.".into());
    }
    if columns.len() > 200 {
        return Err("Colunas demais (máximo 200).".into());
    }
    let mut seen = std::collections::HashSet::new();
    let pk: Vec<&NewColumnDef> = columns.iter().filter(|c| c.primary_key).collect();
    let mut lines = Vec::new();
    for col in columns {
        validate_new_identifier(&col.name)?;
        if !seen.insert(col.name.to_ascii_lowercase()) {
            return Err(format!("A coluna {} aparece duas vezes.", col.name));
        }
        if col.column_type == ColumnType::Identity && !col.primary_key {
            return Err(format!("A numeração automática ({}) só vale para a chave primária.", col.name));
        }
        let long_text = dialect == Dialect::MySql && matches!(col.column_type, ColumnType::Text | ColumnType::Json);
        if long_text && (col.primary_key || col.unique) {
            return Err(format!("No MySQL, {} não pode ser chave primária nem única com esse tipo: use Texto curto.", col.name));
        }
        let quote = match dialect {
            Dialect::Postgres => pg_ident(&col.name),
            Dialect::MySql => my_ident(&col.name),
        };
        let mut line = format!("  {quote} {}", column_type_sql(dialect, col.column_type));
        if !col.nullable && !col.primary_key {
            line.push_str(" NOT NULL");
        }
        if let Some(default) = default_sql(dialect, col)? {
            line.push_str(&format!(" DEFAULT {default}"));
        }
        if col.unique && !col.primary_key {
            line.push_str(" UNIQUE");
        }
        if col.primary_key && pk.len() == 1 {
            line.push_str(" PRIMARY KEY");
        }
        lines.push(line);
    }
    if pk.len() > 1 {
        let names: Vec<String> = pk
            .iter()
            .map(|c| match dialect {
                Dialect::Postgres => pg_ident(&c.name),
                Dialect::MySql => my_ident(&c.name),
            })
            .collect();
        lines.push(format!("  PRIMARY KEY ({})", names.join(", ")));
    }
    let table = match dialect {
        Dialect::Postgres => t.pg(),
        Dialect::MySql => t.my(),
    };
    Ok(format!("CREATE TABLE {table} (\n{}\n);", lines.join(",\n")))
}

/// Splits a SQL script into statements on `;` outside strings, quoted names, comments and
/// (Postgres) dollar quotes. Comment-only pieces are dropped.
pub fn split_statements(sql: &str, dialect: Dialect) -> Vec<String> {
    #[derive(PartialEq)]
    enum S {
        Normal,
        Single { backslash: bool },
        Double,
        Backtick,
        Line,
        Block(u32),
        Dollar(String),
    }
    let chars: Vec<(usize, char)> = sql.char_indices().collect();
    let at = |i: usize| chars.get(i).map(|c| c.1);
    let mut out = Vec::new();
    let mut state = S::Normal;
    let mut start = 0usize;
    let mut has_code = false;
    let mut i = 0;
    let push = |out: &mut Vec<String>, from: usize, to: usize, has_code: bool| {
        let piece = sql[from..to].trim();
        if has_code && !piece.is_empty() {
            out.push(piece.to_string());
        }
    };
    while i < chars.len() {
        let c = chars[i].1;
        match &state {
            S::Normal => match c {
                ';' => {
                    push(&mut out, start, chars[i].0, has_code);
                    start = chars[i].0 + 1;
                    has_code = false;
                }
                '-' if at(i + 1) == Some('-') => {
                    state = S::Line;
                    i += 1;
                }
                '#' if dialect == Dialect::MySql => state = S::Line,
                '/' if at(i + 1) == Some('*') => {
                    state = S::Block(1);
                    i += 1;
                }
                '\'' => {
                    has_code = true;
                    let prev = if i > 0 { at(i - 1) } else { None };
                    let before = if i > 1 { at(i - 2) } else { None };
                    let e_string = matches!(prev, Some('E' | 'e')) && !before.is_some_and(|b| b.is_alphanumeric() || b == '_');
                    state = S::Single { backslash: dialect == Dialect::MySql || e_string };
                }
                '"' => {
                    has_code = true;
                    state = S::Double;
                }
                '`' if dialect == Dialect::MySql => {
                    has_code = true;
                    state = S::Backtick;
                }
                '$' if dialect == Dialect::Postgres && !(i > 0 && at(i - 1).is_some_and(|p| p.is_alphanumeric() || p == '_')) => {
                    has_code = true;
                    // $tag$ or $$ (not $1, a parameter).
                    let mut j = i + 1;
                    while at(j).is_some_and(|ch| ch.is_alphanumeric() || ch == '_') {
                        j += 1;
                    }
                    let tag_ok = at(j) == Some('$') && !at(i + 1).is_some_and(|ch| ch.is_ascii_digit());
                    if tag_ok {
                        let tag: String = chars[i..=j].iter().map(|c| c.1).collect();
                        state = S::Dollar(tag);
                        i = j;
                    }
                }
                c if !c.is_whitespace() => has_code = true,
                _ => {}
            },
            S::Single { backslash } => {
                if *backslash && c == '\\' {
                    i += 1;
                } else if c == '\'' {
                    if at(i + 1) == Some('\'') {
                        i += 1;
                    } else {
                        state = S::Normal;
                    }
                }
            }
            S::Double | S::Backtick => {
                let q = if state == S::Double { '"' } else { '`' };
                if c == q {
                    if at(i + 1) == Some(q) {
                        i += 1;
                    } else {
                        state = S::Normal;
                    }
                }
            }
            S::Line => {
                if c == '\n' {
                    state = S::Normal;
                }
            }
            S::Block(depth) => {
                let depth = *depth;
                if c == '*' && at(i + 1) == Some('/') {
                    state = if depth == 1 { S::Normal } else { S::Block(depth - 1) };
                    i += 1;
                } else if c == '/' && at(i + 1) == Some('*') && dialect == Dialect::Postgres {
                    state = S::Block(depth + 1);
                    i += 1;
                }
            }
            S::Dollar(tag) => {
                let len = tag.chars().count();
                if c == '$' && chars.get(i..i + len).is_some_and(|s| s.iter().map(|c| c.1).eq(tag.chars())) {
                    i += len - 1;
                    state = S::Normal;
                }
            }
        }
        i += 1;
    }
    push(&mut out, start, sql.len(), has_code);
    out
}

/// RFC 4180 CSV (psql `--csv`): quoted fields may hold commas, quotes and newlines.
pub fn parse_csv(text: &str) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = text.chars().peekable();
    let mut any = false;
    while let Some(c) = chars.next() {
        any = true;
        if quoted {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    field.push('"');
                    chars.next();
                } else {
                    quoted = false;
                }
            } else {
                field.push(c);
            }
            continue;
        }
        match c {
            '"' => quoted = true,
            ',' => row.push(std::mem::take(&mut field)),
            '\r' => {}
            '\n' => {
                row.push(std::mem::take(&mut field));
                rows.push(std::mem::take(&mut row));
                any = false;
            }
            c => field.push(c),
        }
    }
    if any {
        row.push(field);
        rows.push(row);
    }
    rows
}

/// mysql `--batch` cell: `\t`, `\n`, `\\`, `\0` escapes; `NULL` is NULL.
fn mysql_cell(s: &str) -> Option<String> {
    if s == "NULL" {
        return None;
    }
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('t') => out.push('\t'),
                Some('n') => out.push('\n'),
                Some('0') => out.push('\0'),
                Some(other) => out.push(other),
                None => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    Some(out)
}

/// A token that can't appear in user data by accident (separates outputs of one script).
fn token() -> String {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    format!("SYMB{nanos:x}{:x}", std::process::id())
}

/// The editor's statements for psql (`--csv -q`): after each one, a marker line with
/// `:ROW_COUNT`. With `-q`, only statements that return rows print anything.
pub fn pg_editor_script(statements: &[String], mark: &str, null: &str) -> String {
    let mut script = format!("\\pset null '{null}'\n");
    for s in statements {
        // The `;` on its own line: a trailing `-- comment` can't swallow it.
        script.push_str(&format!("{s}\n;\n\\echo {mark} :ROW_COUNT\n"));
    }
    script
}

pub fn parse_pg_editor_output(stdout: &str, mark: &str, null: &str) -> DbQueryResult {
    let mut result = DbQueryResult::default();
    let mut block: Vec<&str> = Vec::new();
    for line in stdout.split('\n') {
        if let Some(rest) = line.strip_prefix(mark).and_then(|r| r.strip_prefix(' ')) {
            result.statements += 1;
            let affected = rest.trim().parse::<i64>().ok();
            if !block.is_empty() {
                let mut rows = parse_csv(&block.join("\n"));
                if !rows.is_empty() {
                    result.columns = rows.remove(0);
                    result.truncated = rows.len() > MAX_RESULT_ROWS;
                    rows.truncate(MAX_RESULT_ROWS);
                    result.rows = rows
                        .into_iter()
                        .map(|r| r.into_iter().map(|v| if v == null { None } else { Some(v) }).collect())
                        .collect();
                }
            }
            result.rows_affected = affected;
            block.clear();
        } else {
            block.push(line);
        }
    }
    result
}

pub fn my_editor_script(statements: &[String], mark: &str) -> String {
    statements.iter().map(|s| format!("{s}\n;\nSELECT ROW_COUNT() AS {mark};\n")).collect()
}

pub fn parse_my_editor_output(stdout: &str, mark: &str) -> DbQueryResult {
    let mut result = DbQueryResult::default();
    let mut block: Vec<&str> = Vec::new();
    let mut lines = stdout.lines();
    while let Some(line) = lines.next() {
        if line == mark {
            result.statements += 1;
            let count = lines.next().and_then(|l| l.trim().parse::<i64>().ok());
            if let Some((header, rows)) = block.split_first() {
                result.columns = header.split('\t').map(|h| mysql_cell(h).unwrap_or_default()).collect();
                result.truncated = rows.len() > MAX_RESULT_ROWS;
                result.rows = rows.iter().take(MAX_RESULT_ROWS).map(|r| r.split('\t').map(mysql_cell).collect()).collect();
                result.rows_affected = Some(rows.len() as i64);
            } else {
                result.rows_affected = count.filter(|c| *c >= 0);
            }
            block.clear();
        } else {
            block.push(line);
        }
    }
    result
}

/// Lua run with `redis-cli EVAL`: up to [`MAX_KEYS`] keys with type and TTL, as JSON. One line:
/// no `--` comments.
const REDIS_KEYS_LUA: &str = "local out, cur = {}, '0' \
repeat local r = redis.call('SCAN', cur, 'COUNT', 500) cur = r[1] \
for _, k in ipairs(r[2]) do if #out < tonumber(ARGV[1]) then out[#out + 1] = {k, redis.call('TYPE', k).ok, redis.call('TTL', k)} end end \
until cur == '0' or #out >= tonumber(ARGV[1]) \
return cjson.encode({keys = out, total = redis.call('DBSIZE')})";

/// One key's content as a small table: `columns` + `rows` (+ type, TTL, total).
const REDIS_VALUE_LUA: &str = "local k, off, lim = KEYS[1], tonumber(ARGV[1]), tonumber(ARGV[2]) \
local t = redis.call('TYPE', k).ok \
local res = {type = t, ttl = redis.call('TTL', k), columns = {}, rows = {}, total = 0} \
if t == 'string' then res.columns = {'value'} res.total = 1 res.rows = {{redis.call('GET', k)}} \
elseif t == 'hash' then res.columns = {'field', 'value'} res.total = redis.call('HLEN', k) \
local all = redis.call('HGETALL', k) for i = off * 2 + 1, math.min(#all, (off + lim) * 2), 2 do res.rows[#res.rows + 1] = {all[i], all[i + 1]} end \
elseif t == 'list' then res.columns = {'index', 'value'} res.total = redis.call('LLEN', k) \
for i, v in ipairs(redis.call('LRANGE', k, off, off + lim - 1)) do res.rows[#res.rows + 1] = {tostring(off + i - 1), v} end \
elseif t == 'set' then res.columns = {'value'} res.total = redis.call('SCARD', k) \
local all = redis.call('SMEMBERS', k) for i = off + 1, math.min(#all, off + lim) do res.rows[#res.rows + 1] = {all[i]} end \
elseif t == 'zset' then res.columns = {'value', 'score'} res.total = redis.call('ZCARD', k) \
local all = redis.call('ZRANGE', k, off, off + lim - 1, 'WITHSCORES') for i = 1, #all, 2 do res.rows[#res.rows + 1] = {all[i], all[i + 1]} end \
elseif t == 'stream' then res.columns = {'id', 'fields'} res.total = redis.call('XLEN', k) \
local all = redis.call('XRANGE', k, '-', '+', 'COUNT', off + lim) for i = off + 1, #all do res.rows[#res.rows + 1] = {all[i][1], cjson.encode(all[i][2])} end \
end return cjson.encode(res)";

/// Redis keys travel as arguments: no control characters.
pub fn validate_redis_key(key: &str) -> Result<(), String> {
    if key.is_empty() || key.len() > 1024 || key.chars().any(char::is_control) {
        Err("Chave inválida: use até 1024 caracteres, sem quebras de linha.".into())
    } else {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Running clients inside the container
// ---------------------------------------------------------------------------

/// psql as the container's own user, on the local socket.
const PG_CLIENT: &str = r#"export PGPASSWORD="${PGPASSWORD:-${POSTGRES_PASSWORD:-}}"
exec psql -X -q -v ON_ERROR_STOP=1 -P pager=off -U "${POSTGRES_USER:-postgres}" -d "${POSTGRES_DB:-${POSTGRES_USER:-postgres}}" "$@""#;

/// mysql/mariadb as root when its password is known, else as the app user.
const MY_CLIENT: &str = r#"u=root; p="${MYSQL_ROOT_PASSWORD:-${MARIADB_ROOT_PASSWORD:-}}"
au="${MYSQL_USER:-${MARIADB_USER:-}}"
if [ -z "$p" ] && [ -n "$au" ]; then u="$au"; p="${MYSQL_PASSWORD:-${MARIADB_PASSWORD:-}}"; fi
db="${MYSQL_DATABASE:-${MARIADB_DATABASE:-}}"
c=mysql; command -v mariadb >/dev/null 2>&1 && c=mariadb
if [ -n "$db" ]; then set -- -D "$db" "$@"; fi
if [ -n "$p" ]; then set -- --password="$p" "$@"; fi
exec "$c" -u"$u" --default-character-set=utf8mb4 "$@""#;

const REDIS_CLIENT: &str = r#"if [ -z "${REDISCLI_AUTH:-}" ] && [ -n "${REDIS_PASSWORD:-}" ]; then export REDISCLI_AUTH="$REDIS_PASSWORD"; fi
exec redis-cli --no-auth-warning "$@""#;

struct Target {
    engine: Engine,
    id: String,
    kind: DbKind,
    /// `--requirepass` from the command line (not in the container's environment).
    redis_auth: Option<String>,
}

impl Target {
    fn dialect(&self) -> Result<Dialect, String> {
        match self.kind {
            DbKind::Postgres => Ok(Dialect::Postgres),
            DbKind::Mysql | DbKind::Mariadb => Ok(Dialect::MySql),
            DbKind::Redis => Err("No Redis não há tabelas: use as chaves.".into()),
            DbKind::Mongo => Err(MONGO_UNSUPPORTED.into()),
        }
    }

    fn client(&self) -> &'static str {
        match self.kind {
            DbKind::Postgres => PG_CLIENT,
            DbKind::Mysql | DbKind::Mariadb => MY_CLIENT,
            _ => REDIS_CLIENT,
        }
    }

    async fn exec(&self, args: &[&str], stdin: &[u8], timeout: Duration) -> Result<Captured, String> {
        let mut all = vec!["exec", "-i"];
        let mut secrets = Vec::new();
        if let Some(auth) = &self.redis_auth {
            all.extend(["-e", "REDISCLI_AUTH"]);
            secrets.push(("REDISCLI_AUTH", auth.as_str()));
        }
        all.extend([self.id.as_str(), "sh", "-c", self.client(), "sh"]);
        all.extend_from_slice(args);
        self.engine.run(&all, timeout, Some(stdin), &secrets).await
    }

    /// stdout of a successful run, or the explained error.
    async fn query(&self, args: &[&str], stdin: &str, timeout: Duration) -> Result<String, String> {
        let out = self.exec(args, stdin.as_bytes(), timeout).await?;
        if out.success() {
            Ok(String::from_utf8_lossy(&out.stdout).into_owned())
        } else {
            Err(explain_db_error(&out.error_text()))
        }
    }

    /// One JSON value per line / per marker block (psql `-t -A`, mysql `-N -B -r`).
    async fn json_query(&self, sql: &str) -> Result<String, String> {
        match self.kind {
            DbKind::Postgres => self.query(&["-t", "-A"], sql, QUERY_TIMEOUT).await,
            _ => self.query(&["-N", "-B", "-r"], sql, QUERY_TIMEOUT).await,
        }
    }
}

const MONGO_UNSUPPORTED: &str =
    "O explorador de tabelas ainda não abre MongoDB. Use o link de conexão no MongoDB Compass ou na extensão do VS Code.";

async fn target(docker: &DockerManager, distro: &str, container: &str) -> Result<Target, String> {
    docker::validate_container_ref(container)?;
    let engine = docker.engine(distro).await?;
    let info = docker::inspect(&engine, &[container])
        .await?
        .into_iter()
        .next()
        .ok_or("Esse container não existe mais. Atualize a lista.")?;
    let kind = info.kind.ok_or("Esse container não é um banco de dados que o explorador conheça.")?;
    if !info.running {
        return Err("O banco está parado. Ligue-o primeiro.".into());
    }
    let redis_auth = match kind {
        DbKind::Redis if !info.env.contains_key("REDIS_PASSWORD") => info.database.and_then(|d| d.password),
        _ => None,
    };
    Ok(Target { engine, id: info.id, kind, redis_auth })
}

/// Drops the noise clients add around the real message.
fn clean_client_error(raw: &str) -> String {
    raw.lines()
        .filter(|l| !l.contains("[Warning] Using a password on the command line"))
        .map(|l| {
            // "psql:<stdin>:3: ERROR:  x" → "ERROR:  x"
            let l = match l.strip_prefix("psql:") {
                Some(rest) => rest.splitn(3, ':').nth(2).map(str::trim_start).unwrap_or(l),
                None => l,
            };
            // "ERROR 1146 (42S02) at line 2: x" → "ERROR 1146 (42S02): x"
            match (l.find(" at line "), l.starts_with("ERROR ")) {
                (Some(at), true) => match l[at..].find(':') {
                    Some(colon) => format!("{}{}", &l[..at], &l[at + colon..]),
                    None => l.to_string(),
                },
                _ => l.to_string(),
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string()
}

/// First line in plain Portuguese, then the database's own message.
pub fn explain_db_error(raw: &str) -> String {
    let raw = clean_client_error(raw);
    let l = raw.to_ascii_lowercase();
    let has = |needles: &[&str]| needles.iter().any(|n| l.contains(n));
    let hint = if has(&["is not running", "is restarting"]) {
        "O banco está parado. Ligue-o primeiro."
    } else if has(&["the database system is starting up", "could not connect to server", "connection to server on socket", "can't connect to local", "error 2002", "connection refused", "loading the dataset in memory"]) {
        "O banco ainda está iniciando. Aguarde alguns segundos e tente de novo."
    } else if has(&["already exists"]) {
        "Já existe uma tabela (ou coluna) com esse nome."
    } else if has(&["does not exist", "doesn't exist", "unknown table"]) {
        "Essa tabela ou coluna não existe (talvez tenha sido apagada). Atualize a lista."
    } else if has(&["duplicate key", "duplicate entry"]) {
        "Já existe uma linha com esse valor num campo que não pode repetir (único ou chave primária)."
    } else if has(&["violates not-null", "cannot be null", "doesn't have a default value"]) {
        "Faltou preencher um campo obrigatório."
    } else if has(&["foreign key"]) {
        "Essa linha está ligada a outra tabela (chave estrangeira)."
    } else if has(&["invalid input syntax", "incorrect integer value", "incorrect datetime value", "incorrect date value", "invalid json", "out of range", "data too long", "invalid input value"]) {
        "Um dos valores não combina com o tipo da coluna."
    } else if has(&["syntax error", "error 1064"]) {
        "Há um erro de digitação no SQL."
    } else if has(&["permission denied", "access denied", "noauth", "wrongpass"]) {
        "O usuário do banco não tem permissão para isso (ou a senha não confere)."
    } else if has(&["executable file not found", "not found in $path", "psql: not found", "mysql: not found", "redis-cli: not found"]) {
        "A imagem desse container não traz o programa cliente do banco."
    } else if raw.is_empty() {
        return "O banco não respondeu.".into();
    } else {
        return raw;
    };
    format!("{hint}\n{raw}")
}

fn split_blocks<'a>(stdout: &'a str, mark: &str) -> Vec<String> {
    stdout.split(&format!("{mark}\n")).map(|b| b.trim().to_string()).collect()
}

fn parse_json<T: for<'de> Deserialize<'de>>(text: &str) -> Result<T, String> {
    serde_json::from_str(text.trim()).map_err(|e| format!("Resposta inesperada do banco: {e}\n{}", text.chars().take(300).collect::<String>()))
}

/// cjson encodes an empty Lua table as `{}`.
fn json_array(v: &Value) -> Vec<Value> {
    v.as_array().cloned().unwrap_or_default()
}

// ---------------------------------------------------------------------------
// SQL used by the explorer
// ---------------------------------------------------------------------------

const PG_LIST_TABLES: &str = r#"SELECT coalesce(json_agg(t ORDER BY t.schema <> 'public', t.schema, t.name), '[]'::json) FROM (
  SELECT n.nspname AS schema, c.relname AS name,
    CASE WHEN c.relkind IN ('v', 'm') THEN 'view' ELSE 'table' END AS kind,
    CASE WHEN c.relkind IN ('r', 'p') AND c.reltuples < 100000 THEN
      (xpath('/row/c/text()', query_to_xml(format('SELECT count(*) AS c FROM %I.%I', n.nspname, c.relname), false, true, '')))[1]::text::bigint
      WHEN c.reltuples >= 0 THEN c.reltuples::bigint END AS rows
  FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace
  WHERE c.relkind IN ('r', 'p', 'v', 'm') AND NOT c.relispartition
    AND n.nspname NOT IN ('pg_catalog', 'information_schema') AND n.nspname NOT LIKE 'pg\_%'
) t;"#;

const MY_LIST_TABLES: &str = r#"SET SESSION group_concat_max_len = 1048576;
SELECT COALESCE(JSON_ARRAYAGG(JSON_OBJECT('schema', TABLE_SCHEMA, 'name', TABLE_NAME, 'kind', IF(TABLE_TYPE = 'VIEW', 'view', 'table'),
  'rows', TABLE_ROWS, 'isDefault', TABLE_SCHEMA = DATABASE())), JSON_ARRAY())
  FROM information_schema.TABLES WHERE (DATABASE() IS NOT NULL AND TABLE_SCHEMA = DATABASE())
  OR (DATABASE() IS NULL AND TABLE_SCHEMA NOT IN ('mysql', 'information_schema', 'performance_schema', 'sys'));
SET @symb_q = (SELECT GROUP_CONCAT(CONCAT('SELECT ', QUOTE(TABLE_SCHEMA), ' s, ', QUOTE(TABLE_NAME), ' n, COUNT(*) c FROM `',
  REPLACE(TABLE_SCHEMA, '`', '``'), '`.`', REPLACE(TABLE_NAME, '`', '``'), '`') SEPARATOR ' UNION ALL ')
  FROM information_schema.TABLES WHERE TABLE_TYPE = 'BASE TABLE' AND ((DATABASE() IS NOT NULL AND TABLE_SCHEMA = DATABASE())
  OR (DATABASE() IS NULL AND TABLE_SCHEMA NOT IN ('mysql', 'information_schema', 'performance_schema', 'sys'))));
SET @symb_q = CONCAT('SELECT COALESCE(JSON_ARRAYAGG(JSON_OBJECT(''schema'', s, ''name'', n, ''rows'', c)), JSON_ARRAY()) FROM (',
  COALESCE(@symb_q, 'SELECT NULL s, NULL n, 0 c FROM DUAL WHERE FALSE'), ') x');
PREPARE symb_s FROM @symb_q; EXECUTE symb_s; DEALLOCATE PREPARE symb_s;"#;

fn pg_table_script(t: &TableRef, limit: u32, offset: u32, mark: &str) -> String {
    let rel = format!("{}::regclass", pg_literal(&t.pg()));
    format!(
        r#"SELECT coalesce(string_agg(quote_ident(a.attname), ', ' ORDER BY array_position(i.indkey::int2[], a.attnum)), '(SELECT NULL)') AS symb_order
  FROM pg_index i JOIN pg_attribute a ON a.attrelid = i.indrelid AND a.attnum = ANY(i.indkey)
  WHERE i.indrelid = {rel} AND i.indisprimary \gset
SELECT coalesce(json_agg(c ORDER BY c.pos), '[]'::json) FROM (
  SELECT a.attnum AS pos, a.attname AS name, format_type(a.atttypid, a.atttypmod) AS "dataType",
    NOT a.attnotnull AS "isNullable", pg_get_expr(d.adbin, d.adrelid) AS "defaultValue",
    coalesce(a.attnum = ANY(p.indkey), false) AS "isPrimaryKey",
    (a.attidentity <> '' OR coalesce(pg_get_expr(d.adbin, d.adrelid), '') LIKE 'nextval(%') AS "isIdentity"
  FROM pg_attribute a
  LEFT JOIN pg_attrdef d ON d.adrelid = a.attrelid AND d.adnum = a.attnum
  LEFT JOIN pg_index p ON p.indrelid = a.attrelid AND p.indisprimary
  WHERE a.attrelid = {rel} AND a.attnum > 0 AND NOT a.attisdropped
) c;
\echo {mark}
SELECT count(*) FROM {t};
\echo {mark}
SELECT coalesce(json_agg(r), '[]'::json) FROM (SELECT * FROM {t} ORDER BY :symb_order LIMIT {limit} OFFSET {offset}) r;
"#,
        t = t.pg()
    )
}

fn my_table_script(t: &TableRef, limit: u32, offset: u32) -> String {
    let schema = t.my_schema_expr();
    let name = &t.name;
    // Inside a string literal below: validated identifiers have no quotes.
    let table = t.my();
    format!(
        r#"SET SESSION group_concat_max_len = 1048576;
SELECT COALESCE(JSON_ARRAYAGG(JSON_OBJECT('name', COLUMN_NAME, 'dataType', COLUMN_TYPE, 'isNullable', IS_NULLABLE = 'YES',
  'defaultValue', COLUMN_DEFAULT, 'isPrimaryKey', COLUMN_KEY = 'PRI', 'isIdentity', EXTRA LIKE '%auto_increment%', 'pos', ORDINAL_POSITION)), JSON_ARRAY())
  FROM information_schema.COLUMNS WHERE TABLE_SCHEMA = {schema} AND TABLE_NAME = '{name}';
SELECT COUNT(*) FROM {table};
SET @symb_cols = (SELECT GROUP_CONCAT(CONCAT(QUOTE(COLUMN_NAME), ', `', REPLACE(COLUMN_NAME, '`', '``'), '`') ORDER BY ORDINAL_POSITION SEPARATOR ', ')
  FROM information_schema.COLUMNS WHERE TABLE_SCHEMA = {schema} AND TABLE_NAME = '{name}');
SET @symb_ord = (SELECT GROUP_CONCAT(CONCAT('`', REPLACE(COLUMN_NAME, '`', '``'), '`') ORDER BY SEQ_IN_INDEX SEPARATOR ', ')
  FROM information_schema.STATISTICS WHERE TABLE_SCHEMA = {schema} AND TABLE_NAME = '{name}' AND INDEX_NAME = 'PRIMARY');
SET @symb_q = CONCAT('SELECT COALESCE(JSON_ARRAYAGG(JSON_OBJECT(', @symb_cols, ')), JSON_ARRAY()) FROM (SELECT * FROM {table}',
  IF(@symb_ord IS NULL, '', CONCAT(' ORDER BY ', @symb_ord)), ' LIMIT {limit} OFFSET {offset}) x');
PREPARE symb_s FROM @symb_q; EXECUTE symb_s; DEALLOCATE PREPARE symb_s;
"#
    )
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawTable {
    schema: Option<String>,
    name: String,
    #[serde(default)]
    kind: Option<String>,
    rows: Option<i64>,
    #[serde(default, deserialize_with = "lenient_bool")]
    is_default: bool,
}

#[tauri::command]
pub async fn db_list_tables(
    distro: String,
    container_id: String,
    docker: State<'_, DockerManager>,
) -> Result<Vec<DbTableSummary>, String> {
    let t = target(&docker, &distro, &container_id).await?;
    list_tables(&t).await
}

async fn list_tables(t: &Target) -> Result<Vec<DbTableSummary>, String> {
    let summary = |r: RawTable, default_schema: bool| DbTableSummary {
        id: match (&r.schema, default_schema) {
            (Some(s), false) => format!("{s}.{}", r.name),
            _ => r.name.clone(),
        },
        name: r.name,
        schema: r.schema,
        kind: r.kind.unwrap_or_else(|| "table".into()),
        rows: r.rows,
        ttl: None,
    };
    match t.kind {
        DbKind::Postgres => {
            let raw: Vec<RawTable> = parse_json(&t.json_query(PG_LIST_TABLES).await?)?;
            Ok(raw
                .into_iter()
                .map(|r| {
                    let public = r.schema.as_deref() == Some("public");
                    summary(r, public)
                })
                .collect())
        }
        DbKind::Mysql | DbKind::Mariadb => {
            let out = t.json_query(MY_LIST_TABLES).await?;
            let mut lines = out.lines().filter(|l| !l.trim().is_empty());
            let tables: Vec<RawTable> = parse_json(lines.next().unwrap_or("[]"))?;
            let counts: Vec<RawTable> = parse_json(lines.next().unwrap_or("[]"))?;
            Ok(tables
                .into_iter()
                .map(|mut r| {
                    if let Some(c) = counts.iter().find(|c| c.name == r.name && c.schema == r.schema) {
                        r.rows = c.rows;
                    }
                    let default = r.is_default;
                    summary(r, default)
                })
                .collect())
        }
        DbKind::Redis => {
            let limit = MAX_KEYS.to_string();
            let out = t.query(&["--raw", "EVAL", REDIS_KEYS_LUA, "0", &limit], "", QUERY_TIMEOUT).await?;
            let v: Value = parse_json(&out)?;
            Ok(json_array(&v["keys"])
                .iter()
                .filter_map(|k| {
                    let k = k.as_array()?;
                    let name = k.first()?.as_str()?.to_string();
                    Some(DbTableSummary {
                        id: name.clone(),
                        name,
                        schema: None,
                        kind: k.get(1).and_then(Value::as_str).unwrap_or("string").to_string(),
                        rows: None,
                        ttl: k.get(2).and_then(Value::as_i64),
                    })
                })
                .collect())
        }
        DbKind::Mongo => Err(MONGO_UNSUPPORTED.into()),
    }
}

#[tauri::command]
pub async fn db_query_table(
    distro: String,
    container_id: String,
    table_name: String,
    limit: u32,
    offset: u32,
    docker: State<'_, DockerManager>,
) -> Result<DbTableData, String> {
    let t = target(&docker, &distro, &container_id).await?;
    query_table(&t, &table_name, limit, offset).await
}

async fn query_table(t: &Target, table_name: &str, limit: u32, offset: u32) -> Result<DbTableData, String> {
    let limit = limit.clamp(1, MAX_PAGE);
    if t.kind == DbKind::Redis {
        validate_redis_key(table_name)?;
        let (off, lim) = (offset.to_string(), limit.to_string());
        let out = t.query(&["--raw", "EVAL", REDIS_VALUE_LUA, "1", table_name, &off, &lim], "", QUERY_TIMEOUT).await?;
        let v: Value = parse_json(&out)?;
        let names: Vec<String> = json_array(&v["columns"]).iter().filter_map(|c| c.as_str().map(String::from)).collect();
        let rows = json_array(&v["rows"])
            .iter()
            .map(|r| {
                let cells = json_array(r);
                Value::Object(names.iter().cloned().zip(cells.into_iter()).collect())
            })
            .collect();
        let columns = names
            .iter()
            .map(|n| DbColumn {
                name: n.clone(),
                data_type: if n == "score" { "number".into() } else { "string".into() },
                is_nullable: false,
                default_value: None,
                is_primary_key: false,
                is_identity: false,
                pos: 0,
            })
            .collect();
        return Ok(DbTableData {
            columns,
            rows,
            total: v["total"].as_i64(),
            limit,
            offset,
            kind: v["type"].as_str().map(String::from),
            ttl: v["ttl"].as_i64(),
        });
    }

    let table = parse_table_ref(table_name)?;
    let (columns_json, count, rows_json) = match t.dialect()? {
        Dialect::Postgres => {
            let mark = token();
            let out = t.json_query(&pg_table_script(&table, limit, offset, &mark)).await?;
            let blocks = split_blocks(&out, &mark);
            if blocks.len() != 3 {
                return Err(format!("Resposta inesperada do banco:\n{}", out.chars().take(300).collect::<String>()));
            }
            (blocks[0].clone(), blocks[1].clone(), blocks[2].clone())
        }
        Dialect::MySql => {
            let out = t.json_query(&my_table_script(&table, limit, offset)).await?;
            let lines: Vec<&str> = out.lines().filter(|l| !l.trim().is_empty()).collect();
            if lines.len() != 3 {
                return Err(format!("Resposta inesperada do banco:\n{}", out.chars().take(300).collect::<String>()));
            }
            (lines[0].to_string(), lines[1].to_string(), lines[2].to_string())
        }
    };
    let mut columns: Vec<DbColumn> = parse_json(&columns_json)?;
    if columns.is_empty() {
        return Err("Essa tabela não existe (talvez tenha sido apagada). Atualize a lista.".into());
    }
    columns.sort_by_key(|c| c.pos);
    let rows: Vec<Value> = parse_json(&rows_json)?;
    Ok(DbTableData {
        columns,
        rows,
        total: count.trim().parse().ok(),
        limit,
        offset,
        kind: None,
        ttl: None,
    })
}

#[tauri::command]
pub async fn db_create_table(
    distro: String,
    container_id: String,
    table_name: String,
    columns: Vec<NewColumnDef>,
    docker: State<'_, DockerManager>,
) -> Result<(), String> {
    let t = target(&docker, &distro, &container_id).await?;
    create_table(&t, &table_name, &columns).await
}

async fn create_table(t: &Target, table_name: &str, columns: &[NewColumnDef]) -> Result<(), String> {
    let table = parse_table_ref(table_name)?;
    let sql = create_table_sql(t.dialect()?, &table, columns)?;
    t.query(&[], &sql, QUERY_TIMEOUT).await.map(drop)
}

#[tauri::command]
pub async fn db_insert_row(
    distro: String,
    container_id: String,
    table_name: String,
    values: Map<String, Value>,
    docker: State<'_, DockerManager>,
) -> Result<(), String> {
    let t = target(&docker, &distro, &container_id).await?;
    insert_row(&t, &table_name, &values).await
}

async fn insert_row(t: &Target, table_name: &str, values: &Map<String, Value>) -> Result<(), String> {
    if t.kind == DbKind::Redis {
        // A new string key: {"key", "value", "ttl"?}.
        let key = values.get("key").and_then(Value::as_str).ok_or("Informe o nome da chave.")?;
        validate_redis_key(key)?;
        let value = match values.get("value") {
            Some(Value::String(s)) => s.clone(),
            Some(Value::Null) | None => String::new(),
            Some(other) => other.to_string(),
        };
        let ttl = values.get("ttl").and_then(Value::as_i64).filter(|t| *t > 0).map(|t| t.to_string());
        let mut args = vec!["-x", "SET", key];
        if let Some(ttl) = &ttl {
            args.extend(["EX", ttl.as_str()]);
        }
        // -x: the value comes from stdin (any size, any characters).
        let out = t.query(&args, &value, QUERY_TIMEOUT).await?;
        return if out.trim() == "OK" { Ok(()) } else { Err(explain_db_error(&out)) };
    }
    let table = parse_table_ref(table_name)?;
    let sql = insert_sql(t.dialect()?, &table, values)?;
    let out = t.json_query(&sql).await?;
    match out.lines().rev().find_map(|l| l.trim().parse::<i64>().ok()) {
        Some(n) if n > 0 => Ok(()),
        _ => Err("Nenhuma linha foi inserida.".into()),
    }
}

#[tauri::command]
pub async fn db_delete_row(
    distro: String,
    container_id: String,
    table_name: String,
    pk_column: String,
    pk_value: String,
    docker: State<'_, DockerManager>,
) -> Result<(), String> {
    let t = target(&docker, &distro, &container_id).await?;
    delete_row(&t, &table_name, &pk_column, &pk_value).await
}

async fn delete_row(t: &Target, table_name: &str, pk_column: &str, pk_value: &str) -> Result<(), String> {
    let deleted = if t.kind == DbKind::Redis {
        validate_redis_key(table_name)?;
        let out = t.query(&["DEL", table_name], "", QUERY_TIMEOUT).await?;
        out.trim().parse::<i64>().unwrap_or(0)
    } else {
        let table = parse_table_ref(table_name)?;
        let sql = delete_sql(t.dialect()?, &table, pk_column, pk_value)?;
        let out = t.json_query(&sql).await?;
        out.lines().rev().find_map(|l| l.trim().parse::<i64>().ok()).unwrap_or(0)
    };
    if deleted > 0 { Ok(()) } else { Err("A linha não foi encontrada (talvez já tenha sido apagada). Atualize a tabela.".into()) }
}

#[tauri::command]
pub async fn db_execute_sql(
    distro: String,
    container_id: String,
    sql: String,
    docker: State<'_, DockerManager>,
) -> Result<DbQueryResult, String> {
    let t = target(&docker, &distro, &container_id).await?;
    execute_sql(&t, &sql).await
}

async fn execute_sql(t: &Target, sql: &str) -> Result<DbQueryResult, String> {
    if sql.len() > 4 * 1024 * 1024 {
        return Err("SQL grande demais para o editor (máximo 4 MB).".into());
    }
    if t.kind == DbKind::Redis {
        // One command per line, as in redis-cli.
        let commands: String = sql.lines().map(str::trim).filter(|l| !l.is_empty()).map(|l| format!("{l}\n")).collect();
        if commands.is_empty() {
            return Err("Digite um comando (ex.: KEYS *).".into());
        }
        let out = t.query(&[], &commands, SQL_TIMEOUT).await?;
        let lines: Vec<Vec<Option<String>>> = out.lines().map(|l| vec![Some(l.to_string())]).collect();
        return Ok(DbQueryResult {
            columns: vec!["resposta".into()],
            truncated: lines.len() > MAX_RESULT_ROWS,
            rows_affected: None,
            statements: commands.lines().count(),
            rows: lines.into_iter().take(MAX_RESULT_ROWS).collect(),
        });
    }
    let dialect = t.dialect()?;
    let statements = split_statements(sql, dialect);
    if statements.is_empty() {
        return Err("Digite um comando SQL.".into());
    }
    let mark = token();
    let (args, script, null): (&[&str], String, String) = match dialect {
        Dialect::Postgres => {
            let null = format!("NULL{mark}");
            (&["--csv"], pg_editor_script(&statements, &mark, &null), null)
        }
        Dialect::MySql => (&["-B"], my_editor_script(&statements, &mark), String::new()),
    };
    let out = t.exec(args, script.as_bytes(), SQL_TIMEOUT).await?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    let result = match dialect {
        Dialect::Postgres => parse_pg_editor_output(&stdout, &mark, &null),
        Dialect::MySql => parse_my_editor_output(&stdout, &mark),
    };
    if out.success() {
        return Ok(DbQueryResult { statements: statements.len(), ..result });
    }
    let mut message = explain_db_error(&out.error_text());
    if result.statements > 0 {
        message.push_str(&format!(
            "\n\n{} comando(s) antes deste já foram aplicados (o editor não desfaz o que já rodou).",
            result.statements
        ));
    }
    Err(message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn col(name: &str, ty: ColumnType) -> NewColumnDef {
        NewColumnDef { name: name.into(), column_type: ty, nullable: true, primary_key: false, unique: false, default: None }
    }

    #[test]
    fn validates_identifiers_against_injection() {
        for ok in ["clientes", "Clientes", "_prisma_migrations", "pedidos-2024", "usuários", "Order Details"] {
            assert!(validate_identifier(ok).is_ok(), "{ok}");
        }
        for bad in [
            "",
            "x\"; DROP TABLE y; --",
            "a`b",
            "a'b",
            "a.b",
            "a;b",
            "a\\b",
            " lead",
            "tab\tname",
            "new\nline",
            "a/*b*/",
            &"x".repeat(64),
        ] {
            assert!(validate_identifier(bad).is_err(), "{bad:?}");
        }
        assert!(validate_new_identifier("created_at").is_ok());
        assert!(validate_new_identifier("1abc").is_err());
        assert!(validate_new_identifier("usuários").is_err());
        assert!(validate_new_identifier("drop table").is_err());
    }

    #[test]
    fn parses_table_refs() {
        assert_eq!(parse_table_ref("clientes").unwrap(), TableRef { schema: None, name: "clientes".into() });
        let r = parse_table_ref("auth.users").unwrap();
        assert_eq!(r.pg(), r#""auth"."users""#);
        assert_eq!(r.my(), "`auth`.`users`");
        assert_eq!(parse_table_ref("clientes").unwrap().pg(), r#""public"."clientes""#);
        assert!(parse_table_ref("a.b.c").is_err());
        assert!(parse_table_ref("x\"").is_err());
    }

    #[test]
    fn quotes_literals_safely() {
        assert_eq!(pg_literal("abc"), "$symb$abc$symb$");
        assert_eq!(pg_literal("tem $symb$ dentro"), "$symb1$tem $symb$ dentro$symb1$");
        assert_eq!(my_literal("a'b"), "CONVERT(X'612762' USING utf8mb4)");
        assert_eq!(my_literal(""), "''");
        assert_eq!(pg_ident(r#"a"b"#), r#""a""b""#);
        assert_eq!(my_ident("a`b"), "`a``b`");
    }

    #[test]
    fn generates_insert_without_values_in_sql_text() {
        let t = parse_table_ref("clientes").unwrap();
        let mut v = Map::new();
        v.insert("nome".into(), json!("Robert'); DROP TABLE clientes;--"));
        v.insert("ativo".into(), json!(true));
        v.insert("idade".into(), json!(42));
        v.insert("extra".into(), json!({"a": 1}));
        let pg = insert_sql(Dialect::Postgres, &t, &v).unwrap();
        assert!(pg.contains(r#"INSERT INTO "public"."clientes" ("nome", "ativo", "idade", "extra") SELECT"#), "{pg}");
        assert!(pg.contains(r#"json_populate_record(NULL::"public"."clientes", $symb$"#));
        // The value only lives inside the dollar-quoted JSON document.
        assert_eq!(pg.matches("DROP TABLE").count(), 1);
        assert!(pg.contains("$symb$::json"));

        let my = insert_sql(Dialect::MySql, &t, &v).unwrap();
        assert!(my.starts_with("INSERT INTO `clientes` (`nome`, `ativo`, `idade`, `extra`) VALUES (CONVERT(X'"), "{my}");
        assert!(!my.contains("DROP"));
        assert!(my.contains(", TRUE, 42, CONVERT(X'"));

        let empty = insert_sql(Dialect::Postgres, &t, &Map::new()).unwrap();
        assert!(empty.contains("DEFAULT VALUES"));

        let mut bad = Map::new();
        bad.insert("x\" = 1; --".into(), json!(1));
        assert!(insert_sql(Dialect::Postgres, &t, &bad).is_err());
    }

    #[test]
    fn generates_delete_by_primary_key() {
        let t = parse_table_ref("clientes").unwrap();
        let pg = delete_sql(Dialect::Postgres, &t, "id", "1 OR 1=1").unwrap();
        assert_eq!(
            pg,
            r#"WITH symb_del AS (DELETE FROM "public"."clientes" WHERE "id" = $symb$1 OR 1=1$symb$ RETURNING 1) SELECT count(*) FROM symb_del;"#
        );
        let my = delete_sql(Dialect::MySql, &t, "id", "7").unwrap();
        assert!(my.starts_with("DELETE FROM `clientes` WHERE `id` = CONVERT(X'37' USING utf8mb4) LIMIT 1;"));
        assert!(delete_sql(Dialect::Postgres, &t, "id\"--", "1").is_err());
    }

    #[test]
    fn generates_create_table() {
        let t = parse_table_ref("clientes").unwrap();
        let mut id = col("id", ColumnType::Identity);
        id.primary_key = true;
        let mut nome = col("nome", ColumnType::Text);
        nome.nullable = false;
        let mut email = col("email", ColumnType::Varchar);
        email.unique = true;
        let mut ativo = col("ativo", ColumnType::Bool);
        ativo.default = Some(ColumnDefault::True);
        let mut criado = col("created_at", ColumnType::Timestamp);
        criado.default = Some(ColumnDefault::Now);
        let mut obs = col("obs", ColumnType::Text);
        obs.default = Some(ColumnDefault::Value("it's".into()));
        let cols = vec![id, nome, email, ativo, criado, obs];

        let pg = create_table_sql(Dialect::Postgres, &t, &cols).unwrap();
        assert_eq!(
            pg,
            "CREATE TABLE \"public\".\"clientes\" (\n  \"id\" bigint GENERATED BY DEFAULT AS IDENTITY PRIMARY KEY,\n  \"nome\" text NOT NULL,\n  \"email\" varchar(255) UNIQUE,\n  \"ativo\" boolean DEFAULT TRUE,\n  \"created_at\" timestamptz DEFAULT now(),\n  \"obs\" text DEFAULT $symb$it's$symb$\n);"
        );
        let my = create_table_sql(Dialect::MySql, &t, &cols).unwrap();
        assert!(my.contains("`id` BIGINT AUTO_INCREMENT PRIMARY KEY"), "{my}");
        assert!(my.contains("`created_at` DATETIME(3) DEFAULT CURRENT_TIMESTAMP(3)"));
        assert!(my.contains("`obs` TEXT DEFAULT (CONVERT(X'"));

        let mut uuid = col("id", ColumnType::Uuid);
        uuid.primary_key = true;
        uuid.default = Some(ColumnDefault::RandomUuid);
        assert!(create_table_sql(Dialect::Postgres, &t, &[uuid]).unwrap().contains("\"id\" uuid DEFAULT gen_random_uuid() PRIMARY KEY"));

        // Composite key.
        let mut a = col("a", ColumnType::Int);
        a.primary_key = true;
        let mut b = col("b", ColumnType::Int);
        b.primary_key = true;
        assert!(create_table_sql(Dialect::Postgres, &t, &[a, b]).unwrap().contains("PRIMARY KEY (\"a\", \"b\")"));

        // Refusals.
        let bad_name = parse_table_ref("Order Details").unwrap();
        assert!(create_table_sql(Dialect::Postgres, &bad_name, &cols).is_err());
        assert!(create_table_sql(Dialect::Postgres, &t, &[]).is_err());
        assert!(create_table_sql(Dialect::Postgres, &t, &[col("id", ColumnType::Identity)]).is_err());
        assert!(create_table_sql(Dialect::Postgres, &t, &[col("x", ColumnType::Text), col("X", ColumnType::Int)]).is_err());
        assert!(create_table_sql(Dialect::Postgres, &t, &[col("x\"); DROP", ColumnType::Text)]).is_err());
        let mut wrong_default = col("n", ColumnType::Int);
        wrong_default.default = Some(ColumnDefault::Now);
        assert!(create_table_sql(Dialect::Postgres, &t, &[wrong_default]).is_err());
        let mut text_pk = col("k", ColumnType::Text);
        text_pk.primary_key = true;
        assert!(create_table_sql(Dialect::MySql, &t, &[text_pk.clone()]).is_err());
        assert!(create_table_sql(Dialect::Postgres, &t, &[text_pk]).is_ok());
    }

    #[test]
    fn splits_sql_statements() {
        let pg = "SELECT 1; SELECT 'a;b'; -- c;\nSELECT $$x;y$$; SELECT $f$ ; $f$; /* a; /* b; */ c; */ SELECT \"q;\"; SELECT E'it\\';s'";
        assert_eq!(
            split_statements(pg, Dialect::Postgres),
            vec!["SELECT 1", "SELECT 'a;b'", "-- c;\nSELECT $$x;y$$", "SELECT $f$ ; $f$", "/* a; /* b; */ c; */ SELECT \"q;\"", "SELECT E'it\\';s'"]
        );
        assert_eq!(split_statements("SELECT $1; SELECT 2", Dialect::Postgres), vec!["SELECT $1", "SELECT 2"]);
        assert_eq!(split_statements("  ;; -- só comentário\n", Dialect::Postgres), Vec::<String>::new());
        assert_eq!(
            split_statements("SELECT 'a\\';b'; # x;\nSELECT `a;b`", Dialect::MySql),
            vec!["SELECT 'a\\';b'", "# x;\nSELECT `a;b`"]
        );
        // Postgres: no backslash escapes in normal strings.
        assert_eq!(split_statements("SELECT 'a\\'; SELECT 2", Dialect::Postgres), vec!["SELECT 'a\\'", "SELECT 2"]);
        assert_eq!(split_statements("SELECT 'acentuação';", Dialect::Postgres), vec!["SELECT 'acentuação'"]);
    }

    #[test]
    fn parses_csv() {
        assert_eq!(
            parse_csv("id,nome\n1,\"Ana, \"\"A\"\"\"\n2,\"linha\nquebrada\"\n3,\n"),
            vec![vec!["id", "nome"], vec!["1", "Ana, \"A\""], vec!["2", "linha\nquebrada"], vec!["3", ""]]
        );
        assert!(parse_csv("").is_empty());
    }

    #[test]
    fn parses_psql_editor_output() {
        let mark = "SYMBx";
        let null = "NULLSYMBx";
        let script = pg_editor_script(&["SELECT 1".into(), "UPDATE t SET a = 1 -- fim".into()], mark, null);
        assert!(script.contains("UPDATE t SET a = 1 -- fim\n;\n\\echo SYMBx :ROW_COUNT"));
        let out = "id,nome\n1,Ana\n2,NULLSYMBx\nSYMBx 2\nSYMBx 5\n";
        let r = parse_pg_editor_output(out, mark, null);
        assert_eq!(r.statements, 2);
        assert_eq!(r.columns, vec!["id", "nome"]);
        assert_eq!(r.rows, vec![vec![Some("1".into()), Some("Ana".into())], vec![Some("2".into()), None]]);
        assert_eq!(r.rows_affected, Some(5));
    }

    #[test]
    fn parses_mysql_editor_output() {
        let mark = "SYMBy";
        let out = "id\tnome\n1\tA\\tB\n2\tNULL\nSYMBy\n-1\nSYMBy\n3\n";
        let r = parse_my_editor_output(out, mark);
        assert_eq!(r.statements, 2);
        assert_eq!(r.columns, vec!["id", "nome"]);
        assert_eq!(r.rows[0], vec![Some("1".into()), Some("A\tB".into())]);
        assert_eq!(r.rows[1][1], None);
        assert_eq!(r.rows_affected, Some(3));
    }

    #[test]
    fn explains_database_errors() {
        let e = explain_db_error("psql:<stdin>:3: ERROR:  relation \"x\" does not exist\nLINE 1: SELECT * FROM x");
        assert!(e.starts_with("Essa tabela ou coluna não existe"), "{e}");
        assert!(e.contains("\nERROR:  relation \"x\" does not exist"));
        let m = explain_db_error("mysql: [Warning] Using a password on the command line interface can be insecure.\nERROR 1062 (23000) at line 1: Duplicate entry '1' for key 'PRIMARY'");
        assert!(m.starts_with("Já existe uma linha"), "{m}");
        assert!(m.contains("ERROR 1062 (23000): Duplicate entry"));
        assert!(!m.contains("[Warning]"));
    }

    #[test]
    fn redis_scripts_are_single_line() {
        assert!(!REDIS_KEYS_LUA.contains('\n') && !REDIS_VALUE_LUA.contains('\n'));
        assert!(!REDIS_KEYS_LUA.contains("--") && !REDIS_VALUE_LUA.contains("--"));
        assert!(validate_redis_key("user:1").is_ok());
        assert!(validate_redis_key("a\nb").is_err());
    }

    #[test]
    fn reads_mysql_flags_as_booleans() {
        let c: DbColumn = serde_json::from_str(
            r#"{"name":"id","dataType":"bigint","isNullable":0,"defaultValue":null,"isPrimaryKey":1,"isIdentity":1,"pos":1}"#,
        )
        .unwrap();
        assert!(c.is_primary_key && c.is_identity && !c.is_nullable);
    }
}

/// Real end-to-end run against a throwaway Postgres in Docker (starts Docker Desktop if needed).
/// `WSL_TEST_DISTRO=Ubuntu cargo test --lib live_docker -- --ignored --nocapture`
#[cfg(test)]
mod live {
    use super::*;
    use serde_json::json;

    struct Cleanup {
        engine: Engine,
        name: String,
    }

    impl Drop for Cleanup {
        fn drop(&mut self) {
            let engine = self.engine.clone();
            let name = self.name.clone();
            // Runs even when an assertion failed: the machine stays clean.
            std::thread::spawn(move || {
                let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
                rt.block_on(async {
                    let _ = docker::remove(&engine, &name, true).await;
                    let _ = engine.text(&["volume", "rm", &docker::volume_name(&name)], Duration::from_secs(30)).await;
                });
            })
            .join()
            .ok();
        }
    }

    async fn engine() -> Engine {
        let distro = std::env::var("WSL_TEST_DISTRO").unwrap_or_else(|_| "Ubuntu".into());
        let manager = DockerManager::default();
        let status = docker::start_engine(&manager, &distro).await.expect("Docker indisponível");
        eprintln!("docker: {:?} {:?}", status.source, status.version);
        manager.engine(&distro).await.unwrap()
    }

    fn free_high_port(from: u16) -> u16 {
        (from..from + 200).find(|p| docker::port_is_free(*p)).expect("sem porta livre")
    }

    async fn target_for(engine: &Engine, name: &str) -> Target {
        let info = docker::inspect(engine, &[name]).await.unwrap().remove(0);
        Target { engine: engine.clone(), id: info.id, kind: info.kind.unwrap(), redis_auth: None }
    }

    #[tokio::test(flavor = "multi_thread")]
    #[ignore]
    async fn live_docker_postgres_table_manager() {
        let engine = engine().await;
        let name = "symbiosis-test-pg".to_string();
        let _ = docker::remove(&engine, &name, true).await;
        let _cleanup = Cleanup { engine: engine.clone(), name: name.clone() };

        let port = free_high_port(55432);
        let request = docker::NewDatabase {
            kind: DbKind::Postgres,
            name: name.clone(),
            database: Some("app_db".into()),
            user: Some("postgres".into()),
            password: Some("p@ss:w/rd".into()),
            port,
        };
        let created = docker::create(&engine, &request, &|stage, detail, pct| eprintln!("  [{stage}] {:?} {}", pct, detail.unwrap_or_default()))
            .await
            .expect("create");
        assert!(created.ready, "pg_isready não respondeu");
        let info = created.database.unwrap();
        assert_eq!(info.connection_url.as_deref(), Some(format!("postgresql://postgres:p%40ss%3Aw%2Frd@localhost:{port}/app_db").as_str()));

        // The password never reached `docker run`'s argv: it is only in the container's env.
        let listed = docker::list(&engine).await.unwrap();
        let card = listed.iter().find(|c| c.name == name).unwrap();
        assert!(card.managed && card.state == "running");
        assert!(card.ports.iter().any(|p| p.host_port == port && p.host_ip == "127.0.0.1"));

        let t = target_for(&engine, &name).await;
        let mut id = NewColumnDef { name: "id".into(), column_type: ColumnType::Identity, nullable: false, primary_key: true, unique: false, default: None };
        id.primary_key = true;
        let columns = vec![
            id,
            NewColumnDef { name: "nome".into(), column_type: ColumnType::Text, nullable: false, primary_key: false, unique: false, default: None },
            NewColumnDef { name: "email".into(), column_type: ColumnType::Varchar, nullable: true, primary_key: false, unique: true, default: None },
            NewColumnDef { name: "ativo".into(), column_type: ColumnType::Bool, nullable: false, primary_key: false, unique: false, default: Some(ColumnDefault::True) },
        ];
        create_table(&t, "clientes", &columns).await.expect("create table");

        let tricky = "O'Brien \"$symb$\" :var \\! echo pwned; -- ação 🚀";
        let rows = [json!({"nome": "Ana", "email": "ana@ex.com"}), json!({"nome": tricky, "email": "ob@ex.com", "ativo": false})];
        for r in &rows {
            insert_row(&t, "clientes", r.as_object().unwrap()).await.expect("insert");
        }

        let tables = list_tables(&t).await.unwrap();
        let clientes = tables.iter().find(|x| x.id == "clientes").expect("tabela listada");
        assert_eq!(clientes.rows, Some(2));

        let data = query_table(&t, "clientes", 50, 0).await.unwrap();
        assert_eq!(data.total, Some(2));
        assert_eq!(data.columns.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), vec!["id", "nome", "email", "ativo"]);
        assert!(data.columns[0].is_primary_key && data.columns[0].is_identity);
        assert_eq!(data.columns[3].data_type, "boolean");
        assert_eq!(data.rows[0], json!({"id": 1, "nome": "Ana", "email": "ana@ex.com", "ativo": true}));
        assert_eq!(data.rows[1]["nome"], json!(tricky));
        assert_eq!(data.rows[1]["ativo"], json!(false));

        let result = execute_sql(&t, "SELECT nome, email FROM clientes WHERE ativo ORDER BY id; -- fim").await.unwrap();
        assert_eq!(result.columns, vec!["nome", "email"]);
        assert_eq!(result.rows, vec![vec![Some("Ana".into()), Some("ana@ex.com".into())]]);
        let nulls = execute_sql(&t, "SELECT NULL AS n, '' AS vazio").await.unwrap();
        assert_eq!(nulls.rows, vec![vec![None, Some(String::new())]]);
        let update = execute_sql(&t, "UPDATE clientes SET ativo = true").await.unwrap();
        assert_eq!((update.columns.len(), update.rows_affected), (0, Some(2)));
        let err = execute_sql(&t, "SELECT 1; SELECT * FROM nao_existe;").await.unwrap_err();
        assert!(err.starts_with("Essa tabela ou coluna não existe") && err.contains("1 comando(s) antes"), "{err}");

        delete_row(&t, "clientes", "id", "1").await.expect("delete");
        assert!(delete_row(&t, "clientes", "id", "1").await.is_err());
        let data = query_table(&t, "clientes", 50, 0).await.unwrap();
        assert_eq!(data.total, Some(1));
        assert_eq!(data.rows[0]["nome"], json!(tricky));

        // Identifier injection never reaches the database.
        assert!(query_table(&t, "clientes\"; DROP TABLE clientes; --", 10, 0).await.is_err());
        assert_eq!(list_tables(&t).await.unwrap().iter().filter(|x| x.id == "clientes").count(), 1);

        // Lifecycle: stop → explorer refuses → start → data survived (volume).
        docker::lifecycle_engine(&engine, "stop", &name).await.unwrap();
        let stopped = docker::inspect(&engine, &[&name]).await.unwrap().remove(0);
        assert!(!stopped.running);
        docker::lifecycle_engine(&engine, "start", &name).await.unwrap();
        assert!(docker::wait_ready(&engine, &name, DbKind::Postgres, Duration::from_secs(60)).await);
        assert_eq!(query_table(&t, "clientes", 50, 0).await.unwrap().total, Some(1));

        let removed = docker::remove(&engine, &name, true).await.unwrap();
        assert_eq!(removed.removed_volumes, vec![docker::volume_name(&name)]);
        assert!(engine.text(&["volume", "inspect", &docker::volume_name(&name)], Duration::from_secs(20)).await.is_err());
    }

    #[tokio::test(flavor = "multi_thread")]
    #[ignore]
    async fn live_docker_mysql_and_redis() {
        let engine = engine().await;

        // MySQL
        let name = "symbiosis-test-mysql".to_string();
        let _ = docker::remove(&engine, &name, true).await;
        let _cleanup_my = Cleanup { engine: engine.clone(), name: name.clone() };
        let request = docker::NewDatabase {
            kind: DbKind::Mysql,
            name: name.clone(),
            database: Some("loja".into()),
            user: Some("root".into()),
            password: Some("root".into()),
            port: free_high_port(53306),
        };
        let created = docker::create(&engine, &request, &|stage, _, pct| eprintln!("  [mysql {stage}] {pct:?}")).await.expect("create mysql");
        assert!(created.ready, "mysql não respondeu");
        let t = target_for(&engine, &name).await;
        let mut id = NewColumnDef { name: "id".into(), column_type: ColumnType::Identity, nullable: false, primary_key: true, unique: false, default: None };
        id.primary_key = true;
        let columns = vec![
            id,
            NewColumnDef { name: "nome".into(), column_type: ColumnType::Varchar, nullable: false, primary_key: false, unique: false, default: None },
            NewColumnDef { name: "preco".into(), column_type: ColumnType::Numeric, nullable: true, primary_key: false, unique: false, default: None },
            NewColumnDef { name: "criado".into(), column_type: ColumnType::Timestamp, nullable: true, primary_key: false, unique: false, default: Some(ColumnDefault::Now) },
        ];
        create_table(&t, "produtos", &columns).await.expect("create table mysql");
        let tricky = "Café \\ 'aspas' \"duplas\" `crase` ; DROP TABLE produtos; --";
        insert_row(&t, "produtos", json!({"nome": tricky, "preco": 9.9}).as_object().unwrap()).await.expect("insert mysql");
        insert_row(&t, "produtos", json!({"nome": "Pão", "preco": null}).as_object().unwrap()).await.expect("insert mysql 2");
        let tables = list_tables(&t).await.unwrap();
        assert_eq!(tables.iter().find(|x| x.id == "produtos").and_then(|x| x.rows), Some(2));
        let data = query_table(&t, "produtos", 50, 0).await.unwrap();
        assert_eq!(data.total, Some(2));
        assert_eq!(data.columns.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), vec!["id", "nome", "preco", "criado"]);
        assert!(data.columns[0].is_primary_key && data.columns[0].is_identity);
        assert_eq!(data.rows[0]["nome"], json!(tricky));
        assert_eq!(data.rows[1]["preco"], Value::Null);
        let r = execute_sql(&t, "SELECT nome FROM produtos ORDER BY id; UPDATE produtos SET preco = 1;").await.unwrap();
        assert_eq!(r.rows_affected, Some(2));
        let r = execute_sql(&t, "SELECT id, nome, preco FROM produtos WHERE preco IS NOT NULL ORDER BY id").await.unwrap();
        assert_eq!(r.columns, vec!["id", "nome", "preco"]);
        assert_eq!(r.rows[0][1].as_deref(), Some(tricky));
        delete_row(&t, "produtos", "id", "2").await.expect("delete mysql");
        assert_eq!(query_table(&t, "produtos", 50, 0).await.unwrap().total, Some(1));
        drop(_cleanup_my);

        // Redis (with password)
        let name = "symbiosis-test-redis".to_string();
        let _ = docker::remove(&engine, &name, true).await;
        let _cleanup_redis = Cleanup { engine: engine.clone(), name: name.clone() };
        let request = docker::NewDatabase {
            kind: DbKind::Redis,
            name: name.clone(),
            database: None,
            user: None,
            password: Some("s3nha".into()),
            port: free_high_port(56379),
        };
        let created = docker::create(&engine, &request, &|stage, _, pct| eprintln!("  [redis {stage}] {pct:?}")).await.expect("create redis");
        assert!(created.ready);
        assert_eq!(created.database.unwrap().password.as_deref(), Some("s3nha"));
        let t = target_for(&engine, &name).await;
        insert_row(&t, "", json!({"key": "user:1", "value": "linha 1\nlinha \"2\""}).as_object().unwrap()).await.expect("set");
        execute_sql(&t, "HSET sessao:9 nome Ana papel admin\nRPUSH fila a b c").await.expect("redis cmds");
        let keys = list_tables(&t).await.unwrap();
        let mut names: Vec<_> = keys.iter().map(|k| (k.id.as_str(), k.kind.as_str())).collect();
        names.sort();
        assert_eq!(names, vec![("fila", "list"), ("sessao:9", "hash"), ("user:1", "string")]);
        let v = query_table(&t, "user:1", 50, 0).await.unwrap();
        assert_eq!(v.rows, vec![json!({"value": "linha 1\nlinha \"2\""})]);
        let h = query_table(&t, "sessao:9", 50, 0).await.unwrap();
        assert_eq!((h.kind.as_deref(), h.total, h.rows.len()), (Some("hash"), Some(2), 2));
        let l = query_table(&t, "fila", 2, 1).await.unwrap();
        assert_eq!(l.rows, vec![json!({"index": "1", "value": "b"}), json!({"index": "2", "value": "c"})]);
        delete_row(&t, "fila", "", "").await.expect("del");
        assert_eq!(list_tables(&t).await.unwrap().len(), 2);
    }
}
