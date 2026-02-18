# Bump PMA + Event Log + Integrity Plan

## Scope

This plan implements the target design:

1. PMA remains the primary runtime image.
2. Add an append-only SQLite event log (no truncation yet).
3. Create an immutable `epoch` snapshot on first boot (or first migration boot).
4. Maintain two rotating snapshots (`A`/`B`) that leapfrog on each snapshot update.
5. Boot by loading latest valid PMA snapshot first, then replaying missing events from SQLite.
6. Add explicit integrity metadata + verification + deterministic recovery fallbacks.

## Non-Goals (for this iteration)

1. Event log truncation/pruning.
2. Full historical compaction of event log.
3. Multi-writer PMA/event pipeline.
4. Cross-node snapshot portability guarantees.

## Current Risks To Address

1. PMA metadata is updated in-memory each event but not durably ordered with poke acknowledgement.
2. PMA GC copy path currently mutates from-space (forwarding pointers), which is unsafe for crash/fallback guarantees.
3. Boot integrity checks are shallow today (metadata checks, not full graph checks).
4. PMA persist currently disables checkpoint load path entirely instead of supporting checkpoint bootstrap-only mode.

## High-Level On-Disk Layout

Under `data_dir`:

1. `pma/epoch.pma`
2. `pma/snap-a.pma`
3. `pma/snap-b.pma`
4. `pma/epoch.manifest`
5. `pma/snap-a.manifest`
6. `pma/snap-b.manifest`
7. `event-log.sqlite3`

Keep existing `checkpoints/*.chkjam` for bootstrap compatibility (read/import path only; no background saver required).

## SQLite Schema (v1)

```sql
PRAGMA journal_mode = WAL;
PRAGMA synchronous = FULL;
PRAGMA temp_store = MEMORY;
PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS meta (
  key TEXT PRIMARY KEY,
  value BLOB NOT NULL
);

CREATE TABLE IF NOT EXISTS events (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  event_num INTEGER NOT NULL UNIQUE,           -- arvo event number
  job_jam BLOB NOT NULL,                       -- deterministic replay payload (full poke job noun)
  wire_source TEXT NOT NULL,                   -- for observability/index/debug
  wire_version INTEGER NOT NULL,
  wire_tags_json TEXT NOT NULL,                -- compact JSON array
  cause_hash BLOB NOT NULL,                    -- blake3(cause_jam) for audit
  job_hash BLOB NOT NULL,                      -- blake3(job_jam)
  created_at_ms INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS events_event_num_idx ON events(event_num);

CREATE TABLE IF NOT EXISTS snapshots (
  snapshot_id INTEGER PRIMARY KEY AUTOINCREMENT,
  kind TEXT NOT NULL CHECK(kind IN ('epoch','a','b')),
  generation INTEGER NOT NULL,                 -- monotonically increasing per kind
  state TEXT NOT NULL CHECK(state IN ('writing','ready','failed','retired')),
  event_num INTEGER NOT NULL,
  pma_path TEXT NOT NULL,
  manifest_path TEXT NOT NULL,
  alloc_words INTEGER NOT NULL,
  kernel_root_raw INTEGER NOT NULL,            -- raw noun word (must be offset-form or direct)
  cold_offset INTEGER NOT NULL,                -- u32 serialized as INTEGER
  used_blake3 BLOB NOT NULL,                   -- hash of [0, alloc_words * 8)
  structure_blake3 BLOB,                       -- optional hash over streamed jam bytes from root
  created_at_ms INTEGER NOT NULL,
  activated_at_ms INTEGER,
  base_snapshot_id INTEGER,                    -- optional lineage (epoch or previous active)
  UNIQUE(kind, generation)
);

CREATE INDEX IF NOT EXISTS snapshots_kind_gen_idx ON snapshots(kind, generation DESC);
CREATE INDEX IF NOT EXISTS snapshots_event_idx ON snapshots(event_num DESC);
```

Notes:

1. `job_jam` is required to avoid replay nondeterminism from regenerated `eny/now` fields.
2. Store snapshot rows in the same DB so event/snapshot references are naturally co-located.
3. No truncation yet: `events` grows unbounded in this phase.

## Snapshot Manifest Format

Use a versioned bincode struct (separate from SQLite row) written to `*.manifest` atomically.

Fields:

