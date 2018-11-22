// Copyright 2018 Peter Williams <peter@newton.cx>
// Licensed under the MIT License.

//! Helpers for error handling.

use google_drive3::Error as GError;
use std::error::Error as StdError;
use std::result;


/// A result whose error type is failure::Error.
///
/// The failure crate provides this type, but under a name I don't like.
pub use failure::Fallible as Result;


/// Helper trait for Google API error conversion.
///
/// The Error type used by the Google API crates includes a
/// Box<std::error::Error>, which isn't Send, and so can't be converted into a
/// failure::Error automatically. We're not allow to add our own From impl to
/// try to fix things, so we use a small extension trait to smooth the
/// conversion process.
pub trait AdaptExternalResult {
    type OkType;

    fn adapt(self) -> Result<Self::OkType>;
}

impl<T> AdaptExternalResult for result::Result<T, GError> {
    type OkType = T;

    /// TODO: we lose all error subtype information. If we find ourselves
    /// needing to preserve this information, we can make this implementation
    /// fancier.
    fn adapt(self) -> Result<T> {
        match self {
            Ok(x) => Ok(x),
            Err(GError::HttpError(e)) => Err(e.into()),
            Err(e) => Err(format_err!("{}", e)),
        }
    }
}

impl<T> AdaptExternalResult for result::Result<T, Box<StdError>> {
    type OkType = T;

    fn adapt(self) -> Result<T> {
        match self {
            Ok(x) => Ok(x),
            Err(e) => Err(format_err!("{}", e)),
        }
    }
}
