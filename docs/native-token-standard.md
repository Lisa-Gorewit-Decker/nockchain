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

- **Atom** (`coins`): native Nicks — identical to pre-v2 behavior
- **Cell** (`[namespace=@tas amount=@]`): named token, e.g., `[%my-token 1.000]`

Discrimination is a simple `?@` test on the noun head.

## Namespace Rules

- **Type**: `@tas` (lowercase a-z, 0-9, hyphens)
- **Reserved**: `%$` (empty term, atom 0) is the implicit native Nicks namespace. Since native nicks use the atom form, `%$` never appears explicitly in an asset field.
- **Examples**: `%my-token`, `%usd-stable`, `%wrapped-btc`

## Minting

A new lock primitive `%mnt` controls token creation:

```hoon
[%mnt token-name=@tas]
```

- `token-name` must be non-empty (native nicks cannot be minted via lock — only via coinbase)
- When a note with a `%mnt` lock for namespace `%foo` is spent, output seeds may contain `[%foo amount]` gifts that exceed the input amount
- Composes with existing lock primitives (Pkh, Tim, Hax) via AND/OR in spend conditions

## Balance Conservation

For a spend of a parent note:

**Native nicks** (atom assets):
```
sum(seed gifts) + fee == parent assets
All seed gifts must be atoms.
```

**Named token** (cell assets `[ns amount]`):
```
sum(seed amounts) == parent amount     (without %mnt lock)
sum(seed amounts) >= 0                 (with %mnt lock for ns)
All seed gifts must be cells with matching namespace ns.
Fee is paid from a separate native-nicks input in the transaction.
```

## Activation

Token assets are gated by `v2-phase` in blockchain constants:

- Before `v2-phase` block height: notes/seeds with cell-form assets are rejected
- After `v2-phase`: both atom and cell forms are accepted
- No note version bump required — the union type is backward-compatible at the noun level

## Type Summary

| Type | Field | Before v2 | After v2 |
|------|-------|-----------|----------|
| `nnote-1` | `assets` | `coins` (atom) | `$@(coins token)` |
| `seed` | `gift` | `coins` (atom) | `$@(coins token)` |
| `lock-primitive` | — | `%pkh %tim %hax %brn` | `+ %mnt` |
| `blockchain-constants` | `v2-phase` | — | `@` (block height) |

## Hashing

The hashable representation depends on the asset form:

- **Atom**: `leaf+assets` (unchanged from v1)
- **Cell**: `[leaf+p.assets leaf+q.assets]` (pair of leaves)

This ensures hash stability for existing notes while providing deterministic hashing for token notes.

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

`$@(coins token)` gives zero-cost backward compatibility. Existing V1 notes with `assets=1.000` are valid without any conversion — they're already the atom case of the union.
