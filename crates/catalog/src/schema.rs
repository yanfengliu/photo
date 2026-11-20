//! The catalog's schema, as the SQL of each migration step.
//!
//! Conventions: every table is `STRICT`; ids are `AUTOINCREMENT` so an id is
//! never reused (previews are cached by photo id, and a removed photo can be
//! restored under its old id); every relation is a foreign key with an
//! explicit delete rule; every foreign-key column is indexed so a cascade
//! never scans a table. Catalog-generated time stamps are microseconds since
//! the Unix epoch.

/// Schema version 1.
pub(crate) const V1: &str = "
CREATE TABLE imports (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    started_at  INTEGER NOT NULL
) STRICT;

CREATE TABLE folders (
    id    INTEGER PRIMARY KEY AUTOINCREMENT,
    path  TEXT NOT NULL UNIQUE
) STRICT;

-- One row per photo. A virtual copy is a row whose master_id points at the
-- row that owns the file; it carries its own copy of the file columns, and
-- the catalog keeps those in step with the master.
CREATE TABLE photos (
    id               INTEGER PRIMARY KEY AUTOINCREMENT,
    master_id        INTEGER REFERENCES photos(id) ON DELETE CASCADE,
    copy_name        TEXT,
    folder_id        INTEGER NOT NULL REFERENCES folders(id),
    import_id        INTEGER NOT NULL REFERENCES imports(id),
    path             TEXT NOT NULL,
    file_name        TEXT NOT NULL,
    file_name_key    TEXT NOT NULL,
    kind             INTEGER NOT NULL CHECK (kind IN (0, 1)),
    format           TEXT NOT NULL,
    file_size        INTEGER NOT NULL,
    file_mtime       INTEGER NOT NULL,
    missing          INTEGER NOT NULL DEFAULT 0 CHECK (missing IN (0, 1)),
    added_at         INTEGER NOT NULL,
    width            INTEGER NOT NULL,
    height           INTEGER NOT NULL,
    capture_time     TEXT,
    camera_make      TEXT,
    camera_model     TEXT,
    camera           TEXT,
    lens             TEXT,
    focal_length     REAL,
    focal_length_35  REAL,
    aperture         REAL,
    shutter          REAL,
    iso              INTEGER,
    exposure_bias    REAL,
    flash            INTEGER CHECK (flash IN (0, 1)),
    gps_latitude     REAL,
    gps_longitude    REAL,
    gps_altitude     REAL,
    artist           TEXT,
    file_copyright   TEXT,
    rating           INTEGER NOT NULL DEFAULT 0 CHECK (rating BETWEEN 0 AND 5),
    flag             INTEGER NOT NULL DEFAULT 0 CHECK (flag BETWEEN -1 AND 1),
    label            INTEGER NOT NULL DEFAULT 0 CHECK (label BETWEEN 0 AND 5),
    title            TEXT NOT NULL DEFAULT '',
    caption          TEXT NOT NULL DEFAULT '',
    creator          TEXT NOT NULL DEFAULT '',
    copyright        TEXT NOT NULL DEFAULT '',
    develop          TEXT,
    develop_version  INTEGER NOT NULL DEFAULT 0,
    has_edits        INTEGER NOT NULL DEFAULT 0 CHECK (has_edits IN (0, 1)),
    edited_at        INTEGER,
    CHECK ((master_id IS NULL) = (copy_name IS NULL))
) STRICT;
CREATE UNIQUE INDEX photos_master_path ON photos(path) WHERE master_id IS NULL;
CREATE INDEX photos_master ON photos(master_id);
CREATE INDEX photos_folder ON photos(folder_id);
CREATE INDEX photos_import ON photos(import_id);
CREATE INDEX photos_duplicate ON photos(file_name, file_size);
CREATE INDEX photos_capture ON photos(capture_time);

