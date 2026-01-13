# PMA persistence massively drops RSS and it makes Chris paranoid

In this succession of git branches we've been working on a persistent memory arena for nockvm. This most recent successor branch was adding an option for using the PMA slab file directly for persistence instead of the checkpoints working alongside the PMA.
Seems like it's working now, but the memory statistics difference vs. the checkpointing version concerns me.

Comparison:
Metric                     PMA         Base    PMA - base
-----------------  -----------  -----------  ------------
VmRSS               1587.2 MiB  10870.4 MiB   -9283.2 MiB
VmSize             50861.9 MiB  26330.0 MiB  +24531.9 MiB
RssAnon             1177.9 MiB  10835.5 MiB   -9657.6 MiB
RssFile              409.3 MiB     34.8 MiB    +374.4 MiB
VmSwap                 0.0 MiB      0.0 MiB      +0.0 MiB
PMA map size       unavailable          n/a           n/a
PMA rss_ratio      unavailable          n/a           n/a
PMA alloc_offset       unknown          n/a           n/a
Checkpoint latest          n/a          n/a           n/a
Checkpoint total           n/a          n/a           n/a

The RSS numbers were a lot closer when the PMA version was still populating from a checkpoint. PMA used less, yes, but not 70-90% less.

I'm running both instances under Docker containers limited to 32 GiB of memory to induce uniform memory pressure on them. My concern is that the checkpointing version wasn't successfully paging out even as memory pressure mounted in the Docker container.
It would get close to 32 GiB RAM used while checkpointing several times before a checkpoint save finally triggered an OOM kill. Both instances were getting OOM killed by checkpointing.

## High‑likelihood hypotheses (and easy ways to falsify)

- H1: Checkpointing allocates huge anonymous buffers (slab copy + jam + bincode output), which are not reclaimable without swap; paging out PMA won’t help. Evidence: create_checkpoint + SerfCheckpoint::new copies kernel/cold into slabs (crates/nockapp/src/
  kernel/form.rs, crates/nockapp/src/noun/slab.rs), then jam + encode duplications (crates/nockapp/src/nockapp/save.rs). Falsify: watch /proc/self/smaps_rollup during a save; if Anonymous/Private_Dirty spikes massively while File doesn’t, this is it.
- H2: Checkpointing touches almost the entire PMA, bringing file‑backed pages resident and “active,” so the kernel can’t drop them mid‑copy. Evidence: slab copy walks the noun graph via PMA‑resolved pointers (crates/nockapp/src/noun/slab.rs), which faults pages
  in. Falsify: sample PMA residency (mincore/vmtouch) immediately before and during save; if residency jumps to near‑full during saves, that’s the trigger.
- H3: PMA pages are dirty and slow to write back, so they aren’t reclaimable under pressure. Evidence: PMA writes are MAP_SHARED and persist_metadata writes directly into the mapping without msync (crates/nockvm/rust/nockvm/src/pma.rs); repeated event updates
  dirty lots of PMA pages. Under overlayfs + cgroup limits, writeback may lag. Falsify: check cgroup memory.stat file_dirty/file_writeback and /proc/meminfo Dirty while saving.
- H4: The NockStack is anonymous mmap; once touched it’s unreclaimable without swap. Checkpointing is stack‑heavy (cold/state conversion), so anon RSS grows and sticks. Evidence: NockStack uses anon mapping (crates/nockvm/rust/nockvm/src/mem.rs) and is never
  madvise’d; cold conversion builds large stack nouns (crates/nockapp/src/kernel/form.rs). Falsify: track anon RSS after saves; if it never drops even after save completes, this is a main contributor.
- H5: Multiple large buffers exist concurrently during a save (state slab + cold slab + jam buffers + bincode envelope + fs write buffer), which can be 2–3× the live state size at peak. Evidence: SaveableCheckpoint::to_jammed_checkpoint +
  JammedCheckpointV2::encode allocate new Vecs (crates/nockapp/src/nockapp/save.rs). Falsify: use heap profiling (jemalloc/heaptrack) to see overlapping large allocations during save.

## Lower‑likelihood but plausible

