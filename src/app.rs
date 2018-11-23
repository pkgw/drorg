// Copyright 2018 Peter Williams <peter@newton.cx>
// Licensed under the MIT License.

//! The main application state.

use diesel::sqlite::SqliteConnection;
use yup_oauth2::ApplicationSecret;

use database::get_db_connection;
use errors::Result;
use google_apis::get_app_secret;


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
        let secret = get_app_secret()?;
        let conn = get_db_connection()?;

        Ok(Application {
            secret,
            conn
        })
    }
}