-- Develop history. At most one step per photo is the current one.
CREATE TABLE history (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    photo_id    INTEGER NOT NULL REFERENCES photos(id) ON DELETE CASCADE,
    label       TEXT NOT NULL,
    settings    TEXT NOT NULL,
    created_at  INTEGER NOT NULL,
    is_current  INTEGER NOT NULL DEFAULT 0 CHECK (is_current IN (0, 1))
) STRICT;
CREATE INDEX history_photo ON history(photo_id, id);
CREATE UNIQUE INDEX history_one_current ON history(photo_id) WHERE is_current = 1;

CREATE TABLE snapshots (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    photo_id    INTEGER NOT NULL REFERENCES photos(id) ON DELETE CASCADE,
    name        TEXT NOT NULL,
    settings    TEXT NOT NULL,
    created_at  INTEGER NOT NULL
) STRICT;
CREATE INDEX snapshots_photo ON snapshots(photo_id, id);

CREATE TABLE presets (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    group_name  TEXT NOT NULL,
    name        TEXT NOT NULL,
    settings    TEXT NOT NULL,
    groups      TEXT NOT NULL,
    created_at  INTEGER NOT NULL
) STRICT;
CREATE UNIQUE INDEX presets_name ON presets(group_name COLLATE NOCASE, name COLLATE NOCASE);

-- kind: 0 collection set, 1 collection, 2 smart collection, 3 the quick
-- collection (exactly one, created below).
CREATE TABLE collections (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    parent_id   INTEGER REFERENCES collections(id) ON DELETE CASCADE,
    kind        INTEGER NOT NULL CHECK (kind BETWEEN 0 AND 3),
    name        TEXT NOT NULL,
    rules       TEXT,
    created_at  INTEGER NOT NULL,
    CHECK ((kind = 2) = (rules IS NOT NULL))
) STRICT;
CREATE UNIQUE INDEX collections_sibling_name
    ON collections(COALESCE(parent_id, 0), name COLLATE NOCASE);
CREATE UNIQUE INDEX collections_one_quick ON collections(kind) WHERE kind = 3;
CREATE INDEX collections_parent ON collections(parent_id);

CREATE TABLE collection_photos (
    collection_id  INTEGER NOT NULL REFERENCES collections(id) ON DELETE CASCADE,
    photo_id       INTEGER NOT NULL REFERENCES photos(id) ON DELETE CASCADE,
    position       INTEGER NOT NULL,
    PRIMARY KEY (collection_id, photo_id)
) STRICT, WITHOUT ROWID;
CREATE INDEX collection_photos_photo ON collection_photos(photo_id);
CREATE INDEX collection_photos_order ON collection_photos(collection_id, position);

CREATE TABLE keywords (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    parent_id  INTEGER REFERENCES keywords(id) ON DELETE CASCADE,
    name       TEXT NOT NULL
) STRICT;
CREATE UNIQUE INDEX keywords_sibling_name
    ON keywords(COALESCE(parent_id, 0), name COLLATE NOCASE);
CREATE INDEX keywords_parent ON keywords(parent_id);
CREATE INDEX keywords_name ON keywords(name COLLATE NOCASE);

CREATE TABLE photo_keywords (
    photo_id    INTEGER NOT NULL REFERENCES photos(id) ON DELETE CASCADE,
    keyword_id  INTEGER NOT NULL REFERENCES keywords(id) ON DELETE CASCADE,
    PRIMARY KEY (photo_id, keyword_id)
) STRICT, WITHOUT ROWID;
CREATE INDEX photo_keywords_keyword ON photo_keywords(keyword_id);

CREATE TABLE settings (
    key    TEXT PRIMARY KEY NOT NULL,
    value  TEXT NOT NULL
) STRICT, WITHOUT ROWID;

-- Numbers drawn for the whole catalog, which only ever count up. Every new
-- develop_version of any photo is the next value of 'develop_version', so a
-- photo never gets a version twice, even when it is removed and restored.
CREATE TABLE counters (
    name   TEXT PRIMARY KEY NOT NULL,
    value  INTEGER NOT NULL
) STRICT, WITHOUT ROWID;
INSERT INTO counters (name, value) VALUES ('develop_version', 0);

INSERT INTO collections (kind, name, created_at) VALUES (3, 'Quick Collection', 0);
";
