Here’s a design that satisfies “no full PMA fault‑in” and “consistent snapshot,” without
forking.

Snapshot protocol (consistency)

- Quiesce writes. Pause the serf/event loop so no PMA mutations happen while snapshot metadata
is captured. This is required because O_DIRECT bypasses the page cache, so it will not see
dirty mmap pages unless you flush them first.
- Flush dirty pages. Call msync(MS_SYNC) on the PMA mapping (or dirty range) while paused. This
guarantees the on‑disk image is consistent for direct reads.
- Capture snapshot header. Record:
    - alloc_offset (word count, upper bound for reads).
    - Root nouns: kernel_state_raw and cold_offset (offset‑form noun raw values).
    - event_num, ker_hash for validation.
      This can reuse the existing PMA metadata format or a new checkpoint header.
- Resume? Only if you can guarantee PMA is immutable (no in‑place updates). If any in‑place
mutations exist, keep the pause until the stream finishes.

Streaming PMA reader (no mmap faults)

- Open the PMA file with O_DIRECT on Linux. On macOS use fcntl(F_NOCACHE) as best‑effort.
- Use a fixed aligned buffer pool (page‑size aligned, page‑size multiples) and pread/preadv2 to
read at offsets. This avoids page cache and avoids touching the mmap at all.
- Implement a tiny user‑space page cache (LRU of a few dozen pages) to avoid rereading the same
offsets. This keeps memory bounded and avoids OS page cache pollution.
- Implement a graph walker by offsets:
    - For each noun raw:
        - If direct, emit jam bits immediately.
        - If offset‑tagged indirect atom, read its header at (offset_words*8) and size at +8,
then read the data words.
        - If offset‑tagged cell, read CellMemory (metadata, head, tail) and push head/tail.
    - Use a backref map keyed by offset noun raw to preserve jam backrefs without pointer
resolution.
- Never call ptr_from_offset or NounSpace in this path.

Why this avoids hot‑paging

- O_DIRECT does not populate the page cache and never touches the mmap. The only resident
memory growth is the fixed buffer pool and backref map.
- The existing runtime mmap stays cold unless the runtime itself touches it.

Hard constraints

- O_DIRECT needs alignment: offsets and buffer lengths must be multiples of the kernel block
size (typically 4 KiB). Small reads are done by reading the containing aligned page into the
buffer and slicing.
- O_DIRECT will not see dirty mmap pages unless you msync first. That is why the pause + msync
step is non‑negotiable.
- If any PMA structures are mutated in place, you cannot resume after capturing the header. You
must hold the pause for the whole read.

If you want, I can outline a concrete “PmaDirectReader” API and the minimal metadata schema so
we can evaluate how big the pause window would be and whether it’s acceptable.
