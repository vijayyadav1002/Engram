use crate::error::Error;
use crate::types::{Confidence, EdgeKind, ExtractedSymbol, ParseStatus, SymbolKind};
use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;

const DDL: &str = r#"
CREATE TABLE meta (
  schema_version INTEGER NOT NULL,
  indexed_at     TEXT,
  root           TEXT NOT NULL,
  file_count     INTEGER NOT NULL DEFAULT 0,
  symbol_count   INTEGER NOT NULL DEFAULT 0,
  edge_count     INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE files (
  id            INTEGER PRIMARY KEY,
  path          TEXT NOT NULL UNIQUE,
  language      TEXT,
  hash          TEXT NOT NULL,
  size          INTEGER NOT NULL,
  mtime         INTEGER NOT NULL,
  parse_status  TEXT NOT NULL
);

CREATE TABLE symbols (
  id          INTEGER PRIMARY KEY,
  file_id     INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,
  name        TEXT NOT NULL,
  kind        TEXT NOT NULL,
  start_line  INTEGER NOT NULL,
  end_line    INTEGER NOT NULL,
  start_byte  INTEGER NOT NULL,
  end_byte    INTEGER NOT NULL,
  signature   TEXT
);

CREATE TABLE edges (
  src_symbol_id INTEGER NOT NULL REFERENCES symbols(id) ON DELETE CASCADE,
  dst_symbol_id INTEGER NOT NULL REFERENCES symbols(id) ON DELETE CASCADE,
  kind          TEXT NOT NULL,
  confidence    TEXT NOT NULL,
  PRIMARY KEY (src_symbol_id, dst_symbol_id, kind)
);

CREATE VIRTUAL TABLE file_fts USING fts5(
  path,
  content,
  tokenize = 'unicode61'
);

CREATE INDEX idx_symbols_name ON symbols(name COLLATE NOCASE);
CREATE INDEX idx_symbols_file ON symbols(file_id);
CREATE INDEX idx_edges_src ON edges(src_symbol_id);
CREATE INDEX idx_edges_dst ON edges(dst_symbol_id);
CREATE INDEX idx_files_hash ON files(hash);
"#;

#[derive(Debug)]
pub struct Store {
    conn: Connection,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileRow {
    pub id: i64,
    pub path: String,
    pub language: Option<String>,
    pub hash: String,
    pub size: i64,
    pub mtime: i64,
    pub parse_status: ParseStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SymbolHit {
    pub id: i64,
    pub file_id: i64,
    pub path: String,
    pub name: String,
    pub kind: SymbolKind,
    pub start_line: u32,
    pub end_line: u32,
    pub start_byte: u32,
    pub end_byte: u32,
    pub signature: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FtsHit {
    pub path: String,
    pub rank: f64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NeighborHit {
    pub src_id: i64,
    pub dst_id: i64,
    pub src_name: String,
    pub dst_name: String,
    pub src_path: String,
    pub dst_path: String,
    pub kind: EdgeKind,
    pub confidence: Confidence,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IncomingEdge {
    pub src_id: i64,
    pub src_path: String,
    pub dst_name: String,
    pub kind: EdgeKind,
    pub confidence: Confidence,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Meta {
    pub schema_version: i64,
    pub indexed_at: Option<String>,
    pub root: String,
    pub file_count: i64,
    pub symbol_count: i64,
    pub edge_count: i64,
}

impl Store {
    pub const SCHEMA_VERSION: i64 = 1;

    pub fn create(path: &Path, root: &str) -> Result<Store, Error> {
        let conn = Connection::open(path).map_err(map_db)?;
        let store = Store { conn };
        store.apply_pragmas(false)?;
        store.conn.execute_batch(DDL).map_err(map_db)?;
        store
            .conn
            .execute(
                "INSERT INTO meta (schema_version, indexed_at, root, file_count, symbol_count, edge_count)
                 VALUES (?1, NULL, ?2, 0, 0, 0)",
                params![Self::SCHEMA_VERSION, root],
            )
            .map_err(map_db)?;
        Ok(store)
    }

    pub fn open_write(path: &Path) -> Result<Store, Error> {
        if !path.exists() {
            return Err(Error::NotInitialized);
        }
        let conn = Connection::open(path).map_err(map_db)?;
        let store = Store { conn };
        store.apply_pragmas(false)?;
        Ok(store)
    }

    pub fn open_read(path: &Path) -> Result<Store, Error> {
        if !path.exists() {
            return Err(Error::NotInitialized);
        }
        let conn = Connection::open(path).map_err(map_db)?;
        let store = Store { conn };
        store.apply_pragmas(true)?;
        Ok(store)
    }

    fn apply_pragmas(&self, query_only: bool) -> Result<(), Error> {
        self.conn
            .pragma_update(None, "journal_mode", "WAL")
            .map_err(map_db)?;
        self.conn
            .pragma_update(None, "synchronous", "NORMAL")
            .map_err(map_db)?;
        self.conn
            .pragma_update(None, "foreign_keys", "ON")
            .map_err(map_db)?;
        if query_only {
            self.conn
                .pragma_update(None, "query_only", "ON")
                .map_err(map_db)?;
        }
        Ok(())
    }

    pub fn upsert_file(&self, row: &FileRow) -> Result<i64, Error> {
        self.conn
            .execute(
                "INSERT INTO files (path, language, hash, size, mtime, parse_status)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(path) DO UPDATE SET
                   language = excluded.language,
                   hash = excluded.hash,
                   size = excluded.size,
                   mtime = excluded.mtime,
                   parse_status = excluded.parse_status",
                params![
                    row.path,
                    row.language,
                    row.hash,
                    row.size,
                    row.mtime,
                    row.parse_status.as_str(),
                ],
            )
            .map_err(map_db)?;
        let id: i64 = self
            .conn
            .query_row(
                "SELECT id FROM files WHERE path = ?1",
                params![row.path],
                |r| r.get(0),
            )
            .map_err(map_db)?;
        Ok(id)
    }

    pub fn delete_file_by_path(&self, path: &str) -> Result<(), Error> {
        self.conn
            .execute("DELETE FROM file_fts WHERE path = ?1", params![path])
            .map_err(map_db)?;
        self.conn
            .execute("DELETE FROM files WHERE path = ?1", params![path])
            .map_err(map_db)?;
        Ok(())
    }

    pub fn get_file(&self, path: &str) -> Result<Option<FileRow>, Error> {
        self.conn
            .query_row(
                "SELECT id, path, language, hash, size, mtime, parse_status
                 FROM files WHERE path = ?1",
                params![path],
                map_file_row,
            )
            .optional()
            .map_err(map_db)
    }

    pub fn list_files(&self) -> Result<Vec<FileRow>, Error> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, path, language, hash, size, mtime, parse_status
                 FROM files",
            )
            .map_err(map_db)?;
        let rows = stmt.query_map([], map_file_row).map_err(map_db)?;
        collect_hits(rows)
    }

    pub fn counts(&self) -> Result<(i64, i64, i64), Error> {
        let file_count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM files", [], |r| r.get(0))
            .map_err(map_db)?;
        let symbol_count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM symbols", [], |r| r.get(0))
            .map_err(map_db)?;
        let edge_count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM edges", [], |r| r.get(0))
            .map_err(map_db)?;
        Ok((file_count, symbol_count, edge_count))
    }

    pub fn replace_file_payload(
        &self,
        file_id: i64,
        symbols: &[ExtractedSymbol],
        fts_content: Option<&str>,
        path: &str,
    ) -> Result<Vec<(i64, String)>, Error> {
        let tx = self.conn.unchecked_transaction().map_err(map_db)?;
        tx.execute("DELETE FROM symbols WHERE file_id = ?1", params![file_id])
            .map_err(map_db)?;

        let mut out = Vec::with_capacity(symbols.len());
        for sym in symbols {
            tx.execute(
                "INSERT INTO symbols
                   (file_id, name, kind, start_line, end_line, start_byte, end_byte, signature)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    file_id,
                    sym.name,
                    sym.kind.as_str(),
                    sym.start_line,
                    sym.end_line,
                    sym.start_byte,
                    sym.end_byte,
                    sym.signature,
                ],
            )
            .map_err(map_db)?;
            let id = tx.last_insert_rowid();
            out.push((id, sym.name.clone()));
        }

        tx.execute("DELETE FROM file_fts WHERE path = ?1", params![path])
            .map_err(map_db)?;
        if let Some(content) = fts_content {
            tx.execute(
                "INSERT INTO file_fts (path, content) VALUES (?1, ?2)",
                params![path, content],
            )
            .map_err(map_db)?;
        }

        tx.commit().map_err(map_db)?;
        Ok(out)
    }

    pub fn insert_edges(&self, triples: &[(i64, i64, EdgeKind, Confidence)]) -> Result<(), Error> {
        let tx = self.conn.unchecked_transaction().map_err(map_db)?;
        for (src, dst, kind, confidence) in triples {
            tx.execute(
                "INSERT OR IGNORE INTO edges (src_symbol_id, dst_symbol_id, kind, confidence)
                 VALUES (?1, ?2, ?3, ?4)",
                params![src, dst, kind.as_str(), confidence.as_str()],
            )
            .map_err(map_db)?;
        }
        tx.commit().map_err(map_db)?;
        Ok(())
    }

    pub fn lookup_symbols_exact(&self, name: &str, limit: usize) -> Result<Vec<SymbolHit>, Error> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT s.id, s.file_id, f.path, s.name, s.kind,
                        s.start_line, s.end_line, s.start_byte, s.end_byte, s.signature
                 FROM symbols s
                 JOIN files f ON f.id = s.file_id
                 WHERE s.name = ?1 COLLATE NOCASE
                 LIMIT ?2",
            )
            .map_err(map_db)?;
        let rows = stmt
            .query_map(params![name, limit as i64], map_symbol_hit)
            .map_err(map_db)?;
        collect_hits(rows)
    }

    pub fn lookup_symbols_prefix(
        &self,
        prefix: &str,
        limit: usize,
    ) -> Result<Vec<SymbolHit>, Error> {
        let pattern = like_prefix(prefix);
        let mut stmt = self
            .conn
            .prepare(
                "SELECT s.id, s.file_id, f.path, s.name, s.kind,
                        s.start_line, s.end_line, s.start_byte, s.end_byte, s.signature
                 FROM symbols s
                 JOIN files f ON f.id = s.file_id
                 WHERE s.name LIKE ?1 ESCAPE '\\' COLLATE NOCASE
                 LIMIT ?2",
            )
            .map_err(map_db)?;
        let rows = stmt
            .query_map(params![pattern, limit as i64], map_symbol_hit)
            .map_err(map_db)?;
        collect_hits(rows)
    }

    pub fn fts_search(&self, query: &str, limit: usize) -> Result<Vec<FtsHit>, Error> {
        let ranked = self.conn.prepare(
            "SELECT path, rank FROM file_fts WHERE file_fts MATCH ?1 ORDER BY rank LIMIT ?2",
        );
        match ranked {
            Ok(mut stmt) => {
                let rows = stmt
                    .query_map(params![query, limit as i64], |r| {
                        Ok(FtsHit {
                            path: r.get(0)?,
                            rank: r.get(1)?,
                        })
                    })
                    .map_err(map_db)?;
                collect_hits(rows)
            }
            Err(_) => {
                let mut stmt = self
                    .conn
                    .prepare("SELECT path FROM file_fts WHERE file_fts MATCH ?1 LIMIT ?2")
                    .map_err(map_db)?;
                let rows = stmt
                    .query_map(params![query, limit as i64], |r| Ok(r.get::<_, String>(0)?))
                    .map_err(map_db)?;
                let paths: Vec<String> = collect_hits(rows)?;
                let n = paths.len();
                Ok(paths
                    .into_iter()
                    .enumerate()
                    .map(|(i, path)| FtsHit {
                        path,
                        rank: (n - i) as f64,
                    })
                    .collect())
            }
        }
    }

    /// Edges into `path` from other files, keyed by destination symbol name.
    pub fn incoming_edges(&self, path: &str) -> Result<Vec<IncomingEdge>, Error> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT e.src_symbol_id, fsrc.path, dst.name, e.kind, e.confidence
                 FROM edges e
                 JOIN symbols dst ON dst.id = e.dst_symbol_id
                 JOIN files fdst ON fdst.id = dst.file_id
                 JOIN symbols src ON src.id = e.src_symbol_id
                 JOIN files fsrc ON fsrc.id = src.file_id
                 WHERE fdst.path = ?1 AND fsrc.path != ?1",
            )
            .map_err(map_db)?;
        let rows = stmt
            .query_map(params![path], |r| {
                let kind_s: String = r.get(3)?;
                let conf_s: String = r.get(4)?;
                Ok(IncomingEdge {
                    src_id: r.get(0)?,
                    src_path: r.get(1)?,
                    dst_name: r.get(2)?,
                    kind: parse_edge_kind(&kind_s, 3)?,
                    confidence: parse_confidence(&conf_s, 4)?,
                })
            })
            .map_err(map_db)?;
        collect_hits(rows)
    }

    pub fn neighbors(&self, symbol_id: i64, cap: usize) -> Result<Vec<NeighborHit>, Error> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT e.src_symbol_id, e.dst_symbol_id,
                        s1.name, s2.name, f1.path, f2.path, e.kind, e.confidence
                 FROM edges e
                 JOIN symbols s1 ON s1.id = e.src_symbol_id
                 JOIN symbols s2 ON s2.id = e.dst_symbol_id
                 JOIN files f1 ON f1.id = s1.file_id
                 JOIN files f2 ON f2.id = s2.file_id
                 WHERE e.src_symbol_id = ?1 OR e.dst_symbol_id = ?1
                 LIMIT ?2",
            )
            .map_err(map_db)?;
        let rows = stmt
            .query_map(params![symbol_id, cap as i64], |r| {
                let kind_s: String = r.get(6)?;
                let conf_s: String = r.get(7)?;
                Ok(NeighborHit {
                    src_id: r.get(0)?,
                    dst_id: r.get(1)?,
                    src_name: r.get(2)?,
                    dst_name: r.get(3)?,
                    src_path: r.get(4)?,
                    dst_path: r.get(5)?,
                    kind: parse_edge_kind(&kind_s, 6)?,
                    confidence: parse_confidence(&conf_s, 7)?,
                })
            })
            .map_err(map_db)?;
        collect_hits(rows)
    }

    pub fn set_meta(
        &self,
        file_count: i64,
        symbol_count: i64,
        edge_count: i64,
    ) -> Result<(), Error> {
        self.conn
            .execute(
                "UPDATE meta SET file_count = ?1, symbol_count = ?2, edge_count = ?3, indexed_at = datetime('now')",
                params![file_count, symbol_count, edge_count],
            )
            .map_err(map_db)?;
        Ok(())
    }

    pub fn meta(&self) -> Result<Meta, Error> {
        self.conn
            .query_row(
                "SELECT schema_version, indexed_at, root, file_count, symbol_count, edge_count
                 FROM meta LIMIT 1",
                [],
                |r| {
                    Ok(Meta {
                        schema_version: r.get(0)?,
                        indexed_at: r.get(1)?,
                        root: r.get(2)?,
                        file_count: r.get(3)?,
                        symbol_count: r.get(4)?,
                        edge_count: r.get(5)?,
                    })
                },
            )
            .map_err(map_db)
    }
}