1. `magic`
2. `version`
3. `kind` (`epoch|a|b`)
4. `generation`
5. `ker_hash`
6. `event_num`
7. `pma_words`
8. `alloc_words`
9. `kernel_root_raw`
10. `cold_offset`
11. `used_blake3`
12. `structure_blake3` (optional in early rollout)
13. `created_at_ms`
14. `checksum` (blake3 over all prior fields)

Important:

1. Do not persist `pma_base` as an integrity requirement.
2. `kernel_root_raw` must be offset-form or direct atom; never pointer-form.

## Event Commit Protocol (Durability + Atomicity)

Ack contract for poke success:

1. Event applied in memory.
2. PMA updated in memory (`preserve_event_update_leftovers`).
3. Event appended and committed in SQLite (`BEGIN IMMEDIATE` + `INSERT` + `COMMIT`).
4. Only then return poke success to caller.

Rationale:

1. SQLite is the authoritative durability boundary for accepted events.
2. PMA can lag durability and be recovered by replay.

Implementation details:

1. Add event-log append in serf thread after successful event update.
2. Log the full `job_jam` event noun (not only external `wire/cause`).
3. On append failure, return poke error and do not ack success.

## Snapshot Construction (Efficient, No Second VM)

### Snapshot trigger

Periodic (configurable interval or event count threshold), executed at event boundaries in serf thread.

### Build algorithm

1. Select target slot:
   1. `a` if last written was `b`
   2. `b` if last written was `a`
2. Create target PMA file fresh.
3. Copy reachable persistent state from active PMA to target PMA.
4. Ensure copy path is non-destructive to source PMA.
5. Compute `used_blake3` over target used range `[0, alloc_words * 8)`.
6. Optionally compute `structure_blake3` by streaming jam from target root to `io::sink`.
7. `msync` target PMA used range and trailer.
8. `fsync` target PMA file descriptor.
9. Write manifest temp file -> `fsync(temp)` -> `rename` -> `fsync(parent dir)`.
10. Insert/update SQLite `snapshots` row to `ready` in a transaction.
11. Mark active snapshot in `meta` (`active_snapshot_id`, `active_kind`, `active_generation`).
12. Switch runtime PMA to target PMA only after steps 1-11 succeed.

### Epoch snapshot creation

If no snapshots exist:

1. Boot from checkpoint/state-jam/current PMA migration source.
2. Create `epoch` snapshot with event number equal to current boot event.
3. Mark it `ready` and active.
4. Subsequent snapshots rotate only between `a` and `b`.

## PMA / Copying Changes Required

1. Replace forwarding-pointer-based from-space mutation in PMA-to-PMA copy with a non-destructive copy map:
   1. Use hash map keyed by source allocation offset.
   2. Map to destination offset without mutating source noun memory.
2. Keep existing forwarding-pointer stack->PMA copy only where source is ephemeral stack data.
3. Add PMA sync methods:
   1. `sync_used_data()` (range to `alloc_words`)
   2. `sync_trailer()`
   3. `sync_file()` (`fdatasync`/`fsync` on backing file)

## Integrity Verification

## Snapshot-level checks (boot + post-write)

1. Manifest decode + checksum + version.
2. PMA trailer magic/version/data_words/alloc bounds.
3. Manifest-vs-trailer consistency (`alloc_words`, `data_words`).
4. `used_blake3` recomputation on PMA used range.
5. Root noun classification sanity:
   1. direct atom OR
   2. offset-form indirect/cell within bounds.
6. `cold_offset` in range and decodable.
7. Structural walk from root using `PmaDirectReader` (or `jam_pma_to_writer` to sink) must complete without:
   1. out-of-bounds offsets
   2. invalid noun tags
   3. forwarding pointers
   4. invalid indirect atom sizes

## SQLite checks (boot)

1. `PRAGMA quick_check`.
2. `SELECT max(event_num) FROM events`.
3. Verify monotonic continuity from chosen snapshot event:
   1. For no-truncation phase, enforce existence of all events `snapshot_event+1..max_event_num`.

## Boot Selection + Recovery/Fallback Matrix

Candidate order:

1. Active snapshot from SQLite/meta.
2. Other rotating snapshot.
3. Epoch snapshot.
4. Checkpoint/state-jam bootstrap.

For each candidate snapshot:

1. Run full integrity checks.
2. If valid:
   1. load snapshot PMA
   2. replay events `(snapshot_event+1..event_log_max)`
   3. run `preserve_event_update_leftovers`
   4. continue startup

If candidate invalid:

