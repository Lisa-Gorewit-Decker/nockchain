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
