// Copyright 2018 Peter Williams <peter@newton.cx>
// Licensed under the MIT License.

//! The main application state.

use diesel::prelude::*;
use diesel::sqlite::SqliteConnection;
use yup_oauth2::ApplicationSecret;

use accounts::{self, Account};
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
                let new_doc = database::NewDoc::from_api_object(&file)?;
                diesel::replace_into(schema::docs::table)
                    .values(&new_doc)
                    .execute(&self.conn)?;
            }

            Ok(())
        })
    }


    /// Synchronize the database with recent changes in this account.
    pub fn sync_account(&mut self, email: &str, account: &mut Account) -> Result<()> {
        let token = account.data.change_page_token.take().ok_or(
            format_err!("no change-paging token for {}", email)
        )?;

        let token = account.with_drive_hub(&self.secret, |hub| {
            let mut lister = google_apis::list_changes(
                &hub, &token,
                |call| call.spaces("drive")
                    .supports_team_drives(true)
                    .include_team_drive_items(true)
                    .include_removed(true)
                    .include_corpus_removals(true)
                    .param("fields", "changes(file(id,name,starred),fileId,removed),newStartPageToken")
            );

            for maybe_change in lister.iter() {
                use schema::docs::dsl::*;

                let change = maybe_change?;

                let removed = change.removed.unwrap_or(false);
                let file_id = (&change.file_id).as_ref().ok_or_else(
                    || format_err!("no file_id provided with change reported by the server")
                )?;

                if removed {
                    // TODO: just save a flag, or something? NOTE: Just
                    // putting a file in the trash doesn't trigger this
                    // action. The user needs to either "Delete forever" the
                    // document from their Trash; or I think this can happen
                    // if they lose access to the document.
                    diesel::delete(docs.filter(id.eq(file_id)))
                        .execute(&self.conn)?;
                } else {
                    let file = &change.file.as_ref().ok_or_else(
                        || format_err!("server reported file change but did not provide its information")
                    )?;
                    let new_doc = database::NewDoc::from_api_object(file)?;
                    diesel::replace_into(schema::docs::table)
                        .values(&new_doc)
                        .execute(&self.conn)?;
                }
            }

            Ok(lister.into_change_page_token())
        })?;

        account.data.change_page_token = Some(token);
        account.save_to_json()?;
        Ok(())
    }


    /// Synchronize the database with recent changes to all accounts.
    pub fn sync_all_accounts(&mut self) -> Result<()> {
        for maybe_info in accounts::get_accounts()? {
            let (email, mut account) = maybe_info?;
            self.sync_account(&email, &mut account)?;
        }

        Ok(())
    }
}
