//! The migration runner, against small made-up schemas.

use rusqlite::config::DbConfig;

use super::*;

const FAKE: &[Migration] = &[
    Migration {
        version: 1,
        sql: "CREATE TABLE notes (id INTEGER PRIMARY KEY, body TEXT NOT NULL);",
    },
    Migration {
        version: 2,
        sql: "ALTER TABLE notes ADD COLUMN pinned INTEGER NOT NULL DEFAULT 0;
                  CREATE TABLE tags (id INTEGER PRIMARY KEY, name TEXT NOT NULL);",
    },
];

fn user_version(conn: &Connection) -> i64 {
    conn.pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap()
}

fn tables(conn: &Connection) -> Vec<String> {
    let mut stmt = conn
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
        .unwrap();
    let names = stmt.query_map([], |row| row.get(0)).unwrap();
    names.map(|name| name.unwrap()).collect()
}

#[test]
fn the_real_steps_are_numbered_from_one_without_gaps() {
    for (index, step) in MIGRATIONS.iter().enumerate() {
        assert_eq!(step.version as usize, index + 1);
    }
}

#[test]
fn a_blank_database_runs_every_step_and_is_stamped() {
    let mut conn = Connection::open_in_memory().unwrap();
    assert_eq!(migrate(&mut conn, FAKE).unwrap(), 2);
    assert_eq!(user_version(&conn), 2);
    assert_eq!(tables(&conn), ["notes", "tags"]);
    assert_eq!(probe(&conn, 2).unwrap(), Probe::Catalog(2));
}

#[test]
fn an_older_database_runs_only_the_steps_it_is_missing_and_keeps_its_rows() {
    let mut conn = Connection::open_in_memory().unwrap();
    migrate(&mut conn, &FAKE[..1]).unwrap();
    conn.execute("INSERT INTO notes (body) VALUES ('kept')", [])
        .unwrap();

    assert_eq!(migrate(&mut conn, FAKE).unwrap(), 2);
    let (body, pinned): (String, i64) = conn
        .query_row("SELECT body, pinned FROM notes", [], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })
        .unwrap();
    assert_eq!((body.as_str(), pinned), ("kept", 0));
}

#[test]
fn migrating_an_up_to_date_database_changes_nothing() {
    let mut conn = Connection::open_in_memory().unwrap();
    migrate(&mut conn, FAKE).unwrap();
    let before = conn.total_changes();
    assert_eq!(migrate(&mut conn, FAKE).unwrap(), 2);
    assert_eq!(conn.total_changes(), before);
}

#[test]
fn a_failing_step_rolls_the_whole_upgrade_back() {
    let broken = [
        Migration {
            version: 1,
            sql: FAKE[0].sql,
        },
        Migration {
            version: 2,
            sql: FAKE[1].sql,
        },
        Migration {
            version: 3,
            sql: "CREATE TABLE extra (id INTEGER PRIMARY KEY);
                      INSERT INTO no_such_table VALUES (1);",
        },
    ];
    let mut conn = Connection::open_in_memory().unwrap();
    migrate(&mut conn, &broken[..1]).unwrap();
    conn.execute("INSERT INTO notes (body) VALUES ('kept')", [])
        .unwrap();

    assert!(matches!(
        migrate(&mut conn, &broken),
        Err(CatalogError::Sqlite(_))
    ));

    // Step 2 succeeded on its own, but it must not survive step 3 failing.
    assert_eq!(user_version(&conn), 1);
    assert_eq!(tables(&conn), ["notes"]);
    assert!(conn.prepare("SELECT pinned FROM notes").is_err());
    let rows: i64 = conn
        .query_row("SELECT COUNT(*) FROM notes", [], |row| row.get(0))
        .unwrap();
    assert_eq!(rows, 1);
    assert!(conn.is_autocommit());
}

#[test]
fn a_failing_first_run_leaves_the_database_blank() {
    let broken = [Migration {
        version: 1,
        sql: "CREATE TABLE notes (id INTEGER PRIMARY KEY); SELECT * FROM no_such_table;",
    }];
    let mut conn = Connection::open_in_memory().unwrap();
    assert!(migrate(&mut conn, &broken).is_err());
    assert_eq!(probe(&conn, 1).unwrap(), Probe::Blank);
}

#[test]
fn a_newer_schema_is_refused_before_anything_runs() {
    let mut conn = Connection::open_in_memory().unwrap();
    migrate(&mut conn, FAKE).unwrap();
    let before = conn.total_changes();

    let error = migrate(&mut conn, &FAKE[..1]).unwrap_err();
    assert!(matches!(
        error,
        CatalogError::NewerSchema {
            found: 2,
            supported: 1
        }
    ));
    assert_eq!(conn.total_changes(), before);
    assert_eq!(user_version(&conn), 2);
}

