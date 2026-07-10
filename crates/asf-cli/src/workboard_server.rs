//! `asf workboard-server` — a local MCP server for the second dogfooding
//! profile. Structured work lives in SQLite; supporting notes live in a
//! Markdown evidence tree. The broker snapshots and promotes both roots.

use crate::mcp;
use anyhow::{bail, Context, Result};
use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};
use serde_json::{json, Value};
use std::fs;
use std::io::{stdin, stdout, BufReader, Write};
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

const STATUSES: &[&str] = &["todo", "in_progress", "blocked", "done"];

fn now() -> Result<String> {
    Ok(time::OffsetDateTime::now_utc().format(&time::format_description::well_known::Rfc3339)?)
}

fn safe_join(root: &Path, rel: &str) -> Result<PathBuf> {
    let path = Path::new(rel);
    if rel.is_empty()
        || path.is_absolute()
        || rel
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
        || path
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        bail!("evidence path must be a non-empty relative path without '.' or '..': {rel}");
    }
    if path.extension().and_then(|extension| extension.to_str()) != Some("md") {
        bail!("evidence path must name a Markdown file: {rel}");
    }
    if fs::symlink_metadata(root)
        .map(|meta| meta.file_type().is_symlink())
        .unwrap_or(false)
    {
        bail!("evidence root must not be a symlink: {}", root.display());
    }
    let mut joined = root.to_path_buf();
    for part in path.components() {
        let Component::Normal(part) = part else {
            unreachable!("validated above")
        };
        joined.push(part);
        if fs::symlink_metadata(&joined)
            .map(|meta| meta.file_type().is_symlink())
            .unwrap_or(false)
        {
            bail!("evidence path traverses a symlink: {rel}");
        }
    }
    Ok(joined)
}

fn open(db: &Path) -> Result<Connection> {
    let conn = Connection::open(db)
        .with_context(|| format!("opening workboard database {}", db.display()))?;
    conn.busy_timeout(Duration::from_secs(5))?;
    conn.execute_batch("PRAGMA foreign_keys = ON;")?;
    Ok(conn)
}

/// Create the two roots and the stable v1 schema. Deliberately leave SQLite
/// in its default rollback-journal mode: the snapshotter treats the database
/// as one opaque store, so a long-lived WAL would be a second untracked file.
pub fn init(db: &Path, evidence: &Path) -> Result<()> {
    if let Some(parent) = db.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::create_dir_all(evidence)?;
    let conn = open(db)?;
    let journal_mode: String =
        conn.query_row("PRAGMA journal_mode = DELETE", [], |row| row.get(0))?;
    if journal_mode != "delete" {
        bail!("workboard database must use SQLite rollback-journal mode, got {journal_mode}");
    }
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS tasks (
             id          INTEGER PRIMARY KEY AUTOINCREMENT,
             title       TEXT NOT NULL CHECK (length(trim(title)) > 0),
             details     TEXT NOT NULL DEFAULT '',
             status      TEXT NOT NULL DEFAULT 'todo'
                         CHECK (status IN ('todo','in_progress','blocked','done')),
             priority    INTEGER NOT NULL DEFAULT 0 CHECK (priority BETWEEN 0 AND 4),
             revision    INTEGER NOT NULL DEFAULT 1 CHECK (revision > 0),
             created_at  TEXT NOT NULL,
             updated_at  TEXT NOT NULL
         );
         CREATE TABLE IF NOT EXISTS dependencies (
             task_id     INTEGER NOT NULL REFERENCES tasks(id) ON DELETE RESTRICT,
             depends_on  INTEGER NOT NULL REFERENCES tasks(id) ON DELETE RESTRICT,
             PRIMARY KEY (task_id, depends_on),
             CHECK (task_id <> depends_on)
         );
         CREATE TABLE IF NOT EXISTS evidence_links (
             task_id     INTEGER NOT NULL REFERENCES tasks(id) ON DELETE RESTRICT,
             path        TEXT NOT NULL,
             PRIMARY KEY (task_id, path)
         );
         CREATE TABLE IF NOT EXISTS activity (
             id          INTEGER PRIMARY KEY AUTOINCREMENT,
             task_id     INTEGER NOT NULL REFERENCES tasks(id) ON DELETE RESTRICT,
             at          TEXT NOT NULL,
             kind        TEXT NOT NULL,
             detail      TEXT NOT NULL
         );
         CREATE INDEX IF NOT EXISTS activity_task_at ON activity(task_id, id);",
    )?;
    Ok(())
}

