# Post-PMA Safe Noun API: Options and Tradeoffs

This captures the options for unifying arena-typed wrappers with `NounHandle<'a>` while keeping
lifetimes tied to provenance (specific `NounSlab`, `NockStack`, or `PMA`). It also calls out
where separate noun types remain useful and how to avoid overlapping API surfaces.

## Goals and constraints

- Make it hard (ideally impossible) to dereference or persist nouns from the wrong arena.
- Keep lifetimes tied to the specific arena instance that owns the noun.
- Avoid API surface overlap while preserving grepability (e.g., `SlabNoun`, `StackNoun`).
- Support mixed-ownership trees (stack + PMA) in a principled way.
- Minimize churn where possible.

## Option A (recommended): Generic `NounHandle<'a, K>` with type aliases

Make `NounHandle` generic over a marker type that encodes which arena(s) are valid:

```rust
pub struct NounHandle<'a, K> {
    noun: Noun,
    space: SpaceRef<'a, K>,
    _marker: PhantomData<K>,
}

pub struct SlabKind;
pub struct StackKind;
pub struct PmaKind;
pub struct StackOrPmaKind; // union for runtime tag discrimination

pub type SlabNoun<'a> = NounHandle<'a, SlabKind>;
pub type StackNoun<'a> = NounHandle<'a, StackKind>;
pub type PmaNoun<'a> = NounHandle<'a, PmaKind>;
pub type StackPmaNoun<'a> = NounHandle<'a, StackOrPmaKind>;
```

### Why it helps

- **Single handle type**: no duplicate API surface, just typed aliases.
- **Greppable**: `SlabNoun` / `StackNoun` / `PmaNoun` appear at call sites.
- **Static safety**: APIs can accept `NounHandle<'a, StackKind>` when they must not
  touch PMA pointers, or `StackOrPmaKind` when tag-based dispatch is allowed.
- **No cross-arena misuse**: `SpaceRef<'a, K>` only constructed from the correct arena.

### Implementation notes

- `SpaceRef<'a, K>` should borrow the actual arena instance (e.g., `&'a NounSlab`,
  `&'a NockStack`, `&'a Pma`), not just address ranges. That ties the lifetime and
  prevents use-after-free when the arena resets.
- Provide explicit constructors on each arena:
  - `NounSlab::handle(noun) -> SlabNoun`
  - `NockStack::handle(noun) -> StackNoun`
  - `Pma::handle(noun) -> PmaNoun`
  - `NounSpace::handle(noun) -> StackPmaNoun` (only when tag-based logic is valid)
- For conversions, require explicit copying:
  - `StackNoun::copy_into_pma(&self, &Pma) -> PmaNoun`
  - `SlabNoun::copy_into_stack(&self, &NockStack) -> StackNoun`

### When it is insufficient

If you only have a raw `Noun` + runtime `NounSpace` and cannot statically know the
arena, you must use `StackOrPmaKind`. This is still safe because tag-based lookup
is centralized and validated.

## Option B: Generic `NounSpace<K>` with typed handles

Make `NounSpace` generic and return handles bound to it:

```rust
pub struct NounSpace<'a, K> { /* arena refs */ }
pub struct NounHandle<'a, K> { noun: Noun, space: &'a NounSpace<'a, K> }
```

### Pros

- Stronger link between space and handle (single source of truth).
- Easy to restrict constructors to space capabilities.

### Cons

- More churn: `NounSpace` is currently a concrete type used broadly.
- Harder to keep `NounSpace` ergonomic in code paths that already manage multiple
  arenas (slab + stack + PMA).

## Option C: Newtype wrappers around a non-generic `NounHandle<'a>`

Keep a single handle but wrap for safety:

```rust
pub struct SlabNoun<'a>(NounHandle<'a>);
pub struct StackNoun<'a>(NounHandle<'a>);
pub struct PmaNoun<'a>(NounHandle<'a>);
```

### Pros

- Minimal disruption to existing handle code.
- Clear grepable types at call sites.

### Cons

- Boilerplate and duplicated impls (traits, methods, conversions).
- Easier to accidentally re-expose unsafe constructors.

## Option D: Fully generic `Noun<A>` (arena-typed noun)

Make nouns themselves generic over arena instance type:

```rust
pub struct Noun<A> { /* pointer + PhantomData<A> */ }
```

### Pros

- Strongest static safety.
- Prevents mixing nouns across arenas without explicit conversion.

### Cons

- Very high churn: `Noun` appears everywhere.
- Hard to represent mixed trees (stack + PMA) without a union arena type.
- Can lead to type explosion or trait indirection.

## Option E: Hybrid: generic handle + typed wrappers (best of both)

Use Option A internally, but provide thin wrappers for clarity:

```rust
pub type SlabNoun<'a> = NounHandle<'a, SlabKind>;
pub type StackNoun<'a> = NounHandle<'a, StackKind>;
pub type PmaNoun<'a> = NounHandle<'a, PmaKind>;
```

Then implement APIs in terms of `NounHandle<'a, K>` + trait bounds where needed
(`impl NounAccess for NounHandle<'a, K>`). This keeps the surface area unified while
still letting call sites declare intent.

## Key correctness points regardless of option

- **Arena provenance** must be tied to a specific instance. Passing a `NounSpace` that
  only stores ranges is insufficient; it must borrow the arena itself so lifetimes
  enforce validity.
- **Conversions should be explicit**. Copying between arenas should require a method
  with the target arena, not an implicit cast.
- **Mixed trees require a union type**. Structural sharing across stack + PMA means
  some APIs must accept `StackOrPma` handles (tag-dispatched).
- **No TLS**. All provenance is explicit through references passed to constructors.

## Why Option A is the cleanest unification

- One handle type = one API surface.
- Type aliases give you the grepability of distinct nouns without duplicating code.
- Marker types let the compiler enforce correct arena usage and disallow invalid
  conversions without explicitly copying.

If you want the least churn while gaining strong safety, Option A + E is the most
practical path.
