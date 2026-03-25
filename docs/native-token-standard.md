# Native Token Standard with ASCII Namespace

## Overview

Nockchain supports user-defined tokens via a tagged asset type on notes and seeds. Each note carries exactly **one** token type, identified by an ASCII namespace (`@tas`). Native nicks remain a bare atom for full backward compatibility.

## Asset Type

```hoon
+$  token  (pair @tas @)
```

The `assets` field on notes and `gift` field on seeds use the union type:

```hoon
$@(coins token)
```

- **Atom** (`coins`): native Nicks -- identical to pre-v2 behavior
- **Cell** (`[namespace=@tas amount=@]`): named token, e.g., `[%my-token 1.000]`

Discrimination is a simple `?@` test on the noun head.

## Namespace Rules

### Format Validation (protocol-enforced)

- **Type**: `@tas` (lowercase a-z, 0-9, hyphens)
- **Length**: 3--32 characters
- **Start**: must begin with a letter (a-z)
- **End**: must end with a letter or digit
- **No consecutive hyphens** (`--` forbidden)

Validated by `++valid-token-name` gate in tx-engine-1.hoon. Called from `based:lock-primitive` for `%mnt`.

### Reserved Namespaces (deny-list)

| Name | Reason |
|------|--------|
| `%$` (empty) | Implicit native nicks; excluded by format rules (length < 3) |
| `%nock` | Protocol-reserved |
| `%nick` / `%nicks` | Avoid currency confusion |

Expandable only via hard fork. Small set (<10).

### Registration: Burn-to-Register with Tiered Fees

The first time a namespace appears in a `%mnt` lock on-chain, the transaction must include a burn output (note locked with `%brn`) of at least `tier-fee(len)` nicks.

**Tier structure** (configurable via `blockchain-constants`):

```hoon
::  in blockchain-constants default:
namespace-fees=[len-3=(bex 30) len-4=(bex 26) len-5=(bex 22) len-6-plus=(bex 18)]

::  type spec:
+$  namespace-fee-tiers
  $:  len-3=@
      len-4=@
      len-5=@
      len-6-plus=@
  ==
```

| Name length | Default burn cost (nicks) | Rationale |
|-------------|--------------------------|-----------|
| 3 chars | 2^30 (~1B nicks) | Premium short names |
| 4 chars | 2^26 (~67M nicks) | Expensive but accessible |
| 5 chars | 2^22 (~4M nicks) | Moderate |
| 6+ chars | 2^18 (~262K nicks) | Low barrier |

Tiers are adjustable via hard fork by changing `blockchain-constants` defaults, without code changes.

**Properties:**
- Deflationary pressure on nicks (burned, not transferred)
- Shorter names are scarcer = more expensive
- Permissionless -- anyone can register, just pay the cost
- One-time cost per namespace (not per mint)

**Validator state:** `registered-namespaces: (z-set @tas)` tracks which namespaces have been registered. Checked during spend validation -- if the `%mnt` namespace is not in the set, require the burn output and add it.

## Minting

### Lock Primitive with Embedded Config

A new lock primitive `%mnt` controls token creation with embedded configuration:

```hoon
::  $supply-policy: how the token's supply is governed
::  The policy mode is immutable -- committed at token creation.
+$  supply-policy
  $%  [%capped cap=@]       ::  hard cap; can only decrease, never removed
      [%managed cap=@]      ::  issuer-managed cap; can increase or decrease
      [%unlimited ~]        ::  no supply cap; mint freely
  ==

+$  mint-config
  $:  supply=supply-policy
      divisibility=@        ::  minimum unit; amounts must be multiples. >=1
  ==

[%mnt token-name=@tas config=mint-config]
```

**Token archetypes and their supply policies:**

| Archetype | Policy | Example |
|-----------|--------|---------|
| Governance token | `[%capped 10.000.000]` | Fixed supply, mint once, close the mint |
| Bitcoin-like | `[%capped 21.000.000]` | Hard cap, gradual emission |
| Stablecoin | `[%managed 1.000.000]` | Issuer raises/lowers cap as reserves change |
| Reward/utility | `[%unlimited ~]` | Continuous minting, no cap |

