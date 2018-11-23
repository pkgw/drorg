// Copyright 2018 Peter Williams <peter@newton.cx>
// Licensed under the MIT License.

//! The local database of document information.

use diesel::prelude::*;
use diesel::sqlite::SqliteConnection;
use failure::Error;

use super::schema::*;

/// Connect to the Sqlite database.
pub fn get_db_connection() -> Result<SqliteConnection, Error> {
    let p = app_dirs::get_app_dir(app_dirs::AppDataType::UserData, &super::APP_INFO, "db.sqlite")?;
    let as_str = p.to_str().ok_or_else(|| format_err!("cannot express user data path as Unicode"))?;
    Ok(SqliteConnection::establish(&as_str)?)
}


/// A document residing on a Google Drive.
#[derive(Queryable)]
pub struct Doc {
    /// The unique identifier of this document.
    ///
    /// This value never changes, but does not make any sense to a user.
    pub id: String,

    /// The current name of this document.
    ///
    /// This value can change.
    pub name: String,
}


/// Data representing a new document row to insert into the database.
///
/// See the documentation for `Doc` for explanations of the fields. This type
/// is different than Doc in that it contains references to borrowed values
/// for non-Copy types, rather than owned values.
#[derive(Insertable)]
#[table_name="docs"]
pub struct NewDoc<'a> {
    /// The unique identifier of this document.
    pub id: &'a str,

    /// The current name of this document.
    pub name: &'a str,
}
