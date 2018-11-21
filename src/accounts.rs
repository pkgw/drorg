// Copyright 2018 Peter Williams <peter@newton.cx>
// Licensed under the MIT License.

//! State regarding the logged-in accounts.

use failure::Error;
use serde_json;
use std::fs;
use std::io;
use std::path::PathBuf;
use yup_oauth2::ApplicationSecret;

use gdrive::Drive;
use token_storage::SerdeMemoryStorage;


/// Information about one logged-in Google Drive account.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct AccountData {
    /// The OAuth2 tokens we use when issuing API calls for this account.
    ///
    /// This collection of tokens can be empty! In which case, your API calls
    /// are not going to be very successful.
    pub tokens: SerdeMemoryStorage,

    /// A token used to ask the API about recent changes.
    pub change_page_token: Option<String>,
}


/// A reference to a logged-in account.
#[derive(Debug)]
pub struct Account {
    /// The path to the backing file for this account.
    path: PathBuf,

    /// The persistent data.
    data: AccountData,
}

impl Account {
    /// Read account information.
    ///
    /// If the backing JSON data file does not exist, the error is swallowed
    /// and an empty data structure is returned.
    ///
    /// Accounts should be keyed by an associated email address, although we
    /// can't technically enforce that the user specifies one as the key,
    pub fn load<S: AsRef<str>>(email: S) -> Result<Account, Error> {
        let mut path = app_dirs::get_app_dir(app_dirs::AppDataType::UserData, &::APP_INFO, "accounts")?;
        path.push(email.as_ref());
        path.set_extension("json");

        let data = match fs::File::open(&path) {
            Ok(f) => serde_json::from_reader(f)?,

            Err(e) => {
                if e.kind() != io::ErrorKind::NotFound {
                    return Err(e.into());
                }

                AccountData::default()
            }
        };

        Ok(Account { path, data })
    }

    /// Write the account information to the backing file.
    ///
    /// A temporary file is used in case something goes wrong while writing
    /// out the data.
    pub fn save_to_json(&self) -> Result<(), Error> {
        let mut destdir = self.path.clone();
        destdir.pop();

        let temp = tempfile::Builder::new()
            .prefix("account")
            .suffix(".json")
            .tempfile_in(destdir)?;

        serde_json::to_writer(&temp, &self.data)?;

        temp.persist(&self.path)?;
        Ok(())
    }

    /// Ask the user to authorize our app to use this account, interactively.
    pub fn authorize_interactively(&mut self, secret: &ApplicationSecret) -> Result<(), Error> {
        ::gdrive::authorize_interactively(secret, &mut self.data.tokens)?;
        self.save_to_json()
    }

    /// Perform a web-API operation using this account.
    ///
    /// The callback has the signature `FnMut(hub: &Drive) -> Result<T,
    /// Error>`. In the definition here we get to use the elusive `where for`
    /// syntax!
    pub fn with_hub<T, F>(&mut self, secret: &ApplicationSecret, mut callback: F) -> Result<T, Error>
        where for<'a> F: FnMut(&'a Drive<'a>) -> Result<T, Error>
    {
        use yup_oauth2::{Authenticator, DefaultAuthenticatorDelegate};
        use gdrive::get_http_client;

        let result = {
            let auth = Authenticator::new(
                secret,
                DefaultAuthenticatorDelegate,
                get_http_client()?,
                &mut self.data.tokens,
                None
            );
            let hub = google_drive3::Drive::new(get_http_client()?, auth);
            callback(&hub)?
        };

        // Our token(s) might have gotten updated.
        self.save_to_json()?;

        Ok(result)
    }

}


/// Get information about all of the accounts.
pub fn get_accounts() -> Result<impl Iterator<Item = Result<(String, Account), Error>>, Error> {
    let path = app_dirs::get_app_dir(app_dirs::AppDataType::UserData, &::APP_INFO, "accounts")?;

    // Surely there's a better way to implement this ...
    Ok(fs::read_dir(path)?.filter_map(|maybe_entry| {
        match maybe_entry {
            Err(e) => Some(Err(e.into())),

            Ok(entry) => {
                let mut name: PathBuf = entry.file_name().into();

                if let Some(ext) = name.extension() {
                    if let Some(ext_str) = ext.to_str() {
                        if ext_str == "json" {
                        } else {
                            return None;
                        }
                    } else {
                        return None;
                    }
                } else {
                    return None;
                }

                name.set_extension("");

                if let Some(email) = name.to_str() {
                    let email = email.to_owned();

                    match Account::load(&email) {
                        Ok(acct) => Some(Ok((email, acct))),
                        Err(e) => Some(Err(e.into())),
                    }
                } else {
                    None
                }
            },
        }
    }))
}