- H6: Cgroup accounting charges file‑backed PMA pages against the limit, so you hit OOM before reclaim keeps up, especially while actively scanning the PMA. Falsify: inspect cgroup memory.stat file during saves; if it spikes alongside anon, this contributes.
- H7: THP / huge page effects make file‑backed or anon pages harder to reclaim under pressure. Falsify: check AnonHugePages/FilePmdMapped in /proc/self/smaps; rerun with THP disabled and compare.
- H8: Save scheduling causes long‑lived allocations because the background save holds on to slabs/jam longer than expected. Evidence: save tasks are async and gated by a mutex, but the checkpoint is created on the serf thread and then consumed later (crates/
  nockapp/src/nockapp/mod.rs). Falsify: log timestamps for checkpoint creation vs jam encode completion; if the overlap is large, peak rises.

## Why PMA‑persist looks so much lower

With PMA persistence on, you’re not doing the slab + jam path at all, so anonymous memory stays low. The file‑backed PMA can be cold, so RSS stays small even though VmSize is huge (expected for mmap).

---

Hypotheses (ranked by likelihood + ease to falsify)

1. Full‑state traversal faults PMA pages in during checkpoint.
    SaveableCheckpoint::new copies kernel_state and cold_state into NounSlab via copy_into, which walks everything and reads PMA pages (crates/nockapp/src/kernel/form.rs, crates/nockapp/src/noun/slab.rs). That alone can make a big PMA map fully resident while the slab allocations
    are growing.
    Falsify: measure RssFile/mincore just before/after a checkpoint or run with a tiny state and see if RSS stays low.
2. Jam serialization doubles memory on top of the slab copy.
    SaveableCheckpoint::to_jammed_checkpoint builds two large Vec<u8> (state + cold) while the NounSlab copies still exist (crates/nockapp/src/nockapp/save.rs). That’s another full‑state footprint, and it’s anonymous memory (hard to reclaim without swap).
    Falsify: skip jam (or stream it) and see if OOM goes away.
3. Dirty PMA pages are hard to reclaim under pressure.
    We write PMA via MmapMut without msync/madvise in production; dirty pages can’t be evicted until writeback keeps up (crates/nockvm/rust/nockvm/src/pma.rs, crates/nockvm/rust/nockvm/src/mem.rs). Under checkpoint pressure, the kernel may not reclaim fast enough.
    Falsify: watch Dirty in /proc/<pid>/smaps for the PMA mapping; add a temporary msync/madvise(MADV_PAGEOUT) and see if it changes the spike.
4. PMA size == stack size; a large stack means a huge file map.
    PMA is created with the same words as stack size (PmaConfig in crates/nockapp/src/kernel/boot.rs, crates/nockapp/src/kernel/form.rs). If you’re on --stack-size medium/large/huge, the PMA file is 16–64 GiB and a checkpoint scan can fault a lot of that into RSS.
    Falsify: run --stack-size small/normal and compare checkpoint RSS.
5. No swap in the container → anon pages can’t be reclaimed.
    NockStack + NounSlab + jam buffers are anonymous. In Docker with --memory and no swap, these can’t be evicted. Even if PMA pages are reclaimable, anon growth can still trigger OOM first.
    Falsify: enable swap (--memory-swap) or run outside Docker.
6. Allocator retention after checkpoint.
    NounSlab deallocates, but the allocator may keep large arenas around; RSS stays high across checkpoints and you eventually OOM.
    Falsify: check RSS after checkpoint completes; experiment with MALLOC_TRIM_THRESHOLD_ or jemalloc decay.
7. Cgroup page‑cache accounting.
    PMA is file‑backed and charged to the container (RssFile), so page‑cache reclaim is constrained by cgroup limits. Under heavy dirty/writeback it may OOM rather than evict.
    Falsify: inspect memory.stat for file/anon and pgscan/pgsteal during checkpoint.

If you want to dig deeper without changing behavior yet, I can help design instrumentation to answer: “how many PMA pages are resident during checkpoint” and “how big are the slab + jam allocations” so we can pinpoint the dominant factor.

Suggested next steps:

1. Capture smaps/memory.stat snapshots right before and during a checkpoint to confirm whether PMA RssFile or anon slabs/jam are the main spike.
2. Temporarily short‑circuit jam or the slab copy to isolate which phase causes the OOM.
3. Try a smaller stack size to see if the PMA mapping size is the dominant driver.
