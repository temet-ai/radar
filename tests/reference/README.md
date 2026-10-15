# The reference library

Every screen `tests/mix.rs` records, in every mix of parts, at every size and
with each icon pack, as Ratatui draws it in the mono tier
(`mix__<mix>_<what>_<w>x<h>.snap`, and `mix__joined__…`, `mix__alerts__…` for
the screens two parts make together).

They are the reference the Ink renderer is compared against. They are not
run as tests: per commit `tests/mix.rs` compares only the few recordings in
`tests/snapshots/` (GUIDE 3, the testing pyramid), each a copy of one here.

Regenerate them on purpose, when Ratatui's look changes:

```bash
scripts/check-mixes --reference
```

It runs `tests/mix.rs`'s ignored `the_reference_library` in every mix, and
writes here only the recordings whose text changed or that are new; read the
`git diff` before committing it.
