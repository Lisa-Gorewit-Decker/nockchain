# C1 / M-S3 — vendor Plonky3-recursion + align the Plonky3 rev

> **Status:** DESIGN+IMPL (2026-05-18). Roadmap Phase C, the
> first/critical-path milestone (gates C2→C3→C4). Goal: an
> **audit-stable, owned recursion substrate** whose Plonky3 is
> *the same* as ai-pow-zk's, so the recursion verifier (C2/C3)
> can soundly verify our Layer-0 STARK proofs. Governed by
> `~/.claude/CLAUDE.md` R1 — the recursion substrate is
> soundness-load-bearing for the consensus certificate; staged,
> de-risk-first, no fake completion.

## 1. Problem & the forced direction

`Plonky3-recursion/` is present (13 MB, a *nested git clone* —
own `.git`, no `target/`), **untracked, not git-ignored, not a
nockchain workspace member, referenced by nothing**. It is its
own 6-member Cargo workspace (`circuit`, `circuit-prover`,
`poseidon1/2-circuit-air`, `recursion`, `test-utils`) pinning
Plonky3 at **`56952503e1401a62982ceaf952c5e4a829b61803`** via
`https://github.com/Plonky3/Plonky3` (no `.git`), 25 `p3-*` pins
in `[workspace.dependencies]` (members inherit `.workspace=true`).

ai-pow-zk's Plonky3 is **`6de5cba7c429dd89a219a3ac9849dd27b901af0e`**
(Cargo.lock-pinned; declared `{ git =
"https://github.com/Plonky3/Plonky3.git" }`, no explicit rev).

**Why the direction is forced (soundness):** the recursion
verifier must verify Layer-0 STARK proofs *produced by
ai-pow-zk's Plonky3*. Cargo treats a different `(git-url, rev)`
as a **different crate** — a `p3-field::Goldilocks` /
`p3-fri`/`p3-commit` type or (de)serialization from rev
`5695250` is not the one from `6de5cba`. A substrate mismatch
⇒ the recursion circuit cannot soundly consume our proofs (type
incompat at best; silent FRI/commitment-semantics drift at
worst). ai-pow-zk's `6de5cba` is the **fixed point**: the entire
*validated* Phase A/A-CR soundness stack, the §4.C machinery and
the 120-bit FRI sweep were built and exhaustively tested on it;
moving it would invalidate all of that. The roadmap says align
the rev *in the vendored tree*. ⇒ **Plonky3-recursion →
`(https://github.com/Plonky3/Plonky3.git, 6de5cba…)`** (both URL
*and* rev matched to ai-pow-zk so a later C2/C3 build unifies to
one Plonky3, not two).

## 2. Staged plan (R1 — commit per validated stage)