- `token-name` must pass `++valid-token-name` (non-empty, 3-32 chars, letter-start, no `--`, not reserved)
- `divisibility` must be >= 1
- Config is hashed into the lock root, making it a cryptographic commitment
- When a note with a `%mnt` lock for namespace `%foo` is spent, output seeds may contain `[%foo amount]` gifts that exceed the input amount
- Composes with existing lock primitives (Pkh, Tim, Hax) via AND/OR in spend conditions

### Lock Composition Patterns

| Pattern | Lock Structure | Effect |
|---------|---------------|--------|
| Authorized minter | `[[%pkh ...] [%mnt %foo ...]]` | Only key-holder can mint %foo |
| Time-limited mint | `[[%pkh ...] [%mnt %foo ...] [%tim ...]]` | Minting only in time window |
| Close the mint | Spend `%mnt` note without successor | No more minting possible |
| Token freeze | `[[%pkh owner] [%tim min=height]]` on token note | Frozen until block height |
| Clawback | Lock tree: `[%pkh owner]` OR `[%pkh issuer]` | Issuer retains spend path |
| Burn tokens | Send to `[%brn ~]` lock | Unspendable = burned |

## Config Mutability: UTXO Chain Model

The "current" config for a namespace is the config on the **unspent** `%mnt` note. Config evolves when the `%mnt` note is spent and a successor is created:

```
[%mnt %foo [%managed cap=1M] div=100]  -- note A (UTXO)
        |  (spend A, create B)
[%mnt %foo [%managed cap=2M] div=100]  -- note B (stablecoin issuer raises cap)
```

### Transition Rules (protocol-enforced during spend validation)

| Field | Mutability | Rule |
|-------|-----------|------|
| `token-name` | Immutable | Must match parent (same namespace) |
| `divisibility` | Immutable | Must equal parent's divisibility (changing breaks existing amounts) |
| `supply` policy mode | Immutable | `%capped` stays `%capped`, `%managed` stays `%managed`, etc. |
| `cap` (when `%capped`) | Monotonic decrease | Can only decrease or stay same. Cannot become 0. |
| `cap` (when `%managed`) | Free | Issuer can increase or decrease. Cannot become 0 (use `%unlimited` instead). |
| `%unlimited` | N/A | No cap to change |

**Key difference between `%capped` and `%managed`:** Both enforce the cap at mint time (cumulative supply cannot exceed cap). But `%capped` tokens promise the cap will never increase (monotonic decrease on config transition), while `%managed` tokens allow the issuer to raise the cap via a successor `%mnt` note. Both are fully auditable on-chain -- the full history of cap changes is visible in the UTXO chain.

**Successor detection:** When spending a note whose spend-condition contains `[%mnt ns ...]`, if any output seed's lock contains `[%mnt ns ...]` for the same namespace, validate the config transition.