fn required_str<'a>(args: &'a Value, field: &str) -> Result<&'a str> {
    args.get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("missing string argument {field}"))
}

fn required_i64(args: &Value, field: &str) -> Result<i64> {
    args.get(field)
        .and_then(Value::as_i64)
        .ok_or_else(|| anyhow::anyhow!("missing integer argument {field}"))
}

fn validate_status(status: &str) -> Result<()> {
    if !STATUSES.contains(&status) {
        bail!("status must be one of {}", STATUSES.join(", "));
    }
    Ok(())
}

fn validate_priority(priority: i64) -> Result<()> {
    if !(0..=4).contains(&priority) {
        bail!("priority must be between 0 and 4");
    }
    Ok(())
}

fn task_json(conn: &Connection, id: i64) -> Result<Value> {
    let task = conn
        .query_row(
            "SELECT id, title, details, status, priority, revision, created_at, updated_at
             FROM tasks WHERE id = ?1",
            [id],
            |row| {
                Ok(json!({
                    "id": row.get::<_, i64>(0)?,
                    "title": row.get::<_, String>(1)?,
                    "details": row.get::<_, String>(2)?,
                    "status": row.get::<_, String>(3)?,
                    "priority": row.get::<_, i64>(4)?,
                    "revision": row.get::<_, i64>(5)?,
                    "created_at": row.get::<_, String>(6)?,
                    "updated_at": row.get::<_, String>(7)?,
                }))
            },
        )
        .optional()?
        .with_context(|| format!("work item {id} does not exist"))?;

    let mut dependencies = Vec::new();
    let mut stmt =
        conn.prepare("SELECT depends_on FROM dependencies WHERE task_id = ?1 ORDER BY depends_on")?;
    for item in stmt.query_map([id], |row| row.get::<_, i64>(0))? {
        dependencies.push(json!(item?));
    }
    let mut evidence = Vec::new();
    let mut stmt =
        conn.prepare("SELECT path FROM evidence_links WHERE task_id = ?1 ORDER BY path")?;
    for item in stmt.query_map([id], |row| row.get::<_, String>(0))? {
        evidence.push(json!(item?));
    }
    let mut activity = Vec::new();
    let mut stmt =
        conn.prepare("SELECT at, kind, detail FROM activity WHERE task_id = ?1 ORDER BY id")?;
    for item in stmt.query_map([id], |row| {
        Ok(json!({
            "at": row.get::<_, String>(0)?,
            "kind": row.get::<_, String>(1)?,
            "detail": row.get::<_, String>(2)?,
        }))
    })? {
        activity.push(item?);
    }
    let mut task = task;
    task["dependencies"] = Value::Array(dependencies);
    task["evidence"] = Value::Array(evidence);
    task["activity"] = Value::Array(activity);
    Ok(task)
}

fn task_summary(conn: &Connection, id: i64) -> Result<Value> {
    Ok(conn.query_row(
        "SELECT id, title, status, priority, revision, updated_at FROM tasks WHERE id = ?1",
        [id],
        |row| {
            Ok(json!({
                "id": row.get::<_, i64>(0)?,
                "title": row.get::<_, String>(1)?,
                "status": row.get::<_, String>(2)?,
                "priority": row.get::<_, i64>(3)?,
                "revision": row.get::<_, i64>(4)?,
                "updated_at": row.get::<_, String>(5)?,
            }))
        },
    )?)
}

fn assert_revision(tx: &Transaction<'_>, id: i64, expected: i64) -> Result<()> {
    let actual: Option<i64> = tx
        .query_row("SELECT revision FROM tasks WHERE id = ?1", [id], |row| {
            row.get(0)
        })
        .optional()?;
    match actual {
        None => bail!("work item {id} does not exist"),
        Some(actual) if actual != expected => {
            bail!("stale revision for work item {id}: expected {expected}, current is {actual}")
        }
        Some(_) => Ok(()),
    }
}

