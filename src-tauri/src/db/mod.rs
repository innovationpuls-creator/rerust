pub mod crud;
pub mod schema;

use rusqlite::Connection;
use std::path::Path;

pub fn init_database(db_path: &Path) -> Result<Connection, Box<dyn std::error::Error>> {
    let conn = Connection::open(db_path)?;

    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;

    schema::create_tables(&conn)?;

    Ok(conn)
}