#[test]
fn a_database_that_belongs_to_something_else_is_refused_untouched() {
    let mut conn = Connection::open_in_memory().unwrap();
    conn.execute_batch("CREATE TABLE theirs (id INTEGER PRIMARY KEY);")
        .unwrap();
    let before = conn.total_changes();

    assert!(matches!(
        migrate(&mut conn, FAKE),
        Err(CatalogError::NotACatalog)
    ));
    assert_eq!(conn.total_changes(), before);
    assert_eq!(tables(&conn), ["theirs"]);
}

#[test]
fn a_blank_database_stamped_by_another_application_is_refused() {
    let mut conn = Connection::open_in_memory().unwrap();
    conn.pragma_update(None, "user_version", 7).unwrap();
    assert!(matches!(
        migrate(&mut conn, FAKE),
        Err(CatalogError::NotACatalog)
    ));
    assert_eq!(user_version(&conn), 7);
}

/// Two tables shaped like photos and their history: deleting a parent
/// deletes its children, and a parent's id is never used twice.
const FAMILY: Migration = Migration {
    version: 1,
    sql: "CREATE TABLE parent (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL);
              CREATE TABLE child (
                  id INTEGER PRIMARY KEY,
                  parent_id INTEGER NOT NULL REFERENCES parent(id) ON DELETE CASCADE
              );",
};

fn family_with_children() -> Connection {
    let mut conn = Connection::open_in_memory().unwrap();
    migrate(&mut conn, &[FAMILY]).unwrap();
    conn.execute_batch(
        "INSERT INTO parent (id, name) VALUES (1, 'one'), (2, 'two');
             INSERT INTO child (parent_id) VALUES (1), (1), (2);",
    )
    .unwrap();
    conn
}

fn count(conn: &Connection, table: &str) -> i64 {
    conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
        row.get(0)
    })
    .unwrap()
}

fn foreign_keys_enforced(conn: &Connection) -> bool {
    conn.pragma_query_value(None, "foreign_keys", |row| row.get(0))
        .unwrap()
}

#[test]
fn a_step_that_rebuilds_a_table_keeps_the_rows_that_refer_to_it() {
    // The standard way to change a table SQLite cannot ALTER: build the
    // new one, copy, drop the old, rename. With foreign keys enforced the
    // DROP would delete every child row; and without the two lines about
    // `sqlite_sequence` the new table would count on from its largest
    // surviving row, handing out the ids of rows removed earlier.
    let rebuild = Migration {
        version: 2,
        sql: "CREATE TABLE parent_new (
                      id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL, extra TEXT
                  );
                  INSERT INTO parent_new (id, name) SELECT id, name FROM parent;
                  DELETE FROM sqlite_sequence WHERE name = 'parent_new';
                  INSERT INTO sqlite_sequence (name, seq)
                      SELECT 'parent_new', seq FROM sqlite_sequence WHERE name = 'parent';
                  DROP TABLE parent;
                  ALTER TABLE parent_new RENAME TO parent;",
    };
    let mut conn = family_with_children();
    // As the catalog runs it: nothing in the recipe needs more than
    // defensive mode allows.
    conn.set_db_config(DbConfig::SQLITE_DBCONFIG_DEFENSIVE, true)
        .unwrap();
    conn.execute_batch(
        "INSERT INTO parent (id, name) VALUES (3, 'gone');
             DELETE FROM parent WHERE id = 3;",
    )
    .unwrap();
    assert!(foreign_keys_enforced(&conn));

    assert_eq!(migrate(&mut conn, &[FAMILY, rebuild]).unwrap(), 2);

    assert_eq!((count(&conn, "parent"), count(&conn, "child")), (2, 3));
    assert!(conn.prepare("SELECT extra FROM parent").is_ok());
    // Enforcement is back, and the children still hang off the new table.
    assert!(foreign_keys_enforced(&conn));
    conn.execute("DELETE FROM parent WHERE id = 1", []).unwrap();
    assert_eq!(count(&conn, "child"), 1);
    // The id of the parent that was removed before the upgrade stays retired.
    conn.execute("INSERT INTO parent (name) VALUES ('new')", [])
        .unwrap();
    assert_eq!(conn.last_insert_rowid(), 4);
}