fn parse_edge_kind(kind_s: &str, idx: usize) -> rusqlite::Result<EdgeKind> {
    EdgeKind::from_str(kind_s).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            idx,
            rusqlite::types::Type::Text,
            format!("unknown edge kind: {kind_s}").into(),
        )
    })
}

fn parse_confidence(conf_s: &str, idx: usize) -> rusqlite::Result<Confidence> {
    Confidence::from_str(conf_s).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            idx,
            rusqlite::types::Type::Text,
            format!("unknown confidence: {conf_s}").into(),
        )
    })
}

fn map_file_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<FileRow> {
    let status_s: String = r.get(6)?;
    let parse_status = ParseStatus::from_str(&status_s).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            6,
            rusqlite::types::Type::Text,
            format!("unknown parse_status: {status_s}").into(),
        )
    })?;
    Ok(FileRow {
        id: r.get(0)?,
        path: r.get(1)?,
        language: r.get(2)?,
        hash: r.get(3)?,
        size: r.get(4)?,
        mtime: r.get(5)?,
        parse_status,
    })
}

fn map_symbol_hit(r: &rusqlite::Row<'_>) -> rusqlite::Result<SymbolHit> {
    let kind_s: String = r.get(4)?;
    let kind = SymbolKind::from_str(&kind_s).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            4,
            rusqlite::types::Type::Text,
            format!("unknown symbol kind: {kind_s}").into(),
        )
    })?;
    Ok(SymbolHit {
        id: r.get(0)?,
        file_id: r.get(1)?,
        path: r.get(2)?,
        name: r.get(3)?,
        kind,
        start_line: r.get::<_, i64>(5)? as u32,
        end_line: r.get::<_, i64>(6)? as u32,
        start_byte: r.get::<_, i64>(7)? as u32,
        end_byte: r.get::<_, i64>(8)? as u32,
        signature: r.get(9)?,
    })
}

