// Copyright 2018 Peter Williams <peter@newton.cx>
// Licensed under the MIT License.

//! State regarding the logged-in accounts.

use failure::Error;
use serde_json;
use std::collections::{HashMap, hash_map};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use gdrive::Drive;
use token_storage::SerdeMemoryStorage;


/// Get the account data structure.
pub fn get_accounts() -> Result<Accounts, Error> {
    let p = app_dirs::get_app_dir(app_dirs::AppDataType::UserData, &::APP_INFO, "accounts.json")?;
    Accounts::new(p)
}


/// Information about one-logged in Google Drive account.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Account {
    /// The OAuth2 tokens we use when issuing API calls for this account.
    ///
    /// This collection of tokens can be empty! In which case, your API calls
    /// are not going to be very successful.
    pub tokens: SerdeMemoryStorage,
}


/// A colletion of account information, serializable to and from JSON.
#[derive(Debug)]
pub struct Accounts {
    /// The path to the backing file for this collection.
    path: PathBuf,

    /// The table of accounts, keyed by an associated email address.
    ///
    /// (The way things work, the key can actually be anything, but let's not
    /// introduce confusion.)
    accounts: HashMap<String, Account>,
}

impl Accounts {
    /// Read the account information.
    ///
    /// If the specified file does not exist, the error is swallowed and an
    /// empty data structure is returned.
    pub fn new<P: AsRef<Path>>(path: P) -> Result<Accounts, Error> {
        let mut accounts = Accounts {
            path: path.as_ref().to_owned(),
            accounts: HashMap::new(),
        };

        if let Err(e) = accounts.load_from_json() {
            if e.kind() != io::ErrorKind::NotFound {
                return Err(e.into());
            }
        }

        Ok(accounts)
    }

    /// Fill in `self.accounts` with information gathered from the JSON-format
    /// file `self.path`.
    fn load_from_json(&mut self) -> Result<(), io::Error> {
        let f = fs::File::open(&self.path)?;
        self.accounts = serde_json::from_reader(f)?;
        Ok(())
    }

    /// Write the account information to the backing file.
    ///
    /// A temporary file is used in case something goes wrong while writing
    /// out the data.
    pub fn save_to_json(&self) -> Result<(), Error> {
        let mut destdir = self.path.clone();
        destdir.pop();

        let temp = tempfile::Builder::new()
            .prefix("accounts")
            .suffix(".json")
            .tempfile_in(destdir)?;

        serde_json::to_writer(&temp, &self.accounts)?;

        temp.persist(&self.path)?;
        Ok(())
    }

    /// Get a mutable reference to an account information structure.
    ///
    /// If the account's key was not present, a new empty structure is
    /// created. The structure will have an empty OAuth2 token storage, so it
    /// won't be possible to make any API calls.
    pub fn get_mut<S: AsRef<str>>(&mut self, key: S) -> &mut Account {
        // I don't like to always clone the key, but I can never figure out
        // how to avoid it without borrowck problems.
        self.accounts.entry(key.as_ref().to_owned()).or_insert(Account::default())
    }

    /// Ask the user to authorize our app to use this account, interactively.
    ///
    /// The argument `key` is only used to specify the key under which the login
    /// information is stored in the token JSON file.
    pub fn authorize_interactively<S: AsRef<str>>(&mut self, key: S) -> Result<(), Error> {
        {
            let account = self.get_mut(key);
            ::gdrive::authorize_interactively(&mut account.tokens)?;
        }

        self.save_to_json()
    }

    /// Perform a web-API operation for each logged-in account.
    ///
    /// The callback has the signature `FnMut(email: &str, hub: &Drive) ->
    /// Result<(), Error>`. In the definition here we get to use the elusive
    /// `where for` syntax!
    ///
    /// TBD: would this be better as an iterator? I had trouble implementing it
    /// that way, and there's also the matter that we want to finish up by saving
    /// the tokens, which could fail.
    pub fn foreach_hub<F>(&mut self, mut callback: F) -> Result<(), Error>
        where for<'a> F: FnMut(&'a str, &'a Drive<'a>) -> Result<(), Error>
    {
        use yup_oauth2::{Authenticator, DefaultAuthenticatorDelegate};
        use gdrive::{get_app_secret, get_http_client};

        let secret = get_app_secret()?;

        for (email, account) in &mut self.accounts {
            let auth = Authenticator::new(
                &secret,
                DefaultAuthenticatorDelegate,
                get_http_client()?,
                &mut account.tokens,
                None
            );

            let hub = google_drive3::Drive::new(get_http_client()?, auth);
            callback(&email, &hub)?;
        }

        // Our token(s) might have gotten updated.
        self.save_to_json()
    }
}

impl<'a> IntoIterator for &'a mut Accounts {
    type Item = (&'a String, &'a mut Account);
    type IntoIter = hash_map::IterMut<'a, String, Account>;

    fn into_iter(self) -> hash_map::IterMut<'a, String, Account> {
        self.accounts.iter_mut()
    }
}
