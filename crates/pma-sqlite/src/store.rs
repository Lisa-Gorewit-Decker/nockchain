use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int, c_void};
use std::path::{Path, PathBuf};
use std::ptr;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use libsqlite3_sys as sqlite;
use nockvm::ext::AtomExt;
use nockvm::mem::NockStack;
use nockvm::noun::{Atom, Noun};
use nockvm::pma::{Pma, PmaCopy};
use nockvm::serialization;
use tracing::debug;

use crate::lru::LruCache;
use crate::{PmaSqliteError, Result};

#[derive(Debug, Clone)]
pub struct SqlitePmaConfig {
    pub path: PathBuf,
    pub cache_capacity: usize,
    pub stack_words_hint: usize,
}

impl SqlitePmaConfig {
    pub fn new<P: AsRef<Path>>(path: P) -> Self {
        Self {
            path: path.as_ref().to_path_buf(),
            cache_capacity: 1024,
            stack_words_hint: 1024,
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SqlitePmaStats {
    pub cache_hits: u64,
    pub cache_misses: u64,
    pub inserts: u64,
}

pub struct CachedNoun<'a> {
    stack: &'a mut NockStack,
    root: Noun,
}

impl<'a> CachedNoun<'a> {
    fn new(stack: &'a mut NockStack, root: Noun) -> Self {
        Self { stack, root }
    }

    pub fn root(&self) -> Noun {
        self.root
    }

    pub fn stack(&self) -> &NockStack {
        self.stack
    }

    pub fn stack_mut(&mut self) -> &mut NockStack {
        self.stack
    }
}

pub struct SqlitePma {
    db: SqliteDb,
    insert_stmt: SqliteStatement,
    select_stmt: SqliteStatement,
    list_stmt: SqliteStatement,
    cache: LruCache<i64, CachedEntry>,
    stats: SqlitePmaStats,
    stack_words_hint: usize,
    cache_pma: Pma,
    cache_pma_words: usize,
    work_stack: NockStack,
    work_stack_words: usize,
}

impl SqlitePma {
    pub fn open(config: SqlitePmaConfig) -> Result<Self> {
        let db = SqliteDb::open(&config.path)?;
        db.exec(
            "PRAGMA journal_mode=WAL;\
             PRAGMA synchronous=NORMAL;\
             PRAGMA temp_store=MEMORY;\
             CREATE TABLE IF NOT EXISTS nouns (\
                 id INTEGER PRIMARY KEY AUTOINCREMENT,\
                 jam BLOB NOT NULL\
             );",
        )?;

        let insert_stmt = db.prepare("INSERT INTO nouns (jam) VALUES (?1)")?;
        let select_stmt = db.prepare("SELECT jam FROM nouns WHERE id = ?1")?;
        let list_stmt = db.prepare("SELECT id FROM nouns ORDER BY id")?;

        let cache_capacity = config.cache_capacity.max(1);
        let cache_pma_words = config
            .stack_words_hint
            .saturating_mul(cache_capacity)
            .max(config.stack_words_hint.max(1024));
        let cache_pma_path = cache_pma_path();
        let cache_pma = Pma::new(cache_pma_words, cache_pma_path)?;
        let work_stack_words = config.stack_words_hint.max(1024);
        let (mut work_stack, _) = NockStack::new_(work_stack_words, 0)?;
        work_stack.install_pma_arena(Arc::clone(cache_pma.arena()));

        Ok(Self {
            db,
            insert_stmt,
            select_stmt,
            list_stmt,
            cache: LruCache::new(cache_capacity),
            stats: SqlitePmaStats::default(),
            stack_words_hint: config.stack_words_hint,
            cache_pma,
            cache_pma_words,
            work_stack,
            work_stack_words,
        })
    }

    pub fn stats(&self) -> SqlitePmaStats {
        self.stats
    }

    pub fn begin_transaction(&mut self) -> Result<()> {
        self.db.exec("BEGIN")
    }

    pub fn commit_transaction(&mut self) -> Result<()> {
        self.db.exec("COMMIT")
    }

    pub fn rollback_transaction(&mut self) -> Result<()> {
        self.db.exec("ROLLBACK")
    }

    pub fn insert_noun(&mut self, stack: &mut NockStack, noun: Noun) -> Result<i64> {
        let jammed = jam_noun(stack, noun);
        self.insert_jam(&jammed)
    }

    pub fn insert_jam(&mut self, jammed: &[u8]) -> Result<i64> {
        self.insert_stmt.reset()?;
        self.insert_stmt.bind_blob(1, jammed)?;
        match self.insert_stmt.step()? {
            Step::Done => {}
            Step::Row => {
                return Err(PmaSqliteError::Sqlite(
                    "unexpected row while inserting jam".to_string(),
                ));
            }
        }
        let id = self.db.last_insert_rowid();
        self.stats.inserts = self.stats.inserts.saturating_add(1);
        Ok(id)
    }

    pub fn with_cached<R, F>(&mut self, id: i64, f: F) -> Result<R>
    where
        F: FnOnce(&mut CachedNoun<'_>) -> R,
    {
        if let Some(root) = self.cache.get_mut(&id).map(|entry| entry.root) {
            self.stats.cache_hits = self.stats.cache_hits.saturating_add(1);
            unsafe {
                self.work_stack.reset(0);
            }
            let mut cached = CachedNoun::new(&mut self.work_stack, root);
            return Ok(f(&mut cached));
        }

        let jam = self.get_jam(id)?;
        let estimate_words = estimate_stack_words(jam.len(), self.stack_words_hint);
        self.ensure_cache_capacity(estimate_words)?;
        self.ensure_work_stack_capacity(estimate_words)?;
        unsafe {
            self.work_stack.reset(0);
        }
        let mut root = decode_jam_into_stack(&mut self.work_stack, &jam)?;
        unsafe {
            root.copy_to_pma(&self.work_stack, &mut self.cache_pma);
        }
        unsafe {
            self.work_stack.reset(0);
        }
        self.cache.insert(id, CachedEntry { root });
        self.stats.cache_misses = self.stats.cache_misses.saturating_add(1);

        let mut cached = CachedNoun::new(&mut self.work_stack, root);
        Ok(f(&mut cached))
    }

    pub fn get_jam(&mut self, id: i64) -> Result<Vec<u8>> {
        self.select_stmt.reset()?;
        self.select_stmt.bind_int64(1, id)?;
        let jam = match self.select_stmt.step()? {
            Step::Row => self.select_stmt.column_blob(0)?,
            Step::Done => return Err(PmaSqliteError::Missing(id)),
        };
        Ok(jam)
    }

    pub fn list_ids(&mut self) -> Result<Vec<i64>> {
        self.list_stmt.reset()?;
        let mut ids = Vec::new();
        loop {
            match self.list_stmt.step()? {
                Step::Row => ids.push(self.list_stmt.column_int64(0)?),
                Step::Done => break,
            }
        }
        Ok(ids)
    }

    pub fn clear_cache(&mut self) {
        self.cache = LruCache::new(self.cache.capacity());
        self.cache_pma.reset();
        unsafe {
            self.work_stack.reset(0);
        }
    }

    fn ensure_cache_capacity(&mut self, estimate_words: usize) -> Result<()> {
        if estimate_words > self.cache_pma_words {
            return Err(PmaSqliteError::Sqlite(
                "cache arena too small for noun".to_string(),
            ));
        }
        if self.cache_pma.free_words() < estimate_words {
            self.cache_pma.reset();
            self.cache = LruCache::new(self.cache.capacity());
        }
        Ok(())
    }

    fn ensure_work_stack_capacity(&mut self, estimate_words: usize) -> Result<()> {
        if estimate_words <= self.work_stack_words {
            return Ok(());
        }
        let new_words = estimate_words
            .max(self.work_stack_words.saturating_mul(2))
            .max(1024);
        let (mut work_stack, _) = NockStack::new_(new_words, 0)?;
        work_stack.install_pma_arena(Arc::clone(self.cache_pma.arena()));
        self.work_stack = work_stack;
        self.work_stack_words = new_words;
        Ok(())
    }
}

fn jam_noun(stack: &mut NockStack, noun: Noun) -> Vec<u8> {
    let jammed = serialization::jam(stack, noun);
    let space = stack.noun_space();
    jammed.in_space(&space).to_ne_bytes()
}

fn decode_jam_into_stack(stack: &mut NockStack, jam: &[u8]) -> Result<Noun> {
    let atom = <Atom as AtomExt>::from_bytes(stack, jam);
    let root = serialization::cue(stack, atom)?;
    debug!(
        "sqlite-pma: decoded jam bytes len={} into stack_words={}",
        jam.len(),
        stack.arena_ref().words()
    );
    Ok(root)
}

fn estimate_stack_words(jam_len: usize, stack_words_hint: usize) -> usize {
    let jam_words = jam_len.saturating_add(7) / 8;
    let estimate = jam_words.saturating_mul(4).saturating_add(1024);
    estimate.max(stack_words_hint)
}

#[derive(Debug)]
struct CachedEntry {
    root: Noun,
}

fn cache_pma_path() -> PathBuf {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    std::env::temp_dir().join(format!(
        "pma_sqlite_cache_{}_{}",
        std::process::id(),
        now.as_nanos()
    ))
}

#[derive(Debug)]
struct SqliteDb {
    raw: *mut sqlite::sqlite3,
}

impl SqliteDb {
    fn open(path: &Path) -> Result<Self> {
        let path_str = path.to_str().ok_or(PmaSqliteError::InvalidPath)?;
        let c_path = CString::new(path_str).map_err(|_| PmaSqliteError::InvalidPath)?;
        let mut raw = ptr::null_mut();
        let rc = unsafe { sqlite::sqlite3_open(c_path.as_ptr(), &mut raw) };
        if rc != sqlite::SQLITE_OK {
            let message = sqlite_error_message(raw, rc);
            if !raw.is_null() {
                unsafe {
                    sqlite::sqlite3_close(raw);
                }
            }
            return Err(PmaSqliteError::Sqlite(message));
        }
        Ok(Self { raw })
    }

    fn exec(&self, sql: &str) -> Result<()> {
        let c_sql = CString::new(sql).map_err(|_| PmaSqliteError::Sqlite("invalid SQL".into()))?;
        let mut err_msg: *mut c_char = ptr::null_mut();
        let rc = unsafe {
            sqlite::sqlite3_exec(
                self.raw,
                c_sql.as_ptr(),
                None,
                ptr::null_mut(),
                &mut err_msg,
            )
        };
        if rc != sqlite::SQLITE_OK {
            let message = if !err_msg.is_null() {
                let message = unsafe { CStr::from_ptr(err_msg) }
                    .to_string_lossy()
                    .into_owned();
                unsafe {
                    sqlite::sqlite3_free(err_msg as *mut c_void);
                }
                format!("sqlite exec failed: {message} (code {rc})")
            } else {
                sqlite_error_message(self.raw, rc)
            };
            return Err(PmaSqliteError::Sqlite(message));
        }
        Ok(())
    }

    fn prepare(&self, sql: &str) -> Result<SqliteStatement> {
        let c_sql = CString::new(sql).map_err(|_| PmaSqliteError::Sqlite("invalid SQL".into()))?;
        let mut stmt = ptr::null_mut();
        let rc = unsafe {
            sqlite::sqlite3_prepare_v2(self.raw, c_sql.as_ptr(), -1, &mut stmt, ptr::null_mut())
        };
        if rc != sqlite::SQLITE_OK {
            return Err(PmaSqliteError::Sqlite(sqlite_error_message(self.raw, rc)));
        }
        Ok(SqliteStatement { raw: stmt })
    }

    fn last_insert_rowid(&self) -> i64 {
        unsafe { sqlite::sqlite3_last_insert_rowid(self.raw) as i64 }
    }
}

impl Drop for SqliteDb {
    fn drop(&mut self) {
        if !self.raw.is_null() {
            unsafe {
                sqlite::sqlite3_close(self.raw);
            }
        }
    }
}

#[derive(Debug)]
struct SqliteStatement {
    raw: *mut sqlite::sqlite3_stmt,
}

impl SqliteStatement {
    fn bind_blob(&mut self, index: i32, data: &[u8]) -> Result<()> {
        let len = data
            .len()
            .try_into()
            .map_err(|_| PmaSqliteError::Sqlite("blob too large".into()))?;
        let ptr = if data.is_empty() {
            ptr::null()
        } else {
            data.as_ptr()
        };
        let rc = unsafe {
            sqlite::sqlite3_bind_blob(
                self.raw,
                index,
                ptr as *const c_void,
                len,
                sqlite::SQLITE_TRANSIENT(),
            )
        };
        self.check(rc, "bind blob")
    }

    fn bind_int64(&mut self, index: i32, value: i64) -> Result<()> {
        let rc = unsafe { sqlite::sqlite3_bind_int64(self.raw, index, value) };
        self.check(rc, "bind int64")
    }

    fn column_blob(&self, index: i32) -> Result<Vec<u8>> {
        let size = unsafe { sqlite::sqlite3_column_bytes(self.raw, index) };
        if size < 0 {
            return Err(PmaSqliteError::Sqlite("negative blob size returned".into()));
        }
        let ptr = unsafe { sqlite::sqlite3_column_blob(self.raw, index) } as *const u8;
        if ptr.is_null() && size > 0 {
            return Err(PmaSqliteError::Sqlite("null blob pointer returned".into()));
        }
        let slice = if size == 0 {
            &[]
        } else {
            unsafe { std::slice::from_raw_parts(ptr, size as usize) }
        };
        Ok(slice.to_vec())
    }

    fn column_int64(&self, index: i32) -> Result<i64> {
        Ok(unsafe { sqlite::sqlite3_column_int64(self.raw, index) })
    }

    fn step(&mut self) -> Result<Step> {
        let rc = unsafe { sqlite::sqlite3_step(self.raw) };
        match rc {
            sqlite::SQLITE_ROW => Ok(Step::Row),
            sqlite::SQLITE_DONE => Ok(Step::Done),
            _ => Err(PmaSqliteError::Sqlite(sqlite_error_message(
                self.db_handle(),
                rc,
            ))),
        }
    }

    fn reset(&mut self) -> Result<()> {
        let rc = unsafe { sqlite::sqlite3_reset(self.raw) };
        self.check(rc, "reset")?;
        let rc = unsafe { sqlite::sqlite3_clear_bindings(self.raw) };
        self.check(rc, "clear bindings")
    }

    fn check(&self, rc: c_int, context: &str) -> Result<()> {
        if rc == sqlite::SQLITE_OK {
            return Ok(());
        }
        Err(PmaSqliteError::Sqlite(format!(
            "{}: {}",
            context,
            sqlite_error_message(self.db_handle(), rc)
        )))
    }

    fn db_handle(&self) -> *mut sqlite::sqlite3 {
        unsafe { sqlite::sqlite3_db_handle(self.raw) }
    }
}

impl Drop for SqliteStatement {
    fn drop(&mut self) {
        if !self.raw.is_null() {
            unsafe {
                sqlite::sqlite3_finalize(self.raw);
            }
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum Step {
    Row,
    Done,
}

fn sqlite_error_message(conn: *mut sqlite::sqlite3, rc: c_int) -> String {
    if conn.is_null() {
        return format!("sqlite error code {rc}");
    }
    let message = unsafe { sqlite::sqlite3_errmsg(conn) };
    if message.is_null() {
        return format!("sqlite error code {rc}");
    }
    let message = unsafe { CStr::from_ptr(message) }
        .to_string_lossy()
        .into_owned();
    format!("{message} (code {rc})")
}
