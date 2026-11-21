//! SQLite is a selected DB capability, never an application SQL bridge.
use crate::files::FileError;
use rusqlite::{
    Connection, OpenFlags,
    config::DbConfig,
    hooks::{AuthAction, Authorization},
    limits::Limit,
    types::ValueRef,
};
use serde::Serialize;
use std::{
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Table {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<String>>,
    pub truncated: bool,
    pub changes: u64,
}
fn fail(e: rusqlite::Error) -> FileError {
    FileError::new("sqlite", format!("SQLite refused the query: {e}"))
}
fn function(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "abs"
            | "avg"
            | "count"
            | "sum"
            | "total"
            | "min"
            | "max"
            | "lower"
            | "upper"
            | "length"
            | "substr"
            | "substring"
            | "trim"
            | "ltrim"
            | "rtrim"
            | "replace"
            | "coalesce"
            | "ifnull"
            | "nullif"
            | "round"
            | "typeof"
            | "instr"
            | "hex"
            | "quote"
            | "printf"
            | "format"
            | "like"
            | "glob"
            | "date"
            | "time"
            | "datetime"
            | "julianday"
            | "unixepoch"
            | "strftime"
            | "json"
            | "json_extract"
            | "json_array_length"
            | "json_type"
            | "json_valid"
            | "json_quote"
            | "json_array"
            | "json_object"
            | "json_group_array"
            | "json_group_object"
    )
}
// Admission only: the connection/authorizer enforce read/write permissions.
// Bound one statement before rusqlite's recursive tail validation.
fn single_statement(sql: &str) -> bool {
    let b = sql.as_bytes();
    let mut i = 0;
    let mut ended = false;
    while i < b.len() {
        if b[i].is_ascii_whitespace() {
            i += 1;
            continue;
        }
        if b.get(i..i + 2) == Some(b"--") {
            while i < b.len() && b[i] != b'\r' && b[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if b.get(i..i + 2) == Some(b"/*") {
            i += 2;
            while i + 1 < b.len() && b.get(i..i + 2) != Some(b"*/") {
                i += 1;
            }
            if i + 1 >= b.len() {
                return false;
            }
            i += 2;
            continue;
        }
        if ended {
            return false;
        }
        match b[i] {
            b';' => {
                ended = true;
                i += 1;
            }
            b'\'' | b'"' | b'`' => {
                let quote = b[i];
                i += 1;
                loop {
                    if i >= b.len() {
                        return false;
                    }
                    if b[i] == quote {
                        i += 1;
                        if b.get(i) == Some(&quote) {
                            i += 1;
                        } else {
                            break;
                        }
                    } else {
                        i += 1;
                    }
                }
            }
            b'[' => {
                i += 1;
                while i < b.len() && b[i] != b']' {
                    i += 1;
                }
                if i == b.len() {
                    return false;
                }
                i += 1;
            }
            _ => i += 1,
        }
    }
    true
}
pub fn query(
    path: &Path,
    sql: &str,
    write: bool,
    cancel: Arc<AtomicBool>,
    validate: impl Fn() -> Result<(), FileError>,
) -> Result<Table, FileError> {
    if sql.is_empty() || sql.len() > 65536 || sql.contains('\0') {
        return Err(FileError::new("capacity", "SQL admits 64 KiB without NUL."));
    }
    if !single_statement(sql) {
        return Err(FileError::new(
            "sqlite",
            "Run one complete SQL statement at a time.",
        ));
    }
    validate()?;
    let flags = (if write {
        OpenFlags::SQLITE_OPEN_READ_WRITE
    } else {
        OpenFlags::SQLITE_OPEN_READ_ONLY
    }) | OpenFlags::SQLITE_OPEN_NO_MUTEX
        | OpenFlags::SQLITE_OPEN_NOFOLLOW;
    let mut db = Connection::open_with_flags(path, flags).map_err(fail)?;
    // Bound hostile schema parsing before any query (including trigger checks).
    for (limit, n) in [
        (Limit::SQLITE_LIMIT_LENGTH, 1_048_576),
        (Limit::SQLITE_LIMIT_SQL_LENGTH, 65536),
        (Limit::SQLITE_LIMIT_COLUMN, 64),
        (Limit::SQLITE_LIMIT_EXPR_DEPTH, 32),
        (Limit::SQLITE_LIMIT_COMPOUND_SELECT, 8),
        (Limit::SQLITE_LIMIT_VDBE_OP, 250000),
        (Limit::SQLITE_LIMIT_FUNCTION_ARG, 32),
        (Limit::SQLITE_LIMIT_ATTACHED, 0),
        (Limit::SQLITE_LIMIT_LIKE_PATTERN_LENGTH, 256),
        (Limit::SQLITE_LIMIT_VARIABLE_NUMBER, 32),
        (Limit::SQLITE_LIMIT_TRIGGER_DEPTH, 0),
        (Limit::SQLITE_LIMIT_WORKER_THREADS, 0),
    ] {
        db.set_limit(limit, n).map_err(fail)?;
    }
    db.set_db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_FKEY, true)
        .map_err(fail)?;
    validate()?;
    db.busy_timeout(Duration::from_millis(250)).map_err(fail)?;
    for (flag, on) in [
        (DbConfig::SQLITE_DBCONFIG_DEFENSIVE, true),
        (DbConfig::SQLITE_DBCONFIG_TRUSTED_SCHEMA, false),
        (DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER, false),
        (DbConfig::SQLITE_DBCONFIG_ENABLE_VIEW, false),
        (DbConfig::SQLITE_DBCONFIG_NO_CKPT_ON_CLOSE, true),
    ] {
        db.set_db_config(flag, on).map_err(fail)?;
    }
    if write {
        let triggers: i64 = db
            .query_row(
                "SELECT count(*) FROM sqlite_schema WHERE type='trigger'",
                [],
                |r| r.get(0),
            )
            .map_err(fail)?;
        if triggers != 0 {
            return Err(FileError::new(
                "sqlite",
                "Write mode refuses databases with triggers; their behavior is not silently disabled.",
            ));
        }
    }
    if !write {
        db.execute_batch("PRAGMA query_only=ON").map_err(fail)?;
    }
    let began = Instant::now();
    let stop = cancel.clone();
    db.progress_handler(
        1000,
        Some(move || stop.load(Ordering::SeqCst) || began.elapsed() > Duration::from_secs(5)),
    )
    .map_err(fail)?;
    if write {
        db.execute_batch("BEGIN IMMEDIATE").map_err(fail)?;
    }
    db.authorizer(Some(move |ctx: rusqlite::hooks::AuthContext<'_>| {
        if ctx.database_name.is_some_and(|n| n != "main") {
            return Authorization::Deny;
        }
        let allowed = match ctx.action {
            AuthAction::Select | AuthAction::Recursive => true,
            AuthAction::Read { .. } => true,
            AuthAction::Function { function_name } => function(function_name),
            AuthAction::Insert { table_name }
            | AuthAction::Delete { table_name }
            | AuthAction::Update { table_name, .. } => {
                write && !table_name.to_ascii_lowercase().starts_with("sqlite_")
            }
            _ => false,
        };
        if allowed {
            Authorization::Allow
        } else {
            Authorization::Deny
        }
    }))
    .map_err(fail)?;
    let result = (|| {
        // prepare() plus extra_check refuses a second statement. A single DML is
        // still transaction-wrapped, including RETURNING and cancellation.
        let mut statement = db.prepare(sql).map_err(fail)?;
        if !write && !statement.readonly() {
            return Err(FileError::new("sqlite", "Readonly mode refused a write."));
        }
        let columns = statement
            .column_names()
            .map(|s| s.to_string())
            .collect::<Vec<_>>();
        let mut rows = statement.query([]).map_err(fail)?;
        let mut output = vec![];
        let mut bytes = 0usize;
        let mut truncated = false;
        while let Some(row) = rows.next().map_err(fail)? {
            if cancel.load(Ordering::SeqCst) || began.elapsed() > Duration::from_secs(5) {
                return Err(FileError::new(
                    "canceled",
                    "SQLite query interrupted; unfinished writes rolled back.",
                ));
            }
            if output.len() == 500 {
                if write {
                    return Err(FileError::new(
                        "capacity",
                        "Write result exceeded 500 rows; transaction rolled back.",
                    ));
                }
                truncated = true;
                break;
            }
            let mut values = vec![];
            for i in 0..columns.len() {
                let text = match row.get_ref(i).map_err(fail)? {
                    ValueRef::Null => "NULL".into(),
                    ValueRef::Integer(n) => n.to_string(),
                    ValueRef::Real(n) => n.to_string(),
                    ValueRef::Text(b) => String::from_utf8_lossy(b).into_owned(),
                    ValueRef::Blob(b) => format!("[{} byte blob]", b.len()),
                };
                bytes += text.len();
                if bytes > 1024 * 1024 {
                    return Err(FileError::new(
                        "capacity",
                        "Query output exceeded 1 MiB; unfinished writes rolled back.",
                    ));
                }
                values.push(text);
            }
            output.push(values);
        }
        drop(rows);
        drop(statement);
        validate()?;
        if cancel.load(Ordering::SeqCst) || began.elapsed() > Duration::from_secs(5) {
            return Err(FileError::new(
                "canceled",
                "SQLite query interrupted; unfinished writes rolled back.",
            ));
        }
        Ok(Table {
            columns,
            rows: output,
            truncated,
            changes: db.changes(),
        })
    })();
    db.authorizer(None::<fn(rusqlite::hooks::AuthContext<'_>) -> Authorization>)
        .map_err(fail)?;
    db.progress_handler(0, None::<fn() -> bool>).map_err(fail)?;
    if write {
        if result.is_ok() {
            validate()?;
            if cancel.load(Ordering::SeqCst) {
                let _ = db.execute_batch("ROLLBACK");
                return Err(FileError::new(
                    "canceled",
                    "Write canceled before commit; rolled back.",
                ));
            }
            db.execute_batch("COMMIT").map_err(fail)?;
        } else {
            let _ = db.execute_batch("ROLLBACK");
        }
    }
    result
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn readonly_authorizer_dml_rollback_cancel_and_wal_are_real() {
        let t = tempfile::tempdir().unwrap();
        let path = t.path().join("selected.db");
        let db = Connection::open(&path).unwrap();
        db.execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE items(n INTEGER); INSERT INTO items VALUES(1); CREATE VIEW unsafe AS SELECT n FROM items;").unwrap();
        let path = std::fs::canonicalize(&path).unwrap();
        let c = Arc::new(AtomicBool::new(false));
        assert_eq!(
            query(&path, "SELECT n FROM items", false, c.clone(), || Ok(()))
                .unwrap()
                .rows[0][0],
            "1"
        );
        for sql in [
            "INSERT INTO items VALUES(2)",
            "ATTACH DATABASE 'escape.db' AS other",
            "PRAGMA writable_schema=ON",
            "SELECT load_extension('x')",
            "SELECT readfile('/etc/passwd')",
            "SELECT randomblob(100000000)",
            "SELECT * FROM unsafe",
            "CREATE TABLE forbidden(n)",
            "SELECT 1; DELETE FROM items",
        ] {
            assert!(
                query(&path, sql, false, c.clone(), || Ok(())).is_err(),
                "{sql}"
            );
        }
        query(&path, "UPDATE items SET n=2", true, c.clone(), || Ok(())).unwrap();
        assert_eq!(
            db.query_row("SELECT n FROM items", [], |r| r.get::<_, i32>(0))
                .unwrap(),
            2
        );
        assert!(
            query(
                &path,
                "UPDATE items SET n=n+1 RETURNING randomblob(2000000)",
                true,
                c.clone(),
                || Ok(())
            )
            .is_err()
        );
        assert_eq!(
            db.query_row("SELECT n FROM items", [], |r| r.get::<_, i32>(0))
                .unwrap(),
            2
        );
        let stop = c.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(30));
            stop.store(true, Ordering::SeqCst);
        });
        assert!(query(&path,"WITH RECURSIVE x(n) AS (VALUES(1) UNION ALL SELECT n+1 FROM x WHERE n<100000000) UPDATE items SET n=(SELECT sum(n) FROM x)",true,c,||Ok(())).is_err());
        assert_eq!(
            db.query_row("SELECT n FROM items", [], |r| r.get::<_, i32>(0))
                .unwrap(),
            2
        );
        assert!(!t.path().join("escape.db").exists());
    }
}