1. mark snapshot `failed` in DB
2. continue to next candidate

If no snapshot valid:

1. try checkpoint/state-jam bootstrap
2. create fresh epoch snapshot
3. if bootstrap unavailable, fail startup with explicit operator message

## Replay Semantics

1. Replay from persisted `job_jam` in strict `event_num ASC` order.
2. Feed replay through the same serf event path used for live events, but with a replay-specific entrypoint that:
   1. bypasses wire/cause re-synthesis randomness
   2. uses logged event noun directly
3. During replay boot mode:
   1. disable re-logging of replayed events
   2. preserve PMA periodically (batch) and once at end

## Snapshot References in Event Log (Recommended)

Keep references in DB (already via `snapshots` table):

1. `snapshots.event_num` anchors each snapshot to log position.
2. `snapshots.base_snapshot_id` tracks lineage.
3. `meta.active_snapshot_id` identifies startup default.

No per-event `snapshot_id` column needed in this phase.

## Step-by-Step Implementation Plan

## Phase 1: Foundations (Schema + Plumbing)

1. Add `event_log` module in `crates/nockapp`:
   1. SQLite open/init/migrations
   2. schema v1 from this doc
   3. typed APIs: `append_event`, `max_event_num`, `iter_events_from`, `insert_snapshot`, `set_active_snapshot`
2. Add config flags:
   1. `--event-log-path` (default `data_dir/event-log.sqlite3`)
   2. `--snapshot-interval-events` and/or `--snapshot-interval-ms`
3. Wire event log handle into serf thread state.

## Phase 2: Deterministic Event Capture

1. Capture full event noun/job as jam bytes at commit point.
2. Append event in SQLite transaction before success ack.
3. Add metrics:
   1. `event_log_append_ms`
   2. `event_log_commit_failures`

## Phase 3: Snapshot Format + Integrity

1. Implement manifest struct + encode/decode + checksum.
2. Implement PMA used-range hashing.
3. Implement structural verifier using `PmaDirectReader` traversal.
4. Add `verify_snapshot(slot)` function that executes full check suite.

## Phase 4: Non-Destructive PMA Copy

1. Replace destructive from-space forwarding-pointer PMA copy with non-mutating map-based copy.
2. Gate old path behind temporary debug flag for bisecting.
3. Add tests:
   1. source slab unchanged after copy
   2. crash-injection simulations leave source valid

## Phase 5: Snapshot Builder (Epoch + A/B)

1. Implement snapshot manager:
   1. create epoch if absent
   2. rotate between `a` and `b`
2. Implement durable write ordering:
   1. build target PMA
   2. verify target
   3. sync data + file
   4. write+fsync manifest
   5. DB update transaction
   6. switch active runtime PMA
3. Record snapshot rows in SQLite.

## Phase 6: Boot Recovery Pipeline

1. Candidate selection logic (active -> other -> epoch).
2. Full verify each candidate.
3. Replay missing events from SQLite.
4. Fallback to checkpoint/state-jam only if no valid snapshot.
5. After fallback boot, create/recreate epoch snapshot.

## Phase 7: Migration from Current PMA Persist

1. If old `*.meta` exists and new DB has no snapshots:
   1. attempt old PMA load
   2. verify and register as epoch snapshot in new DB
2. Keep checkpoint import path available regardless of PMA persistence mode.
3. Keep old files untouched until first successful new snapshot commit.

## Phase 8: Hardening + Tests

1. Unit tests:
   1. schema migration
   2. event append/replay continuity
   3. manifest hash mismatch detection
2. Integration tests:
   1. crash before/after SQLite commit
   2. crash mid-snapshot build
   3. corrupted active snapshot fallback to alternate
   4. corrupted both A/B fallback to epoch
3. Property tests:
   1. replay produces same end state as live run for deterministic test streams.

## Acknowledgement Rules

Live poke success must mean:

1. event applied in memory
2. event committed to SQLite

It does not mean PMA snapshot has advanced. PMA advancement is periodic and recoverable via replay.

## Operational Defaults

1. SQLite: WAL + FULL sync.
2. Snapshot cadence: conservative default (for example every 10k events or 10 minutes; tune later).
3. Boot always verifies snapshot before using it.
4. No log truncation in this phase.

## Future Extension (Explicitly Deferred)

1. Event log truncation to `min_ready_snapshot_event`.
2. Optional archival export before truncation.
3. Incremental snapshot delta encoding.
4. Background verification/scrubbing.