fn record(tx: &Transaction<'_>, id: i64, kind: &str, detail: &str, at: &str) -> Result<()> {
    tx.execute(
        "INSERT INTO activity(task_id, at, kind, detail) VALUES (?1, ?2, ?3, ?4)",
        params![id, at, kind, detail],
    )?;
    Ok(())
}

fn assert_dependencies_done(tx: &Transaction<'_>, id: i64) -> Result<()> {
    let open: i64 = tx.query_row(
        "SELECT COUNT(*)
         FROM dependencies d JOIN tasks dependency ON dependency.id = d.depends_on
         WHERE d.task_id = ?1 AND dependency.status <> 'done'",
        [id],
        |row| row.get(0),
    )?;
    if open > 0 {
        bail!("work item {id} has {open} unfinished dependency/dependencies");
    }
    Ok(())
}

fn tool_defs() -> Value {
    let id = json!({"type":"integer", "minimum":1});
    let revision = json!({"type":"integer", "minimum":1});
    let path = json!({"type":"string", "description":"Relative to the evidence root"});
    json!([
        {"name":"work.list", "description":"List work items in priority order, optionally filtered by status",
         "inputSchema":{"type":"object","properties":{"status":{"type":"string","enum":STATUSES}}}},
        {"name":"work.get", "description":"Get one work item with dependencies and evidence links",
         "inputSchema":{"type":"object","properties":{"id":id},"required":["id"]}},
        {"name":"work.create", "description":"Create a work item",
         "inputSchema":{"type":"object","properties":{"title":{"type":"string"},"details":{"type":"string"},"priority":{"type":"integer","minimum":0,"maximum":4}},"required":["title"]}},
        {"name":"work.update", "description":"Update a work item using optimistic concurrency",
         "inputSchema":{"type":"object","properties":{"id":id,"expected_revision":revision,"title":{"type":"string"},"details":{"type":"string"},"status":{"type":"string","enum":STATUSES},"priority":{"type":"integer","minimum":0,"maximum":4}},"required":["id","expected_revision"]}},
        {"name":"work.add_dependency", "description":"Add an acyclic dependency and advance the item's revision",
         "inputSchema":{"type":"object","properties":{"id":id,"depends_on":id,"expected_revision":revision},"required":["id","depends_on","expected_revision"]}},
        {"name":"work.link_evidence", "description":"Link an existing Markdown evidence path to a work item and advance its revision",
         "inputSchema":{"type":"object","properties":{"id":id,"path":path,"expected_revision":revision},"required":["id","path","expected_revision"]}},
        {"name":"work.close", "description":"Mark a work item done using optimistic concurrency after all dependencies are done",
         "inputSchema":{"type":"object","properties":{"id":id,"expected_revision":revision},"required":["id","expected_revision"]}},
        {"name":"evidence.list", "description":"List Markdown evidence files recursively",
         "inputSchema":{"type":"object","properties":{}}},
        {"name":"evidence.read", "description":"Read a Markdown evidence file",
         "inputSchema":{"type":"object","properties":{"path":path},"required":["path"]}},
        {"name":"evidence.create", "description":"Create a new Markdown evidence file; existing files are never overwritten",
         "inputSchema":{"type":"object","properties":{"path":path,"content":{"type":"string"}},"required":["path","content"]}},
        {"name":"evidence.edit", "description":"Replace one exact occurrence in a Markdown evidence file",
         "inputSchema":{"type":"object","properties":{"path":path,"old_string":{"type":"string"},"new_string":{"type":"string"}},"required":["path","old_string","new_string"]}}
    ])
}