#[test]
fn rows_that_were_already_orphaned_do_not_lock_the_catalog_out_of_an_upgrade() {
    // Another tool, run with enforcement off, left a child without a parent.
    let mut conn = family_with_children();
    conn.execute_batch(
        "PRAGMA foreign_keys = OFF;
             INSERT INTO child (parent_id) VALUES (99);
             PRAGMA foreign_keys = ON;",
    )
    .unwrap();
    let careless = Migration {
        version: 2,
        sql: "DELETE FROM parent WHERE id = 1;",
    };
    let harmless = Migration {
        version: 2,
        sql: "ALTER TABLE parent ADD COLUMN extra TEXT;",
    };

    // A step that breaks references of its own is still refused...
    assert!(matches!(
        migrate(&mut conn, &[FAMILY, careless]),
        Err(CatalogError::BrokenReferences { .. })
    ));
    assert_eq!(user_version(&conn), 1);
    // ...but the row that was already adrift is not the upgrade's doing.
    assert_eq!(migrate(&mut conn, &[FAMILY, harmless]).unwrap(), 2);
    assert_eq!((count(&conn, "parent"), count(&conn, "child")), (2, 4));
    assert!(foreign_keys_enforced(&conn));
}

/// A parent and a link table shaped like `photo_keywords`: its rows have
/// no rowid to tell them apart by.
const LINKS: Migration = Migration {
    version: 1,
    sql: "CREATE TABLE parent (id INTEGER PRIMARY KEY, name TEXT NOT NULL);
              CREATE TABLE link (
                  parent_id INTEGER NOT NULL REFERENCES parent(id) ON DELETE CASCADE,
                  tag TEXT NOT NULL,
                  PRIMARY KEY (parent_id, tag)
              ) WITHOUT ROWID;",
};

#[test]
fn a_new_orphan_beside_an_old_one_in_a_table_without_rowids_is_still_caught() {
    let mut conn = Connection::open_in_memory().unwrap();
    migrate(&mut conn, &[LINKS]).unwrap();
    conn.execute_batch(
        "INSERT INTO parent (id, name) VALUES (1, 'one');
             INSERT INTO link VALUES (1, 'kept');
             PRAGMA foreign_keys = OFF;
             INSERT INTO link VALUES (99, 'adrift');
             PRAGMA foreign_keys = ON;",
    )
    .unwrap();
    let careless = Migration {
        version: 2,
        sql: "DELETE FROM parent WHERE id = 1;",
    };

    assert!(matches!(
        migrate(&mut conn, &[LINKS, careless]),
        Err(CatalogError::BrokenReferences { table, .. }) if table == "link"
    ));
    assert_eq!(user_version(&conn), 1);
    assert_eq!((count(&conn, "parent"), count(&conn, "link")), (1, 2));
}

#[test]
fn a_step_that_would_orphan_rows_is_rolled_back() {
    let careless = Migration {
        version: 2,
        sql: "DELETE FROM parent WHERE id = 1;",
    };
    let mut conn = family_with_children();

    let error = migrate(&mut conn, &[FAMILY, careless]).unwrap_err();
    assert!(matches!(
        &error,
        CatalogError::BrokenReferences { table, parent } if table == "child" && parent == "parent"
    ));
    assert_eq!(user_version(&conn), 1);
    assert_eq!((count(&conn, "parent"), count(&conn, "child")), (2, 3));
    assert!(foreign_keys_enforced(&conn));
    assert!(conn.is_autocommit());
}

#[test]
fn a_failed_upgrade_leaves_the_file_on_disk_as_it_was() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("upgrade.db");
    let failing = Migration {
        version: 2,
        sql: "ALTER TABLE parent ADD COLUMN extra TEXT;
                  UPDATE parent SET name = 'changed';
                  INSERT INTO no_such_table VALUES (1);",
    };
    {
        let mut conn = Connection::open(&path).unwrap();
        migrate(&mut conn, &[FAMILY]).unwrap();
        conn.execute("INSERT INTO parent (id, name) VALUES (1, 'one')", [])
            .unwrap();
    }
    let before = std::fs::read(&path).unwrap();
    {
        let mut conn = Connection::open(&path).unwrap();
        assert!(migrate(&mut conn, &[FAMILY, failing]).is_err());
    }
    assert!(std::fs::read(&path).unwrap() == before, "the file changed");

    // A later, working build upgrades it from where it really is.
    let working = Migration {
        version: 2,
        sql: "ALTER TABLE parent ADD COLUMN extra TEXT;",
    };
    let mut conn = Connection::open(&path).unwrap();
    assert_eq!(migrate(&mut conn, &[FAMILY, working]).unwrap(), 2);
    let name: String = conn
        .query_row("SELECT name FROM parent WHERE extra IS NULL", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(name, "one");
}