#[cfg(test)]
mod late_rollback {
    use super::*;
    #[test]
    fn actual_writes_rollback_on_output_capacity_and_cancel_before_commit() {
        let t = tempfile::tempdir().unwrap();
        let path = t.path().join("db");
        let db = Connection::open(&path).unwrap();
        db.execute_batch("CREATE TABLE items(n INTEGER);WITH RECURSIVE x(n) AS(VALUES(1) UNION ALL SELECT n+1 FROM x WHERE n<600) INSERT INTO items SELECT n FROM x;").unwrap();
        let path = std::fs::canonicalize(path).unwrap();
        let before: i64 = db
            .query_row("SELECT sum(n) FROM items", [], |r| r.get(0))
            .unwrap();
        let flag = Arc::new(AtomicBool::new(false));
        assert!(
            query(
                &path,
                "UPDATE items SET n=n+1 RETURNING n",
                true,
                flag.clone(),
                || Ok(())
            )
            .is_err()
        );
        assert_eq!(
            db.query_row("SELECT sum(n) FROM items", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            before
        );
        let count = std::sync::atomic::AtomicUsize::new(0);
        let stop = flag.clone();
        assert!(
            query(&path, "UPDATE items SET n=n+1", true, flag, || {
                if count.fetch_add(1, Ordering::SeqCst) == 2 {
                    stop.store(true, Ordering::SeqCst);
                }
                Ok(())
            })
            .is_err()
        );
        assert_eq!(
            db.query_row("SELECT sum(n) FROM items", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            before
        );
        for sql in [
            "SELECT 1;SELECT 2;SELECT 3;",
            "SELECT 1; -- comment\n DELETE FROM items",
        ] {
            assert!(!single_statement(sql));
        }
        assert!(single_statement("SELECT ';' AS \"x;\"; -- one statement"));
    }
}

#[cfg(test)]
mod hostile_schema {
    use super::*;
    #[test]
    fn limits_precede_write_schema_loading_and_wal_remains_live() {
        let t = tempfile::tempdir().unwrap();
        let path = t.path().join("hostile.db");
        let db = Connection::open(&path).unwrap();
        let columns = (0..65)
            .map(|n| format!("c{n} INTEGER"))
            .collect::<Vec<_>>()
            .join(",");
        db.execute_batch(&format!("PRAGMA journal_mode=WAL;CREATE TABLE wide({columns});CREATE TABLE ordinary(n INTEGER);INSERT INTO ordinary VALUES(1);")).unwrap();
        let path = std::fs::canonicalize(path).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        assert!(
            query(
                &path,
                "INSERT INTO ordinary VALUES(2)",
                true,
                stop.clone(),
                || Ok(())
            )
            .is_err(),
            "hostile schema must be refused before write preparation"
        );
        assert_eq!(
            db.query_row("SELECT sum(n) FROM ordinary", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            1
        );
        db.execute_batch("DROP TABLE wide;").unwrap();
        assert_eq!(
            query(&path, "SELECT n FROM ordinary", false, stop.clone(), || Ok(
                ()
            ))
            .unwrap()
            .rows[0][0],
            "1"
        );
        db.execute_batch("UPDATE ordinary SET n=3;").unwrap();
        assert_eq!(
            query(&path, "SELECT n FROM ordinary", false, stop.clone(), || Ok(
                ()
            ))
            .unwrap()
            .rows[0][0],
            "3",
            "a selected WAL database is mutable, not an immutable snapshot"
        );
        db.execute_batch("CREATE TRIGGER side_effect AFTER UPDATE ON ordinary BEGIN INSERT INTO ordinary VALUES(99); END;").unwrap();
        assert!(
            query(&path, "UPDATE ordinary SET n=4", true, stop.clone(), || Ok(
                ()
            ))
            .is_err()
        );
        assert!(
            query(
                &path,
                "SELECT printf('%1000000000s','x')",
                false,
                stop,
                || Ok(())
            )
            .is_err()
        );
        assert_eq!(
            db.query_row("SELECT sum(n) FROM ordinary", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            3
        );
        assert!(
            std::fs::metadata(t.path().join("hostile.db-wal"))
                .unwrap()
                .len()
                > 0
        );
    }
}
