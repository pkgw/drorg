// Copyright 2018 Peter Williams <peter@newton.cx>
// Licensed under the MIT License.

//! Utilities for storing and using OAuth2 API tokens.

use failure::Error;
use serde_json;
use std::collections::{HashMap, hash_map};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use yup_oauth2::{Token, TokenStorage};


/// This string is `google_drive3::Scope::Full.as_ref()`. It's convenient to
/// have this value as a global string constant rather than the above
/// expression.
pub const SCOPE: &str = "https://www.googleapis.com/auth/drive";


/// Get a backend for storing authentication tokens.
///
/// This uses app_dirs and is specific to this application.
pub fn get_storage() -> Result<DiskMultiTokenStorage, Error> {
    let p = app_dirs::get_app_dir(app_dirs::AppDataType::UserData, &::APP_INFO, "tokens.json")?;
    DiskMultiTokenStorage::new(p)
}

/// Get a ScopeList representing the scopes that we need.
///
/// This list is specific to this application.
pub fn get_scopes() -> ScopeList<'static> {
    ScopeList::new(&[SCOPE])
}


/// A helper type for yup_oauth2 scope lists, which are hashed
/// in a specific way.
pub struct ScopeList<'a> {
    scopes: Vec<&'a str>,
    hash: u64
}

impl<'a> ScopeList<'a> {
    /// Create a new ScopeList using the specified list of scope URLs.
    pub fn new<I, T>(scopes: I) -> ScopeList<'a>
        where T: AsRef<str> + Ord + 'a,
              I: IntoIterator<Item = &'a T>
    {
        // This copy-pastes the logic from yup_oauth2.
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut sv: Vec<&str> = scopes.into_iter()
            .map(|s| s.as_ref())
            .collect::<Vec<&str>>();
        sv.sort();
        let mut sh = DefaultHasher::new();
        &sv.hash(&mut sh);
        let sv = sv;

        ScopeList { scopes: sv, hash: sh.finish() }
    }

    /// Get a reference to the list of scope URLs.
    ///
    /// This is suitable for passing to `yup_oauth2::GetToken::token()` and
    /// `yup_oauth2::TokenStorage::set()`.
    pub fn as_vec(&self) -> &Vec<&'a str> {
        &self.scopes
    }
}


/// This is just yup_oauth2::MemoryStorage, but implementing Serialize and
/// Deserialize. And Debug.
#[derive(Debug, Default, Deserialize, Serialize)]
pub struct SerdeMemoryStorage {
    pub tokens: HashMap<u64, Token>,
}

impl TokenStorage for SerdeMemoryStorage {
    // This type must implement std::error::Error, which failure::Error
    // actually doesn't. So for convenience we use io::Error.
    type Error = io::Error;

    fn set(&mut self, scope_hash: u64, _: &Vec<&str>, token: Option<Token>) -> Result<(), io::Error> {
        match token {
            Some(t) => self.tokens.insert(scope_hash, t),
            None => self.tokens.remove(&scope_hash),
        };
        Ok(())
    }

    fn get(&self, scope_hash: u64, _: &Vec<&str>) -> Result<Option<Token>, io::Error> {
        match self.tokens.get(&scope_hash) {
            Some(t) => Ok(Some(t.clone())),
            None => Ok(None),
        }
    }
}

// I've neer been quite clear on when/why you sometimes (always?) need to
// re-implement trait X for type &T when it's implemented for T ...
impl<'a> TokenStorage for &'a mut SerdeMemoryStorage {
    type Error = io::Error;

    fn set(&mut self, hash: u64, scopes: &Vec<&str>, token: Option<Token>) -> Result<(), io::Error> {
        (**self).set(hash, scopes, token)
    }

    fn get(&self, hash: u64, scopes: &Vec<&str>) -> Result<Option<Token>, io::Error> {
        (**self).get(hash, scopes)
    }
}


/// A way to serialize multiple sets of OAuth tokens all at once. This is
/// basically yup_oauth2::DiskTokenStorage, but with an extra layer of hashmap
/// at the top.
///
/// If you add new tokens to the storage, you must remember to write back out
/// the updated file with `save_to_json()`.
#[derive(Debug, Deserialize, Serialize)]
pub struct DiskMultiTokenStorage {
    path: PathBuf,
    accounts: HashMap<String, SerdeMemoryStorage>,
}

impl DiskMultiTokenStorage {
    /// Create a new storage linked to the specified path.
    ///
    /// Data in the storage are loaded up creation. If the specified file does
    /// not exist, the error is swallowed.
    pub fn new<P: AsRef<Path>>(path: P) -> Result<DiskMultiTokenStorage, Error> {
        let mut dmts = DiskMultiTokenStorage {
            path: path.as_ref().to_owned(),
            accounts: HashMap::new(),
        };

        if let Err(e) = dmts.load_from_json() {
            if e.kind() != io::ErrorKind::NotFound {
                return Err(e.into());
            }
        }

        Ok(dmts)
    }

    /// Fill in `self.accounts` with information gathered from the JSON-format
    /// file `self.path`.
    fn load_from_json(&mut self) -> Result<(), io::Error> {
        let f = fs::File::open(&self.path)?;
        self.accounts = serde_json::from_reader(f)?;
        Ok(())
    }

    /// Write out the current stored token information to the backing file.
    pub fn save_to_json(&self) -> Result<(), io::Error> {
        let mut destdir = self.path.clone();
        destdir.pop();

        let temp = tempfile::Builder::new()
            .prefix("tokens")
            .suffix(".json")
            .tempfile_in(destdir)?;

        serde_json::to_writer(&temp, &self.accounts)?;

        temp.persist(&self.path)?;
        Ok(())
    }

    /// Get a mutable reference to the token storage associated with the given key.
    ///
    /// If the key was not present, a new empty token storage is created.
    pub fn get_mut<S: AsRef<str>>(&mut self, key: S) -> &mut SerdeMemoryStorage {
        // I don't like cloning the key by default, but I can never figure out
        // how to avoid it without borrowck problems.
        self.accounts.entry(key.as_ref().to_owned()).or_insert(SerdeMemoryStorage::default())
    }

    /// Insert a new token into the storage.
    pub fn add_token<'a, S: AsRef<str>>(
        &mut self, scopes: &ScopeList<'a>, key: S, token: Token
    ) -> Result<(), Error>
    {
        let storage = self.get_mut(key);
        Ok(storage.set(scopes.hash, &scopes.scopes, Some(token))?)
    }
}

impl<'a> IntoIterator for &'a mut DiskMultiTokenStorage {
    type Item = (&'a String, &'a mut SerdeMemoryStorage);
    type IntoIter = hash_map::IterMut<'a, String, SerdeMemoryStorage>;

    fn into_iter(self) -> hash_map::IterMut<'a, String, SerdeMemoryStorage> {
        self.accounts.iter_mut()
    }
}