fn call(db: &Path, evidence_root: &Path, name: &str, args: &Value) -> Result<String> {
    let result = match name {
        "work.list" => {
            let status = args.get("status").and_then(Value::as_str);
            if let Some(status) = status {
                validate_status(status)?;
            }
            let conn = open(db)?;
            let mut values = Vec::new();
            if let Some(status) = status {
                let mut stmt = conn.prepare(
                    "SELECT id FROM tasks WHERE status = ?1 ORDER BY priority DESC, id ASC",
                )?;
                for id in stmt.query_map([status], |row| row.get::<_, i64>(0))? {
                    values.push(task_summary(&conn, id?)?);
                }
            } else {
                let mut stmt = conn.prepare(
                    "SELECT id FROM tasks ORDER BY CASE status WHEN 'in_progress' THEN 0 WHEN 'blocked' THEN 1 WHEN 'todo' THEN 2 ELSE 3 END, priority DESC, id ASC",
                )?;
                for id in stmt.query_map([], |row| row.get::<_, i64>(0))? {
                    values.push(task_summary(&conn, id?)?);
                }
            }
            Value::Array(values)
        }
        "work.get" => task_json(&open(db)?, required_i64(args, "id")?)?,
        "work.create" => {
            let title = required_str(args, "title")?.trim();
            if title.is_empty() {
                bail!("title must not be empty");
            }
            let details = args.get("details").and_then(Value::as_str).unwrap_or("");
            let priority = args.get("priority").and_then(Value::as_i64).unwrap_or(0);
            validate_priority(priority)?;
            let at = now()?;
            let mut conn = open(db)?;
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            tx.execute(
                "INSERT INTO tasks(title, details, priority, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?4)",
                params![title, details, priority, at],
            )?;
            let id = tx.last_insert_rowid();
            record(&tx, id, "created", title, &at)?;
            tx.commit()?;
            task_json(&conn, id)?
        }
        "work.update" => {
            let id = required_i64(args, "id")?;
            let expected = required_i64(args, "expected_revision")?;
            if !["title", "details", "status", "priority"]
                .iter()
                .any(|field| args.get(field).is_some())
            {
                bail!("work.update needs at least one changed field");
            }
            let mut conn = open(db)?;
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            assert_revision(&tx, id, expected)?;
            let current = task_json(&tx, id)?;
            let title = args
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or(current["title"].as_str().unwrap())
                .trim();
            if title.is_empty() {
                bail!("title must not be empty");
            }
            let details = args
                .get("details")
                .and_then(Value::as_str)
                .unwrap_or(current["details"].as_str().unwrap());
            let status = args
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or(current["status"].as_str().unwrap());
            validate_status(status)?;
            if status == "done" {
                assert_dependencies_done(&tx, id)?;
            }
            let priority = args
                .get("priority")
                .and_then(Value::as_i64)
                .unwrap_or(current["priority"].as_i64().unwrap());
            validate_priority(priority)?;
            let at = now()?;
            tx.execute(
                "UPDATE tasks SET title=?2, details=?3, status=?4, priority=?5, revision=revision+1, updated_at=?6 WHERE id=?1",
                params![id, title, details, status, priority, at],
            )?;
            record(&tx, id, "updated", "fields updated", &at)?;
            tx.commit()?;
            task_json(&conn, id)?
        }
        "work.add_dependency" => {
            let id = required_i64(args, "id")?;
            let depends_on = required_i64(args, "depends_on")?;
            let expected = required_i64(args, "expected_revision")?;
            if id == depends_on {
                bail!("a work item cannot depend on itself");
            }
            let mut conn = open(db)?;
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            assert_revision(&tx, id, expected)?;
            if !tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM tasks WHERE id=?1)",
                [depends_on],
                |row| row.get::<_, bool>(0),
            )? {
                bail!("dependency work item {depends_on} does not exist");
            }
            let cycles: bool = tx.query_row(
                "WITH RECURSIVE reachable(id) AS (
                     SELECT ?1 UNION SELECT d.depends_on FROM dependencies d JOIN reachable r ON d.task_id=r.id
                 ) SELECT EXISTS(SELECT 1 FROM reachable WHERE id=?2)",
                params![depends_on, id], |row| row.get(0),
            )?;
            if cycles {
                bail!("dependency would create a cycle");
            }
            tx.execute(
                "INSERT INTO dependencies(task_id, depends_on) VALUES (?1, ?2)",
                params![id, depends_on],
            )?;
            let at = now()?;
            tx.execute(
                "UPDATE tasks SET revision=revision+1, updated_at=?2 WHERE id=?1",
                params![id, at],
            )?;
            record(&tx, id, "dependency_added", &depends_on.to_string(), &at)?;
            tx.commit()?;
            task_json(&conn, id)?
        }
        "work.link_evidence" => {
            let id = required_i64(args, "id")?;
            let expected = required_i64(args, "expected_revision")?;
            let path = required_str(args, "path")?;
            let evidence_file = safe_join(evidence_root, path)?;
            if !evidence_file.is_file() {
                bail!("evidence file does not exist: {path}");
            }
            let mut conn = open(db)?;
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            assert_revision(&tx, id, expected)?;
            tx.execute(
                "INSERT INTO evidence_links(task_id, path) VALUES (?1, ?2)",
                params![id, path],
            )?;
            let at = now()?;
            tx.execute(
                "UPDATE tasks SET revision=revision+1, updated_at=?2 WHERE id=?1",
                params![id, at],
            )?;
            record(&tx, id, "evidence_linked", path, &at)?;
            tx.commit()?;
            task_json(&conn, id)?
        }
        "work.close" => {
            let id = required_i64(args, "id")?;
            let expected = required_i64(args, "expected_revision")?;
            let mut conn = open(db)?;
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            assert_revision(&tx, id, expected)?;
            assert_dependencies_done(&tx, id)?;
            let at = now()?;
            tx.execute(
                "UPDATE tasks SET status='done', revision=revision+1, updated_at=?2 WHERE id=?1",
                params![id, at],
            )?;
            record(&tx, id, "closed", "done", &at)?;
            tx.commit()?;
            task_json(&conn, id)?
        }
        "evidence.list" => {
            let mut files = Vec::new();
            let mut dirs = vec![evidence_root.to_path_buf()];
            while let Some(dir) = dirs.pop() {
                for entry in fs::read_dir(dir)?.flatten() {
                    let path = entry.path();
                    let kind = entry.file_type()?;
                    if kind.is_symlink() {
                        bail!("evidence tree contains a symlink: {}", path.display());
                    } else if kind.is_dir() {
                        dirs.push(path);
                    } else if kind.is_file()
                        && path.extension().and_then(|x| x.to_str()) == Some("md")
                    {
                        files.push(
                            path.strip_prefix(evidence_root)?
                                .to_string_lossy()
                                .into_owned(),
                        );
                    }
                }
            }
            files.sort();
            json!(files)
        }
        "evidence.read" => json!({
            "path": required_str(args, "path")?,
            "content": fs::read_to_string(safe_join(evidence_root, required_str(args, "path")?)?)?
        }),
        "evidence.create" => {
            let path = required_str(args, "path")?;
            let target = safe_join(evidence_root, path)?;
            if target.exists() {
                bail!("evidence file already exists: {path}");
            }
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)?;
            }
            let content = required_str(args, "content")?;
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&target)?;
            file.write_all(content.as_bytes())?;
            json!({"path":path,"bytes":content.len()})
        }
        "evidence.edit" => {
            let path = required_str(args, "path")?;
            let target = safe_join(evidence_root, path)?;
            let old = required_str(args, "old_string")?;
            let new = required_str(args, "new_string")?;
            let content = fs::read_to_string(&target)?;
            match content.matches(old).count() {
                0 => bail!("old_string not found in {path}"),
                1 => fs::write(&target, content.replacen(old, new, 1))?,
                n => bail!("old_string matches {n} times in {path}; include more context"),
            }
            json!({"path":path,"edited":true})
        }
        other => bail!("unknown tool {other}"),
    };
    Ok(serde_json::to_string_pretty(&result)?)
}

