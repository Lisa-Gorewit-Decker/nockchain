use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int, c_void};
use std::path::{Path, PathBuf};
use std::ptr;

use libsqlite3_sys as sqlite;
use nockvm::mem::NockStack;
use nockvm::noun::Noun;
use rkyv::api::high::to_bytes_in;
use rkyv::rancor::Error as RkyvError;
use rkyv::util::AlignedVec;

use crate::archive::{ArchivedNoun, NounArchiveBuilder};
use crate::lru::LruCache;
use crate::{PmaSqliteError, Result};

#[derive(Debug, Clone)]
pub struct SqlitePmaConfig {
    pub path: PathBuf,
    pub cache_capacity: usize,
}

impl SqlitePmaConfig {
    pub fn new<P: AsRef<Path>>(path: P) -> Self {
        Self {
            path: path.as_ref().to_path_buf(),
            cache_capacity: 1024,
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SqlitePmaStats {
    pub cache_hits: u64,
    pub cache_misses: u64,
    pub inserts: u64,
}

pub struct CachedArchive<'a> {
    root: &'a ArchivedNoun,
}

impl<'a> CachedArchive<'a> {
    fn new(root: &'a ArchivedNoun) -> Self {
        Self { root }
    }

    pub fn root(&self) -> &'a ArchivedNoun {
        self.root
    }
}

pub struct SqlitePma {
    db: SqliteDb,
    insert_stmt: SqliteStatement,
    select_stmt: SqliteStatement,
    list_stmt: SqliteStatement,
    cache: LruCache<i64, CachedEntry>,
    stats: SqlitePmaStats,
    archive_builder: NounArchiveBuilder,
    archive_scratch: AlignedVec,
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
                 archive BLOB NOT NULL\
             );",
        )?;

        let insert_stmt = db.prepare("INSERT INTO nouns (archive) VALUES (?1)")?;
        let select_stmt = db.prepare("SELECT archive FROM nouns WHERE id = ?1")?;
        let list_stmt = db.prepare("SELECT id FROM nouns ORDER BY id")?;

        let cache_capacity = config.cache_capacity.max(1);

        Ok(Self {
            db,
            insert_stmt,
            select_stmt,
            list_stmt,
            cache: LruCache::new(cache_capacity),
            stats: SqlitePmaStats::default(),
            archive_builder: NounArchiveBuilder::new(),
            archive_scratch: AlignedVec::new(),
        })
    }

    pub fn stats(&self) -> SqlitePmaStats {
        self.stats
    }

    pub fn reserve_archive_nodes(&mut self, nodes: usize) {
        self.archive_builder.reserve_nodes(nodes);
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
        self.insert_stmt.reset()?;
        let space = stack.noun_space();
        let archive = self.archive_builder.build(&space, noun)?;
        let mut scratch = std::mem::take(&mut self.archive_scratch);
        scratch.clear();
        let scratch = match to_bytes_in::<_, RkyvError>(&archive, scratch) {
            Ok(bytes) => bytes,
            Err(err) => {
                self.archive_builder.recycle(archive);
                self.archive_scratch = AlignedVec::new();
                return Err(PmaSqliteError::Archive(err.to_string()));
            }
        };
        let id = match self.insert_archive_static(scratch.as_ref()) {
            Ok(id) => id,
            Err(err) => {
                self.archive_builder.recycle(archive);
                self.archive_scratch = scratch;
                return Err(err);
            }
        };
        self.cache_archive_bytes(id, scratch.as_ref());
        self.archive_builder.recycle(archive);
        self.archive_scratch = scratch;
        Ok(id)
    }

    pub fn insert_archive(&mut self, archive: &[u8]) -> Result<i64> {
        self.insert_stmt.reset()?;
        self.insert_stmt.bind_blob(1, archive)?;
        let id = self.insert_archive_step()?;
        self.cache_archive_bytes(id, archive);
        Ok(id)
    }

    fn insert_archive_static(&mut self, archive: &[u8]) -> Result<i64> {
        self.insert_stmt.bind_blob_static(1, archive)?;
        self.insert_archive_step()
    }

    fn insert_archive_step(&mut self) -> Result<i64> {
        match self.insert_stmt.step()? {
            Step::Done => {}
            Step::Row => {
                return Err(PmaSqliteError::Sqlite(
                    "unexpected row while inserting archive".to_string(),
                ));
            }
        }
        let id = self.db.last_insert_rowid();
        self.stats.inserts = self.stats.inserts.saturating_add(1);
        Ok(id)
    }

    pub fn with_cached<R, F>(&mut self, id: i64, f: F) -> Result<R>
    where
        F: FnOnce(&CachedArchive<'_>) -> R,
    {
        if let Some(entry) = self.cache.get_mut(&id) {
            self.stats.cache_hits = self.stats.cache_hits.saturating_add(1);
            let archived = unsafe { rkyv::access_unchecked::<ArchivedNoun>(&entry.bytes) };
            let cached = CachedArchive::new(archived);
            return Ok(f(&cached));
        }

        let archive = self.get_archive(id)?;
        self.cache.insert(id, CachedEntry { bytes: archive });
        self.stats.cache_misses = self.stats.cache_misses.saturating_add(1);

        let entry = self
            .cache
            .get_mut(&id)
            .ok_or_else(|| PmaSqliteError::Missing(id))?;
        let archived = unsafe { rkyv::access_unchecked::<ArchivedNoun>(&entry.bytes) };
        let cached = CachedArchive::new(archived);
        Ok(f(&cached))
    }

    pub fn get_archive(&mut self, id: i64) -> Result<AlignedVec> {
        self.select_stmt.reset()?;
        self.select_stmt.bind_int64(1, id)?;
        let archive = match self.select_stmt.step()? {
            Step::Row => self.select_stmt.column_blob(0)?,
            Step::Done => return Err(PmaSqliteError::Missing(id)),
        };
        Ok(archive)
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
    }

    fn cache_archive_bytes(&mut self, id: i64, bytes: &[u8]) {
        if self.cache.capacity() == 0 {
            return;
        }
        let mut archive = AlignedVec::with_capacity(bytes.len());
        archive.extend_from_slice(bytes);
        self.cache.insert(id, CachedEntry { bytes: archive });
    }
}

#[derive(Debug)]
struct CachedEntry {
    bytes: AlignedVec,
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

    fn bind_blob_static(&mut self, index: i32, data: &[u8]) -> Result<()> {
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
                sqlite::SQLITE_STATIC(),
            )
        };
        self.check(rc, "bind blob static")
    }

    fn bind_int64(&mut self, index: i32, value: i64) -> Result<()> {
        let rc = unsafe { sqlite::sqlite3_bind_int64(self.raw, index, value) };
        self.check(rc, "bind int64")
    }

    fn column_blob(&self, index: i32) -> Result<AlignedVec> {
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
        let mut bytes = AlignedVec::with_capacity(slice.len());
        bytes.extend_from_slice(slice);
        Ok(bytes)
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
