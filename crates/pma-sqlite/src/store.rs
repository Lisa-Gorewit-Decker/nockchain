use std::path::{Path, PathBuf};

use diesel::connection::SimpleConnection;
use diesel::prelude::*;
use diesel::sql_types::{BigInt, Binary};
use diesel::{sql_query, RunQueryDsl};
use nockvm::ext::AtomExt;
use nockvm::mem::NockStack;
use nockvm::noun::{Atom, Noun};
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

pub struct CachedNoun {
    stack: NockStack,
    root: Noun,
}

impl CachedNoun {
    pub fn root(&self) -> Noun {
        self.root
    }

    pub fn stack(&self) -> &NockStack {
        &self.stack
    }

    pub fn stack_mut(&mut self) -> &mut NockStack {
        &mut self.stack
    }
}

pub struct SqlitePma {
    conn: SqliteConnection,
    cache: LruCache<i64, CachedNoun>,
    stats: SqlitePmaStats,
    stack_words_hint: usize,
}

impl SqlitePma {
    pub fn open(config: SqlitePmaConfig) -> Result<Self> {
        let path_str = config.path.to_str().ok_or(PmaSqliteError::InvalidPath)?;
        let mut conn = SqliteConnection::establish(path_str)?;
        conn.batch_execute(
            "PRAGMA journal_mode=WAL;\
             PRAGMA synchronous=NORMAL;\
             PRAGMA temp_store=MEMORY;\
             CREATE TABLE IF NOT EXISTS nouns (\
                 id INTEGER PRIMARY KEY AUTOINCREMENT,\
                 jam BLOB NOT NULL\
             );",
        )?;
        Ok(Self {
            conn,
            cache: LruCache::new(config.cache_capacity),
            stats: SqlitePmaStats::default(),
            stack_words_hint: config.stack_words_hint,
        })
    }

    pub fn stats(&self) -> SqlitePmaStats {
        self.stats
    }

    pub fn begin_transaction(&mut self) -> Result<()> {
        self.conn.batch_execute("BEGIN")?;
        Ok(())
    }

    pub fn commit_transaction(&mut self) -> Result<()> {
        self.conn.batch_execute("COMMIT")?;
        Ok(())
    }

    pub fn rollback_transaction(&mut self) -> Result<()> {
        self.conn.batch_execute("ROLLBACK")?;
        Ok(())
    }

    pub fn insert_noun(&mut self, stack: &mut NockStack, noun: Noun) -> Result<i64> {
        let jammed = jam_noun(stack, noun);
        self.insert_jam(&jammed)
    }

    pub fn insert_jam(&mut self, jammed: &[u8]) -> Result<i64> {
        sql_query("INSERT INTO nouns (jam) VALUES (?1)")
            .bind::<Binary, _>(jammed)
            .execute(&mut self.conn)?;
        let id: i64 = diesel::select(diesel::dsl::sql::<BigInt>("last_insert_rowid()"))
            .get_result(&mut self.conn)?;
        self.stats.inserts = self.stats.inserts.saturating_add(1);
        Ok(id)
    }

    pub fn with_cached<R, F>(&mut self, id: i64, f: F) -> Result<R>
    where
        F: FnOnce(&mut CachedNoun) -> R,
    {
        if let Some(cached) = self.cache.get_mut(&id) {
            self.stats.cache_hits = self.stats.cache_hits.saturating_add(1);
            return Ok(f(cached));
        }

        let jam = self.get_jam(id)?;
        let mut cached = decode_jam(&jam, self.stack_words_hint)?;
        let result = f(&mut cached);
        self.cache.insert(id, cached);
        self.stats.cache_misses = self.stats.cache_misses.saturating_add(1);
        Ok(result)
    }

    pub fn get_jam(&mut self, id: i64) -> Result<Vec<u8>> {
        let mut rows = sql_query("SELECT jam FROM nouns WHERE id = ?1")
            .bind::<BigInt, _>(id)
            .load::<JamRow>(&mut self.conn)?;
        let row = rows.pop().ok_or(PmaSqliteError::Missing(id))?;
        Ok(row.jam)
    }

    pub fn list_ids(&mut self) -> Result<Vec<i64>> {
        let rows = sql_query("SELECT id FROM nouns ORDER BY id").load::<IdRow>(&mut self.conn)?;
        Ok(rows.into_iter().map(|row| row.id).collect())
    }

    pub fn clear_cache(&mut self) {
        self.cache = LruCache::new(self.cache.capacity());
    }
}

#[derive(QueryableByName)]
struct JamRow {
    #[diesel(sql_type = Binary)]
    jam: Vec<u8>,
}

#[derive(QueryableByName)]
struct IdRow {
    #[diesel(sql_type = BigInt)]
    id: i64,
}

fn jam_noun(stack: &mut NockStack, noun: Noun) -> Vec<u8> {
    let jammed = serialization::jam(stack, noun);
    let space = stack.noun_space();
    jammed.in_space(&space).to_ne_bytes()
}

fn decode_jam(jam: &[u8], stack_words_hint: usize) -> Result<CachedNoun> {
    let stack_words = estimate_stack_words(jam.len(), stack_words_hint);
    let (mut stack, _) = NockStack::new_(stack_words, 0)?;
    let atom = <Atom as AtomExt>::from_bytes(&mut stack, jam);
    let root = serialization::cue(&mut stack, atom)?;
    debug!(
        "sqlite-pma: decoded jam bytes len={} into stack_words={}",
        jam.len(),
        stack_words
    );
    Ok(CachedNoun { stack, root })
}

fn estimate_stack_words(jam_len: usize, stack_words_hint: usize) -> usize {
    let jam_words = jam_len.saturating_add(7) / 8;
    let estimate = jam_words.saturating_mul(4).saturating_add(1024);
    estimate.max(stack_words_hint)
}
