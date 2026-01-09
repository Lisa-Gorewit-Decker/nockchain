# 2026-01-09 Preliminary review notes

- NounSpace seems reasonable, but file is baked into the shared `Arena` type which is used for both a stack and the PMA.
- `memfd` appears to still be getting used by default for `NockStack::new` and `new_` which seems wrong.
- The PMA currently (by design) encompasses more than just `arvo`, it includes cold/warm/open/testjets, etc. Shouldn't be problematic AFAIK.
