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


#[derive(Queryable)]
pub struct Doc {
    pub id: String,
    pub name: String,
}

#[derive(Insertable)]
#[table_name="docs"]
pub struct NewDoc<'a> {
    pub id: &'a str,
    pub name: &'a str,
}

