# NoteStrike source scope

This standalone candidate contains the NoteStrike Rust package, tests, and a locked dependency graph pruned from the verified Office workspace. It includes no donor source, user notebook, private data, capture, binary, build cache, or unrelated Office app. The native UI uses crates.io `eframe`; file identity checking uses pinned `same-file 1.0.6` (MIT/Unlicense upstream). The original Office workspace's DesignCraft preview candidate was not copied or claimed as implemented.

The source and tests are first-party NoteStrike changes; this checkout makes no first-party license grant. A future binary distribution needs its exact dependency and asset notice review. Windows is the tested platform. The latest accepted source and binaries are identified by `docs/note-retry-race-hashes-20261011.tsv` in the local verification workspace; that machine-specific evidence file is not part of this portable candidate.
