// Copyright 2018 Peter Williams <peter@newton.cx>
// Licensed under the MIT License.

//! The local database of document information.

use chrono::{DateTime, NaiveDateTime, Utc};
use diesel::prelude::*;
use diesel::sqlite::SqliteConnection;
use google_drive3;

use app::Application;
use database;
use errors::Result;
use schema::*;

/// Connect to the Sqlite database.
pub fn get_db_connection() -> Result<SqliteConnection> {
    let p = app_dirs::get_app_dir(app_dirs::AppDataType::UserData, &super::APP_INFO, "db.sqlite")?;
    let as_str = p.to_str().ok_or_else(|| format_err!("cannot express user data path as Unicode"))?;
    Ok(SqliteConnection::establish(&as_str)?)
}


/// Superficial information about a logged-in account.
///
/// The bulk of the account state is stored in JSON files, but we use this
/// table to be able to associate documents with accounts via integers rather
/// than strings. I'm not sure if this actually helps but an email address per
/// doc seems like a bit much. Premature optimization never hurts, right?
#[derive(Clone, Debug, Identifiable, PartialEq, Queryable)]
#[table_name = "accounts"]
pub struct Account {
    /// The unique identifier of this account.
    ///
    /// This integer has no semantic meaning outside of the database.
    pub id: i32,

    /// The email address associated with this account.
    pub email: String,
}


/// Data representing a new account row to insert into the database
///
/// See the documentation for `Account` for explanations of the fields. This
/// type is different than Account in that it contains references to borrowed
/// values for non-Copy types, rather than owned values.
#[derive(Debug, PartialEq, Insertable)]
#[table_name = "accounts"]
pub struct NewAccount<'a> {
    /// The email address associated with this account.
    pub email: &'a str,
}

impl<'a> NewAccount<'a> {
    /// Create a new accountage record.
    pub fn new(email: &'a str) -> NewAccount<'a> {
        NewAccount { email }
    }
}


/// A document residing on a Google Drive.
#[derive(Clone, Debug, Identifiable, PartialEq, Queryable)]
#[table_name = "docs"]
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

    /// The last time this document was modified, without timezone information.
    ///
    /// Prefer `utc_mod_time()` to get this information with correct timezone
    /// tagging. (Namely, that this value is UTC.)
    pub modified_time: NaiveDateTime,

    /// Whether the user has starred this document.
    pub starred: bool,

    /// Whether this document is in the trash.
    pub trashed: bool,
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

    /// Discover which accounts this document is associated with.
    pub fn accounts(&self, app: &mut Application) -> Result<Vec<database::Account>> {
        use schema::account_associations::dsl::*;
        let associations = account_associations.inner_join(accounts::table)
            .filter(doc_id.eq(&self.id))
            .load::<(database::AccountAssociation, database::Account)>(&app.conn)?;
        let accounts: Vec<_> = associations.iter().map(|(_assoc, account)| account.clone()).collect();
        Ok(accounts)
    }
}


/// Data representing a new document row to insert into the database.
///
/// See the documentation for `Doc` for explanations of the fields. This type
/// is different than Doc in that it contains references to borrowed values
/// for non-Copy types, rather than owned values.
#[derive(Debug, Insertable, PartialEq)]
#[table_name = "docs"]
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


/// A parent-child relationship link between two documents.
#[derive(Debug, PartialEq, Queryable)]
pub struct Link {
    /// The account ID for which this linkage is relevant.
    pub account_id: i32,

    /// The document ID of the parent.
    pub parent_id: String,

    /// The document ID of the child.
    pub child_id: String,
}


/// Data representing a new link row to insert into the database.
///
/// See the documentation for `Link` for explanations of the fields. This type
/// is different than Link in that it contains references to borrowed values
/// for non-Copy types, rather than owned values.
#[derive(Debug, Insertable, PartialEq)]
#[table_name = "links"]
pub struct NewLink<'a> {
    /// The account ID for which this linkage is relevant.
    pub account_id: i32,

    /// The document ID of the parent.
    pub parent_id: &'a str,

    /// The document ID of the child.
    pub child_id: &'a str,
}

impl<'a> NewLink<'a> {
    /// Create a new linkage record.
    pub fn new(account_id: i32, parent_id: &'a str, child_id: &'a str) -> NewLink<'a> {
        NewLink { account_id, parent_id, child_id }
    }
}


/// A record tying a document to a logged-in account.
///
/// The same document may be associated with more than one account, so we need
/// a side table to track the associations.
#[derive(Debug, PartialEq, Queryable)]
pub struct AccountAssociation {
    /// The ID of the associated document.
    pub doc_id: String,

    /// The ID of the associated account.
    ///
    /// Each document is associated with at least one, but maybe more than
    /// one, account.
    pub account_id: i32,
}


/// Data representing a new account association row to insert into the
/// database.
///
/// See the documentation for `AccountAssociation` for explanations of the
/// fields. This type is different than AccountAssociation in that it contains
/// references to borrowed values for non-Copy types, rather than owned
/// values.
#[derive(Debug, Insertable, PartialEq)]
#[table_name = "account_associations"]
pub struct NewAccountAssociation<'a> {
    /// The ID of the associated document.
    pub doc_id: &'a str,

    /// The ID of the associated account.
    pub account_id: i32,
}

impl<'a> NewAccountAssociation<'a> {
    /// Create a new account association record.
    pub fn new(doc_id: &'a str, account_id: i32) -> NewAccountAssociation<'a> {
        NewAccountAssociation { doc_id, account_id }
    }
}


/// An document that has been entered in some list.
#[derive(Debug, PartialEq, Queryable)]
pub struct ListItem {
    /// The listing ID of this row.
    ///
    /// TBD: at the moment, there is not unique identifiable table of listing
    /// ID's. ID 0 is the list of documents that was last printed in the UI.
    pub listing_id: i32,

    /// The 0-based position of this row in the listing.
    pub position: i32,

    /// The document ID of the document in this row.
    pub doc_id: String,
}


/// In the `ListItems` table, the listing_id corresponding to the list of
/// documents that was most recently printed out in an invocation of the CLI.
pub const CLI_LAST_PRINT_ID: i32 = 0;

/// In the `ListItems` table, the listing_id corresponding to the most
/// recently probed folder. This list should contain only one item.
pub const CLI_CWD_ID: i32 = 1;


/// Data representing a new list-item row to insert into the database.
#[derive(Debug, Insertable, PartialEq)]
#[table_name = "listitems"]
pub struct NewListItem<'a> {
    /// The listing ID of this row.
    pub listing_id: i32,

    /// The 0-based position of this row in the listing.
    pub position: i32,

    /// The document ID of the document in this row.
    pub doc_id: &'a str,
}

impl<'a> NewListItem<'a> {
    /// Create a new list item record.
    pub fn new(listing_id: i32, position: i32, doc_id: &'a str) -> NewListItem<'a> {
        NewListItem { listing_id, position, doc_id }
    }
}
