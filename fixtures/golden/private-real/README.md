# Local private IELTS sample corpus

This directory contains the eight PDFs randomly sampled for Phase 0 from the
user-provided `ReadingPractice/PDF` directory. The PDFs are intentionally
ignored by Git because they are private/copyrighted regression inputs.

The reproducible selection is recorded in `fixtures/golden/manifest.json` and
the Phase 0 plan. The manifest stores normalized local paths, SHA-256 hashes,
and byte sizes; it does not store the source directory as a required runtime
dependency.
