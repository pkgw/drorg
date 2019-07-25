// Copyright 2018 Peter Williams <peter@newton.cx>
// Licensed under the MIT License.

//! State regarding the logged-in accounts.

use chrono::{DateTime, Utc};
use serde_json;
use std::fs;
use std::path::PathBuf;
use yup_oauth2::ApplicationSecret;

use errors::{AdaptExternalResult, Result};
use google_apis::{self, CallBuilderExt, Drive};
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

    /// The identifier of this account's root folder.
    pub root_folder_id: String,

    /// The identifying ID of this account in the SQLite database.
    pub db_id: i32,

    /// The last time this account was successfully synced with the cloud.
    pub last_sync: Option<DateTime<Utc>>,
}

/// A reference to a logged-in account.
#[derive(Debug, Default)]
pub struct Account {
    /// The path to the backing file for this account.
    path: PathBuf,

    /// The persistent data.
    pub data: AccountData,
}

impl Account {
    /// Read account information.
    ///
    /// Accounts are keyed by an email address that is scanned from the
    /// account information upon first login.
    pub fn load<S: AsRef<str>>(email: S) -> Result<Account> {
        // Note that PathBuf.set_extension() will destroy, e.g., ".com" at the
        // end of an email address.
        let mut path =
            app_dirs::get_app_dir(app_dirs::AppDataType::UserData, &::APP_INFO, "accounts")?;
        let mut email_ext = email.as_ref().to_owned();
        email_ext.push_str(".json");
        path.push(&email_ext);

        let file = fs::File::open(&path)?;
        let data = serde_json::from_reader(file)?;

        Ok(Account { path, data })
    }

    /// Write the account information to the backing file.
    ///
    /// A temporary file is used in case something goes wrong while writing
    /// out the data.
    pub fn save_to_json(&self) -> Result<()> {
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
    ///
    /// Note that we do *not* save the JSON file after running this API call.
    /// The authorization may be done right as the Account is created, when it
    /// does not yet know what filename it should save itself under.
    pub fn authorize_interactively(&mut self, secret: &ApplicationSecret) -> Result<()> {
        ::google_apis::authorize_interactively(secret, &mut self.data.tokens)
    }

    /// Shim for with_drive_hub that doesn't save to JSON -- we need this to
    /// make the API call to get the email address associated with the account
    /// when setting it up, because otherwise it will fail when trying to
    /// write JSON to an as-yet-unknown path.
    fn with_drive_hub_nosave<T, F>(
        &mut self,
        secret: &ApplicationSecret,
        mut callback: F,
    ) -> Result<T>
    where
        for<'a> F: FnMut(&'a Drive<'a>) -> Result<T>,
    {
        use google_apis::get_http_client;
        use yup_oauth2::{Authenticator, DefaultAuthenticatorDelegate};

        let auth = Authenticator::new(
            secret,
            DefaultAuthenticatorDelegate,
            get_http_client()?,
            &mut self.data.tokens,
            None,
        );

        let hub = google_drive3::DriveHub::new(get_http_client()?, auth);
        callback(&hub)
    }

    /// Perform a GDrive web-API operation using this account.
    ///
    /// The callback has the signature `FnMut(hub: &Drive) -> Result<T>`. In
    /// the definition here we get to use the elusive `where for` syntax!
    pub fn with_drive_hub<T, F>(&mut self, secret: &ApplicationSecret, callback: F) -> Result<T>
    where
        for<'a> F: FnMut(&'a Drive<'a>) -> Result<T>,
    {
        let result = self.with_drive_hub_nosave(secret, callback)?;
        self.save_to_json()?;
        Ok(result)
    }

    /// Ask Google for the email address associated with this account.
    pub fn fetch_email_address(&mut self, secret: &ApplicationSecret) -> Result<String> {
        let about = self.with_drive_hub_nosave(secret, |hub| google_apis::get_about(&hub))?;
        let user = about.user.ok_or(format_err!(
            "server response did not include user information"
        ))?;
        let email = user
            .email_address
            .ok_or(format_err!("server response did not include email address"))?;

        // Kind of ugly: set the save path for our JSON file now that we know
        // what the associated email is. Then we can save the data. Note that
        // PathBuf.set_extension() will destroy, e.g., ".com" at the end of an
        // email address.

        let mut path = app_dirs::app_dir(app_dirs::AppDataType::UserData, &::APP_INFO, "accounts")?;
        let mut email_ext = email.clone();
        email_ext.push_str(".json");
        path.push(&email_ext);
        self.path = path;
        self.save_to_json()?;

        Ok(email)
    }

    /// Acquire a new token for checking for recent document changes in this account.
    pub fn acquire_change_page_token(&mut self, secret: &ApplicationSecret) -> Result<()> {
        let token = self.with_drive_hub(secret, |hub| {
            let (_resp, info) = hub
                .changes()
                .get_start_page_token()
                .default_scope()
                .doit()
                .adapt()?;
            info.start_page_token
                .ok_or(format_err!("server response did not include token"))
        })?;

        self.data.change_page_token = Some(token);
        self.save_to_json()?;
        Ok(())
    }
}

/// Get information about all of the accounts.
pub fn get_accounts() -> Result<impl Iterator<Item = Result<(String, Account)>>> {
    let path = app_dirs::app_dir(app_dirs::AppDataType::UserData, &::APP_INFO, "accounts")?;

    // Surely there's a better way to implement this ...
    Ok(fs::read_dir(path)?.filter_map(|maybe_entry| {
        match maybe_entry {
            Err(e) => Some(Err(e.into())),

            Ok(entry) => {
                let mut name = match entry.file_name().to_str() {
                    Some(n) => {
                        if n.ends_with(".json") {
                            n.to_owned()
                        } else {
                            return None;
                        }
                    }

                    None => return None,
                };

                // Safe since ".json" is always 5 bytes in UTF8:
                let new_len = name.len() - 5;
                name.truncate(new_len);
                let email = name;

                match Account::load(&email) {
                    Ok(acct) => Some(Ok((email, acct))),
                    Err(e) => Some(Err(e.into())),
                }
            }
        }
    }))
}
