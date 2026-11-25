---
"loro-crdt": patch
---

Make a `LoroDoc` used from several threads (Rust) behave as if each operation ran alone:
- An edit while another thread runs `diff` no longer fails with `AutoCommitNotStarted`, and `apply_diff` or undo no longer fails with `EditWhenDetached`, when the doc is attached.
- Reads on other threads no longer see the older versions that `diff` and `fork_at` check the state out to; they wait until the state is back.
- `set_next_commit_message`/`set_next_commit_origin` and the edits that follow are no longer lost to, or committed by, an export, import or checkout of another thread.
- A failed `apply_diff` batch is rolled back even while other threads edit; their edits wait for it.
- New `LoroDoc::exclusive(f)` (Rust): edits and the commit made inside `f` form one change, and other threads' commits, imports and exports wait for it.
