// Copyright 2018 Peter Williams <peter@newton.cx>
// Licensed under the MIT License.

//! Utilities for storing and using OAuth2 API tokens.

use std::collections::HashMap;
use std::io;
use yup_oauth2::{Token, TokenStorage};


/// This string is `google_drive3::Scope::Full.as_ref()`. It's convenient to
/// have this value as a global string constant rather than the above
/// expression.
pub const SCOPE: &str = "https://www.googleapis.com/auth/drive";


/// Get a ScopeList representing the scopes that we need.
///
/// This list is specific to this application.
pub fn get_scopes() -> ScopeList<'static> {
    ScopeList::new(&[SCOPE])
}


/// A helper type for yup_oauth2 scope lists, which are hashed
/// in a specific way.
pub struct ScopeList<'a> {
    pub scopes: Vec<&'a str>,
    pub hash: u64
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
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
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