- **C1.0 — de-risk the crux (make-or-break).** Repoint all 25
  `[workspace.dependencies]` p3-* pins: rev `5695250…` →
  `6de5cba7c429dd89a219a3ac9849dd27b901af0e`, URL
  `…/Plonky3` → `…/Plonky3.git`. Then **build then test
  Plonky3-recursion standalone at the aligned rev.** Its own
  20-file test suite *is* the KAT: green ⇒ the rev alignment is
  mechanically + (per the substrate's own tests) semantically
  safe; red ⇒ Plonky3 API/semantic drift `5695250↔6de5cba` — a
  soundness-relevant signal (see §4).
- **C1.1 — vendor (track it, independently).** Strip the nested
  `Plonky3-recursion/.git` (vendor as plain in-tree source, not
  a submodule); add `"Plonky3-recursion"` to the nockchain root
  `exclude` (like `pearl`) so it stays an *independent*
  workspace and does not perturb the nockchain build; **force-
  track its `Cargo.lock`** (its `.gitignore` ignores it — but
  for an audit-stable substrate the pinned lock at `6de5cba` is
  the artifact); `git add` source (no `target/`, no `.git`).
- **C1.2 — exhaustive test + integration check.** Full
  `cargo test` of the vendored tree at the aligned rev; confirm
  the nockchain workspace is undisturbed (it excludes the tree;
  no nockchain crate depends on it yet — C2 wires that). Commit.

## 3. Exit gate

`Plonky3-recursion` is tracked in the nockchain repo, an
`exclude`d independent workspace pinned to Plonky3 `6de5cba…`
(URL `.git`, matching ai-pow-zk), **builds clean and its full
test suite passes at the aligned rev**, and `cargo metadata`
on the nockchain workspace is unchanged (no accidental
absorption). Audit-stable owned recursion substrate ready for
C2.

## 4. Escalation criterion (the goal's "hard soundness blocker")

If C1.0 shows Plonky3-recursion **cannot build / its tests fail**
at `6de5cba` due to Plonky3 drift, the options are
soundness-decisions, not mechanical: (a) minimally patch the
recursion crates for the new Plonky3 API — *soundness-sensitive*
(any edit to recursion-circuit/FRI glue must be argued, not
hacked); (b) the revs are incompatibly far ⇒ a substrate-strategy
decision (pin Plonky3 to a common rev — but ai-pow-zk's
`6de5cba` is non-negotiable without re-validating all of Phase
A/A-CR). I will first *attempt* (a) minimally and bounded; only a
genuine, characterized incompatibility (with the exact API/
semantic delta + why it can't be safely bridged) is escalated
for a decision — with specifics, per R1 (validated subset +
precise residual), never a vague stop.

## 4b. Provenance (audit-critical for C4)

Vendored from **`https://github.com/Plonky3/Plonky3-recursion`**
commit **`524665d0c2e1d294722c064786ae11dff8d9f33b`** (2026-05-17,
*"ci: split jobs for faster CI (#451)"*), upstream tree clean
(the only modification is the §1 rev/URL alignment of the 25
`[workspace.dependencies]` p3-* pins → ai-pow-zk's
`6de5cba7c429dd89a219a3ac9849dd27b901af0e` /
`https://github.com/Plonky3/Plonky3.git`). The nested `.git` is
removed on vendor (plain in-tree source, not a submodule); the
pinned `Cargo.lock` IS tracked (its upstream `.gitignore`'s
`Cargo.lock` entry removed) so the substrate is byte-frozen for
audit.

## 4c. C1.0 result — the crux PASSED (2026-05-18)

`cargo build --workspace` on the rev-aligned tree compiled
clean against `6de5cba` (`p3-fri … ?rev=6de5cba…`, `p3-circuit`,
`p3-circuit-prover`; `Finished` 11.58s, exit 0). **No Plonky3
API drift `5695250↔6de5cba`** — the §4 soundness-escalation
criterion is *not* triggered; the rev alignment is mechanically
safe. **C1 COMPLETE.** C1.2 KAT — full `cargo test --workspace` at
`6de5cba`: **15 test binaries ok, 0 failed, 0 errors** (exit 0;
the `N ignored` are upstream `#[ignore]`d benches). C1.1 vendor
done: nested `.git` stripped (plain in-tree source), pinned
`Cargo.lock` tracked (27 `6de5cba` pins — byte-frozen),
`"Plonky3-recursion"` added to the nockchain root `exclude`.
Integration verified: `cargo metadata` on the nockchain
workspace = 38 members, **Plonky3-recursion not absorbed**
(independent workspace, undisturbed); 210 source files staged,
**0 `target/`, 0 `.git/`**. Audit-stable owned recursion
substrate at ai-pow-zk's Plonky3 rev — ready for C2.

## 5. Cross-references

- `Plonky3-recursion/` (vendored tree); `crates/ai-pow-zk/
  Cargo.toml` + root `Cargo.lock` (`6de5cba…` fixed point).
- `PRODUCTION_ROADMAP.md` §2 Phase C (C1 gates C2/C3/C4).
- `PEARL_VLLM_CPU_FORK_DESIGN.md` (vendoring-as-`exclude`d-tree
  precedent: `pearl/`).