pub fn run(db: PathBuf, evidence: PathBuf) -> Result<()> {
    init(&db, &evidence)?;
    let mut reader = BufReader::new(stdin());
    let mut out = stdout();
    while let Some(msg) = mcp::read_msg(&mut reader)? {
        let (id, method) = (msg.get("id").cloned(), msg["method"].as_str().unwrap_or(""));
        let Some(id) = id else { continue };
        let reply = match method {
            "initialize" => mcp::response(
                &id,
                json!({
                    "protocolVersion":msg["params"]["protocolVersion"].as_str().unwrap_or("2025-03-26"),
                    "capabilities":{"tools":{}},
                    "serverInfo":{"name":"asf-workboard-server","version":env!("CARGO_PKG_VERSION")}
                }),
            ),
            "ping" => mcp::response(&id, json!({})),
            "tools/list" => mcp::response(&id, json!({"tools":tool_defs()})),
            "tools/call" => {
                let name = msg["params"]["name"].as_str().unwrap_or("");
                let args = msg["params"].get("arguments").cloned().unwrap_or(json!({}));
                match call(&db, &evidence, name, &args) {
                    Ok(text) => mcp::tool_text(&id, &text),
                    Err(error) => mcp::tool_error(&id, &error.to_string()),
                }
            }
            other => mcp::method_not_found(&id, other),
        };
        mcp::write_msg(&mut out, &reply)?;
    }
    Ok(())
}

