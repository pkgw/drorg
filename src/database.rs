// Copyright 2018 Peter Williams <peter@newton.cx>
// Licensed under the MIT License.

//! The local database of document information.

use chrono::{DateTime, NaiveDateTime, Utc};
use diesel::prelude::*;
use diesel::sqlite::SqliteConnection;
use google_drive3;

use errors::Result;
use schema::*;

/// Connect to the Sqlite database.
pub fn get_db_connection() -> Result<SqliteConnection> {
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

    /// The MIME type of this document.
    ///
    /// Special values include:
    ///
    /// - `application/vnd.google-apps.folder`, which indicates a folder
    pub mime_type: String,

    /// Whether the user has starred this document.
    pub starred: bool,

    /// Whether this document is in the trash.
    pub trashed: bool,

    /// The last time this document was modified, without timezone information.
    ///
    /// Prefer `utc_mod_time()` to get this information with correct timezone
    /// tagging. (Namely, that this value is UTC.)
    pub modified_time: NaiveDateTime,
}

impl Doc {
    /// Retrieve the file's modification time with correct timezone information.
    pub fn utc_mod_time(&self) -> DateTime<Utc> {
        DateTime::from_utc(self.modified_time, Utc)
    }

    /// Get a URL that can be used to open this document in a browser.
    pub fn open_url(&self) -> String {
        use url::percent_encoding::{utf8_percent_encode, QUERY_ENCODE_SET};

        let mut url = hyper::Url::parse("https://drive.google.com/open").unwrap();
        let q = utf8_percent_encode(&self.id, QUERY_ENCODE_SET);
        url.set_query(Some(&format!("id={}", q)));
        url.into_string()
    }

    /// Return true if this document is a folder.
    pub fn is_folder(&self) -> bool {
        self.mime_type == "application/vnd.google-apps.folder"
    }
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

    /// The MIME type of this document.
    pub mime_type: &'a str,

    /// Whether the user has starred this document.
    pub starred: bool,

    /// Whether this document is in the trash.
    pub trashed: bool,

    /// The last time this document was modified.
    pub modified_time: NaiveDateTime,
}

impl<'a> NewDoc<'a> {
    /// Fill in a database record from a file returned by the drive3 API.
    pub fn from_api_object(file: &'a google_drive3::File) -> Result<NewDoc<'a>> {
        let id = &file.id.as_ref().ok_or_else(
            || format_err!("no ID provided with file object")
        )?;
        let name = &file.name.as_ref().map_or("???", |s| s);
        let mime_type = &file.mime_type.as_ref().map_or("", |s| s);
        let starred = file.starred.unwrap_or(false);
        let trashed = file.trashed.unwrap_or(false);
        let modified_time = file.modified_time
            .as_ref()
            .ok_or_else(|| format_err!("no modifiedTime provided with file object"))
            .and_then(|text| Ok(DateTime::parse_from_rfc3339(&text)?))?
            .naive_utc();

        Ok(NewDoc {
            id,
            name,
            mime_type,
            starred,
            trashed,
            modified_time,
        })
   }
}
