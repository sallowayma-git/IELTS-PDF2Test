# Local private IELTS sample corpus

This directory may contain multiple local private corpus layers. The
authoritative Phase 0 plan corpus is the exact eight-entry selection in
`fixtures/golden/manifest.json#requiredPrivateCorpus`; the directory's PDF
count is not a contract. Additional fixed-seed samples may coexist here as an
extended regression corpus. All PDFs are intentionally ignored by Git because
they are private/copyrighted regression inputs.

The manifest stores normalized local paths, SHA-256 hashes, and byte sizes for
each selected fixture; it does not store the source directory as a required
runtime dependency.