fn collect_hits<T, I>(rows: I) -> Result<Vec<T>, Error>
where
    I: Iterator<Item = Result<T, rusqlite::Error>>,
{
    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(map_db)?);
    }
    Ok(out)
}

fn like_prefix(prefix: &str) -> String {
    let mut s = String::with_capacity(prefix.len() + 1);
    for c in prefix.chars() {
        match c {
            '\\' | '%' | '_' => {
                s.push('\\');
                s.push(c);
            }
            _ => s.push(c),
        }
    }
    s.push('%');
    s
}

fn map_db(err: rusqlite::Error) -> Error {
    let msg = err.to_string();
    if msg.contains("database is locked") {
        Error::IndexBusy
    } else {
        Error::Db(msg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::*;
    use std::path::PathBuf;

    fn tmp_db() -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static N: AtomicU64 = AtomicU64::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        let p =
            std::env::temp_dir().join(format!("engram-test-{}-{}.sqlite", std::process::id(), n));
        let _ = std::fs::remove_file(&p);
        p
    }

    #[test]
    fn create_schema_and_cascade_delete() {
        let path = tmp_db();
        let store = Store::create(&path, "/tmp/proj").unwrap();
        assert_eq!(store.meta().unwrap().schema_version, 1);

        let id = store
            .upsert_file(&FileRow {
                id: 0,
                path: "a.py".into(),
                language: Some("python".into()),
                hash: "abc".into(),
                size: 10,
                mtime: 1,
                parse_status: ParseStatus::Graph,
            })
            .unwrap();

        let syms = store
            .replace_file_payload(
                id,
                &[ExtractedSymbol {
                    name: "foo".into(),
                    kind: SymbolKind::Function,
                    start_line: 1,
                    end_line: 2,
                    start_byte: 0,
                    end_byte: 10,
                    signature: Some("def foo()".into()),
                }],
                Some("def foo():\n  pass\n"),
                "a.py",
            )
            .unwrap();
        assert_eq!(syms[0].1, "foo");

        let listed = store.list_files().unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].path, "a.py");
        assert_eq!(store.counts().unwrap(), (1, 1, 0));

        store.delete_file_by_path("a.py").unwrap();
        assert!(store.get_file("a.py").unwrap().is_none());
        assert!(store.lookup_symbols_exact("foo", 10).unwrap().is_empty());
        assert!(store.list_files().unwrap().is_empty());
        assert_eq!(store.counts().unwrap(), (0, 0, 0));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn fts_finds_content() {
        let path = tmp_db();
        let store = Store::create(&path, "/tmp/proj").unwrap();
        let id = store
            .upsert_file(&FileRow {
                id: 0,
                path: "README.md".into(),
                language: Some("markdown".into()),
                hash: "h".into(),
                size: 20,
                mtime: 1,
                parse_status: ParseStatus::Outline,
            })
            .unwrap();
        store
            .replace_file_payload(id, &[], Some("WebSockets replaced polling"), "README.md")
            .unwrap();
        let hits = store.fts_search("WebSockets", 10).unwrap();
        assert_eq!(hits[0].path, "README.md");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn incoming_edges_from_other_files() {
        let path = tmp_db();
        let store = Store::create(&path, "/tmp/proj").unwrap();
        let a = store
            .upsert_file(&FileRow {
                id: 0,
                path: "a.ts".into(),
                language: Some("typescript".into()),
                hash: "a".into(),
                size: 1,
                mtime: 1,
                parse_status: ParseStatus::Graph,
            })
            .unwrap();
        let b = store
            .upsert_file(&FileRow {
                id: 0,
                path: "b.ts".into(),
                language: Some("typescript".into()),
                hash: "b".into(),
                size: 1,
                mtime: 1,
                parse_status: ParseStatus::Graph,
            })
            .unwrap();
        let a_syms = store
            .replace_file_payload(
                a,
                &[ExtractedSymbol {
                    name: "src".into(),
                    kind: SymbolKind::Function,
                    start_line: 1,
                    end_line: 1,
                    start_byte: 0,
                    end_byte: 1,
                    signature: None,
                }],
                None,
                "a.ts",
            )
            .unwrap();
        let b_syms = store
            .replace_file_payload(
                b,
                &[ExtractedSymbol {
                    name: "dst".into(),
                    kind: SymbolKind::Function,
                    start_line: 1,
                    end_line: 1,
                    start_byte: 0,
                    end_byte: 1,
                    signature: None,
                }],
                None,
                "b.ts",
            )
            .unwrap();
        store
            .insert_edges(&[(a_syms[0].0, b_syms[0].0, EdgeKind::Call, Confidence::High)])
            .unwrap();
        let incoming = store.incoming_edges("b.ts").unwrap();
        assert_eq!(incoming.len(), 1);
        assert_eq!(incoming[0].src_path, "a.ts");
        assert_eq!(incoming[0].dst_name, "dst");
        assert!(store.incoming_edges("a.ts").unwrap().is_empty());
        let _ = std::fs::remove_file(&path);
    }
}