**No successor = authority expires.** If you spend a `%mnt` note without creating a successor, the minting authority for that namespace is gone. The existing tokens remain valid (they're just notes with `[ns amount]` assets), but no new tokens can be minted. This is a feature -- it enables "close the mint" by simply not creating a successor.

## Supply Cap Enforcement

Protocol-enforced. Validators track cumulative minted per namespace in `namespace-supply: (z-map @tas @)`. Each mint operation updates the running total.

- `%capped` and `%managed`: reject if `current-supply + new-mint > cap`
- `%unlimited`: no cap check
- State cost is one atom per namespace -- trivial
- Users get cryptographic supply guarantees

## Balance Conservation

For a spend of a parent note:

**Native nicks** (atom assets):
```
sum(seed gifts) + fee == parent assets
All seed gifts must be atoms.
```

**Named token** (cell assets `[ns amount]`):
```
With %mnt lock for ns in spend-condition:
  output amounts must be multiples of divisibility
  supply cap check (per supply-policy):
    %capped/%managed: current-supply + new-mint <= cap
    %unlimited: no check
  fee is paid from a separate native-nicks input

Without %mnt lock (conservation):
  sum(seed amounts) == parent amount
  all seed gifts must be cells with matching namespace ns
  fee is paid from a separate native-nicks input
```

## Validator State

Two new pieces of state tracked by validators (alongside the UTXO set):

1. **`registered-namespaces: (z-set @tas)`** -- set of namespaces that have been registered via burn
2. **`namespace-supply: (z-map @tas @)`** -- cumulative minted supply per namespace

Both are updated during block validation and must be deterministic.

## Activation

Token assets are gated by `v2-phase` in blockchain constants:

- Before `v2-phase` block height: notes/seeds with cell-form assets are rejected
- Before `v2-phase`: transactions with `%mnt` locks are rejected
- After `v2-phase`: both atom and cell forms accepted
- No note version bump required -- the union type is backward-compatible at the noun level

## Type Summary

| Type | Field | Before v2 | After v2 |
|------|-------|-----------|----------|
| `nnote-1` | `assets` | `coins` (atom) | `$@(coins token)` |
| `seed` | `gift` | `coins` (atom) | `$@(coins token)` |
| `lock-primitive` | -- | `%pkh %tim %hax %brn` | `+ [%mnt @tas mint-config]` |
| `blockchain-constants` | `v2-phase` | -- | `@` (block height) |
| `blockchain-constants` | `namespace-fees` | -- | `namespace-fee-tiers` |

## Hashing

The hashable representation depends on the asset form:

- **Atom**: `leaf+assets` (unchanged from v1)
- **Cell**: `[leaf+p.assets leaf+q.assets]` (pair of leaves)

For `%mnt` lock primitive:
- `[leaf+%mnt leaf+token-name leaf+policy-tag leaf+cap-or-0 leaf+divisibility]`

This ensures hash stability for existing notes while providing deterministic hashing for token notes and mint configs.

## Design Rationale

### Why not NoteData?

NoteData (`z-map @tas *`) is opaque to the validation engine. Token balances require transparent conservation checks during consensus validation. A first-class field keeps validation clean.

### Why pair instead of map?

A `(pair @tas @)` limits each note to a single token type. This:
- Keeps the type simple and the conservation check trivial
- Aligns with the UTXO model (one asset per output)
- Avoids dust-token attacks (no multi-namespace notes to bloat)
- Multi-asset transactions use multiple inputs/outputs naturally

### Why union with atom?

`$@(coins token)` gives zero-cost backward compatibility. Existing V1 notes with `assets=1.000` are valid without any conversion -- they're already the atom case of the union.

### Why burn-to-register?

Burning nicks to register a namespace creates deflationary pressure and deters name squatting without requiring a centralized registry. Tiered fees by name length make short (premium) names expensive while keeping longer names accessible. The fee tiers are configurable via `blockchain-constants` so they can be adjusted via hard fork as the economy evolves.

### Why embedded mint-config?

Embedding `supply-policy` and `divisibility` in the `%mnt` lock primitive (rather than in note-data) means:
- Config is hashed into the lock root -- it's a cryptographic commitment
- Validators can enforce rules without inspecting opaque note-data
- Config is immutable per lock-root (changing config = new lock root = new authority)
- The UTXO chain model gives natural mutability: spend the old authority, create a new one

### Why supply-policy modes?

Different tokens need different supply disciplines:
- **Governance tokens** need a hard cap that can never increase (`%capped`) -- holders trust the supply commitment
- **Stablecoins** need elastic supply (`%managed`) -- the issuer must raise the cap as new reserves back more tokens
- **Reward tokens** need unlimited minting (`%unlimited`) -- no cap overhead

Making the policy mode immutable (committed at creation) while allowing cap changes within the mode's rules gives both flexibility and strong guarantees. A `%capped` token can never become `%managed` -- that promise is cryptographic.

### Why protocol-enforced supply caps?

Application-layer supply tracking (wallets/indexers only) means supply caps are just promises. Protocol enforcement (validators track cumulative supply per namespace) gives cryptographic guarantees. The state cost is one atom per namespace -- trivial.
