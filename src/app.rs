// Copyright 2018 Peter Williams <peter@newton.cx>
// Licensed under the MIT License.

//! The main application state.

use diesel::prelude::*;
use diesel::sqlite::SqliteConnection;
use yup_oauth2::ApplicationSecret;

use accounts::Account;
use database;
use errors::Result;
use google_apis;
use schema;


/// The state of the application.
pub struct Application {
    /// The secret we use to identify this client to Google.
    pub secret: ApplicationSecret,

    /// Our connection to the database of document information.
    pub conn: SqliteConnection,
}


impl Application {
    /// Initialize the application.
    pub fn initialize() -> Result<Application> {
        let secret = google_apis::get_app_secret()?;
        let conn = database::get_db_connection()?;

        Ok(Application {
            secret,
            conn
        })
    }

    /// Fill the database with records for all of the documents associated
    /// with an account.
    pub fn import_documents(&mut self, email: &str, account: &mut Account) -> Result<()> {
        account.with_drive_hub(&self.secret, |hub| {
            for maybe_file in google_apis::list_files(&hub, |call| {
                call.spaces("drive")
                    .param("fields", "files(id,name,starred)")
            }) {
                let file = maybe_file?;

                let name = file.name.as_ref().map_or("???", |s| s);
                let id = match file.id.as_ref() {
                    Some(s) => s,
                    None => {
                        eprintln!("got a document without an ID in account {}; ignoring", email);
                        continue;
                    }
                };
                let starred = file.starred.unwrap_or(false);

                let new_doc = database::NewDoc {
                    id,
                    name,
                    starred,
                };

                diesel::replace_into(schema::docs::table)
                    .values(&new_doc)
                    .execute(&self.conn)?;
            }

            Ok(())
        })
    }
}