/// Human/operator entry point. It uses the same implementation as MCP but
/// writes trunk directly, which intentionally appears as human drift at the
/// next boundary when a fabric home already tracks these roots.
pub fn operator(db: &Path, evidence: &Path, action: &str, args: Value) -> Result<()> {
    init(db, evidence)?;
    println!("{}", call(db, evidence, action, &args)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_revision_and_dependency_cycles_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("workboard.db");
        let evidence = dir.path().join("evidence");
        init(&db, &evidence).unwrap();
        call(&db, &evidence, "work.create", &json!({"title":"first"})).unwrap();
        call(&db, &evidence, "work.create", &json!({"title":"second"})).unwrap();
        call(
            &db,
            &evidence,
            "work.add_dependency",
            &json!({"id":1,"depends_on":2,"expected_revision":1}),
        )
        .unwrap();
        let stale = call(
            &db,
            &evidence,
            "work.update",
            &json!({"id":1,"expected_revision":1,"status":"blocked"}),
        )
        .unwrap_err();
        assert!(stale.to_string().contains("stale revision"));
        let cycle = call(
            &db,
            &evidence,
            "work.add_dependency",
            &json!({"id":2,"depends_on":1,"expected_revision":1}),
        )
        .unwrap_err();
        assert!(cycle.to_string().contains("cycle"));

        let unfinished = call(
            &db,
            &evidence,
            "work.close",
            &json!({"id":1,"expected_revision":2}),
        )
        .unwrap_err();
        assert!(unfinished.to_string().contains("unfinished dependency"));
        call(
            &db,
            &evidence,
            "work.close",
            &json!({"id":2,"expected_revision":1}),
        )
        .unwrap();
        call(
            &db,
            &evidence,
            "work.close",
            &json!({"id":1,"expected_revision":2}),
        )
        .unwrap();
    }

    #[test]
    fn evidence_is_confined_and_create_never_overwrites() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("workboard.db");
        let evidence = dir.path().join("evidence");
        init(&db, &evidence).unwrap();
        call(
            &db,
            &evidence,
            "evidence.create",
            &json!({"path":"findings/one.md","content":"# One\n"}),
        )
        .unwrap();
        assert_eq!(
            fs::read_to_string(evidence.join("findings/one.md")).unwrap(),
            "# One\n"
        );
        assert!(call(
            &db,
            &evidence,
            "evidence.create",
            &json!({"path":"findings/one.md","content":"replace"})
        )
        .is_err());
        assert!(call(
            &db,
            &evidence,
            "evidence.read",
            &json!({"path":"../outside"})
        )
        .is_err());
    }

    #[cfg(unix)]
    #[test]
    fn evidence_operations_do_not_follow_symlinks() {
        use std::os::unix::fs::symlink;

        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("workboard.db");
        let evidence = dir.path().join("evidence");
        let outside = dir.path().join("outside");
        init(&db, &evidence).unwrap();
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("secret.md"), "secret").unwrap();
        symlink(&outside, evidence.join("linked")).unwrap();

        assert!(call(
            &db,
            &evidence,
            "evidence.read",
            &json!({"path":"linked/secret.md"})
        )
        .is_err());
        assert!(call(&db, &evidence, "evidence.list", &json!({})).is_err());
    }
}
