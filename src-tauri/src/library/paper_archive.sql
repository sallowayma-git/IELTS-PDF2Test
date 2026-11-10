CREATE TABLE IF NOT EXISTS papers (
 paper_id TEXT PRIMARY KEY, batch_id TEXT NOT NULL, published_at TEXT NOT NULL, superseded_at TEXT,
 destination TEXT NOT NULL, pointer_json TEXT NOT NULL, status TEXT NOT NULL CHECK(status IN ('current','superseded')),
 archive_state TEXT NOT NULL DEFAULT 'confirmed' CHECK(archive_state IN ('confirmed','unconfirmed'))
);
CREATE INDEX IF NOT EXISTS idx_paper_destination ON papers(destination,status);
CREATE TABLE IF NOT EXISTS paper_items (
 paper_id TEXT NOT NULL REFERENCES papers(paper_id), modality TEXT NOT NULL, part TEXT NOT NULL,
 exam_id TEXT NOT NULL, library_item_id TEXT, edit_version INTEGER, title TEXT NOT NULL,
 script_sha256 TEXT NOT NULL, runtime_sha256 TEXT NOT NULL, asset_manifest_sha256 TEXT,
 PRIMARY KEY(paper_id,modality,part,exam_id)
);
CREATE TABLE IF NOT EXISTS item_content_snapshots (
 exam_id TEXT NOT NULL, runtime_sha256 TEXT NOT NULL, runtime_source_json TEXT NOT NULL,
 created_at TEXT NOT NULL, PRIMARY KEY(exam_id,runtime_sha256)
);
CREATE TABLE IF NOT EXISTS paper_archive_destinations(destination TEXT PRIMARY KEY);
