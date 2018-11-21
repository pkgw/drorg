// Copyright 2018 Peter Williams <peter@newton.cx>
// Licensed under the MIT License.

//! Our interface with the Google Drive web API.

use failure::Error;
use hyper::Client;
use std::fs;
use yup_oauth2::{
    Authenticator as YupAuthenticator, ApplicationSecret,
    ConsoleApplicationSecret, DefaultAuthenticatorDelegate,
    FlowType, GetToken, NullStorage,
};

use token_storage::{SCOPE, SerdeMemoryStorage, get_scopes, get_storage};

/// The app-specific token storage type.
pub type TokenStore<'a> = &'a mut SerdeMemoryStorage;

/// The app-specific authenticator type.
pub type Authenticator<'a> = YupAuthenticator<DefaultAuthenticatorDelegate,
                                              TokenStore<'a>,
                                              Client>;

/// The app-specific Drive API "hub" type.
pub type Drive<'a> = google_drive3::Drive<Client, Authenticator<'a>>;


/// Get the "application secret" needed to authenticate against Google APIs.
///
/// TODO: can we automate the creation and retrieval of this file? That would
/// be cool but not something to spend time on right now.
///
/// On Linux the desired filepath is `~/.config/goodriver/client_id.json`.
fn get_app_secret() -> Result<ApplicationSecret, Error> {
    let p = app_dirs::get_app_dir(app_dirs::AppDataType::UserConfig, &::APP_INFO, "client_id.json")?;
    let f = fs::File::open(p)?;
    let cfg: ConsoleApplicationSecret = serde_json::from_reader(f)?;
    cfg.installed.ok_or_else(|| format_err!("no installed-application secret"))
}


/// Get an HTTP client with all the bells and whistles we need.
fn get_http_client() -> Result<hyper::Client, Error> {
    Ok(hyper::Client::with_connector(
        hyper::net::HttpsConnector::new(
            hyper_native_tls::NativeTlsClient::new()?
        )
    ))
}


/// Helper trait for generic operations on API calls
///
/// Every API call implements these features, but not as a trait, so we can't
/// access them generically without adding a helper trait.
pub trait CallBuilderExt {
    /// Set the authorization scope to be used for this API call.
    ///
    /// This just wraps the `add_scope` call implemented for every CallBuilder
    /// type. Note that the auto-generated documentation for those functions
    /// is not accurate.
    fn set_scope<S: AsRef<str>>(self, scope: S) -> Self;
}

macro_rules! impl_call_builder_ext {
    ($type:ty) => {
        impl<'a, C, A> CallBuilderExt for $type
            where C: ::std::borrow::BorrowMut<hyper::Client>, A: GetToken
        {
            fn set_scope<S: AsRef<str>>(self, scope: S) -> Self {
                // I don't know why the compiler needs me to spell out the type here ...
                self.add_scope::<Option<S>, S>(Some(scope))
            }
        }
    }
}

impl_call_builder_ext!(google_drive3::ChangeGetStartPageTokenCall<'a, C, A>);
impl_call_builder_ext!(google_drive3::FileListCall<'a, C, A>);


/// Ask the user to authorize our app to use an account, interactively.
///
/// The argument `key` is only used to specify the key under which the login
/// information is stored in the token JSON file.
pub fn interactive_authorization(key: &str) -> Result<(), Error> {
    let scopes = get_scopes();
    let mut multi_storage = get_storage()?;

    let mut auth = YupAuthenticator::new(
        &get_app_secret()?,
        DefaultAuthenticatorDelegate,
        get_http_client()?,
        NullStorage::default(),
        Some(FlowType::InstalledInteractive)
    );

    let token = match auth.token(scopes.as_vec()) {
        Ok(t) => t,

        // Can't figure out how to adopt `e` into a failure::Error here:
        Err(e) => return Err(format_err!("OAuth2 login failed: {}", e))
    };

    multi_storage.add_token(&scopes, key, token)?;
    multi_storage.save_to_json()?;
    Ok(())
}


/// Perform a web-API operation for each logged-in account.
///
/// The callback has the signature `FnMut(email: &str, hub: &Drive) ->
/// Result<(), Error>`. In the definition here we get to use the elusive
/// `where for` syntax!
///
/// TODO: This can't be an iterator because I couldn't figure out how to write
/// `CentralizingDiskMultiTokenStorage::foreach` as an iterator.
pub fn foreach_account<F>(mut callback: F) -> Result<(), Error>
    where for<'a> F: FnMut(&'a str, &'a Drive<'a>) -> Result<(), Error>
{
    let secret = get_app_secret()?;
    let mut multi_storage = get_storage()?;

    for (email, tokens) in &mut multi_storage {
        let auth = Authenticator::new(
            &secret,
            DefaultAuthenticatorDelegate,
            get_http_client()?,
            tokens,
            None
        );

        let hub = google_drive3::Drive::new(get_http_client()?, auth);
        callback(&email, &hub)?;
    }

    // Our token(s) might have gotten updated.
    multi_storage.save_to_json()?;
    Ok(())
}


/// An app-specific type for the FileListCall type from `google_drive3`.
///
/// The main reason for providing this is to make it easier to write the
/// signature of the `list_files` call.
pub type FileListCall<'a, 'b> = google_drive3::FileListCall<'a, Client, Authenticator<'b>>;

/// Return an iterator over all files associated with this "hub".
///
/// The function *f* can customize the FileListCall instances to tune the
/// query that will be sent to Google's servers. The results for each query
/// may need to be paged, so the function may be called multiple times. (I
/// think? Based on the examples I've seen, it seems that you might need to
/// use the same query details when fetching subsequent pages in a multi-page
/// query.)
pub fn list_files<'a, 'b, F>(hub: &'b Drive<'a>, f: F) -> impl Iterator<Item = Result<google_drive3::File, Error>> + 'a
    where 'b: 'a,
          F: 'a + FnMut(FileListCall<'a, 'b>) -> FileListCall<'a, 'b>
{
    FileListing::new(hub, f)
}


/// Helper class for paging `files.list` results.
///
/// Well, this sure is a fun type to write. For some reason, we need to
/// include a PhantomData including the `'a` lifetime to prevent the compiler
/// from complaining about it being unused, even though that lifetime is
/// referenced by the type parameter F.
struct FileListing<'a, 'b, C, A, F>
    where 'b: 'a,
          F: FnMut(google_drive3::FileListCall<'a, C, A>) -> google_drive3::FileListCall<'a, C, A>,
          C: 'b + std::borrow::BorrowMut<hyper::Client>,
          A: 'b + yup_oauth2::GetToken
{
    hub: &'b google_drive3::Drive<C, A>,
    customizer: F,
    cur_page: Option<std::vec::IntoIter<google_drive3::File>>,
    next_page_token: Option<String>,
    finished: bool,
    final_page: bool,
    phantoma: std::marker::PhantomData<&'a A>,
}

impl<'a, 'b, C, A, F> FileListing<'a, 'b, C, A, F>
    where 'b: 'a,
          F: FnMut(google_drive3::FileListCall<'a, C, A>) -> google_drive3::FileListCall<'a, C, A>,
          C: 'b + std::borrow::BorrowMut<hyper::Client>,
          A: 'b + yup_oauth2::GetToken
{
    fn new(hub: &'b google_drive3::Drive<C, A>, f: F) -> FileListing<'a, 'b, C, A, F> {
        FileListing {
            hub,
            customizer: f,
            cur_page: None,
            next_page_token: None,
            finished: false,
            final_page: false,
            phantoma: std::marker::PhantomData,
        }
    }
}

impl<'a, 'b, C, A, F> Iterator for FileListing<'a, 'b, C, A, F>
    where 'b: 'a,
          F: FnMut(google_drive3::FileListCall<'a, C, A>) -> google_drive3::FileListCall<'a, C, A>,
          C: 'b + std::borrow::BorrowMut<hyper::Client>,
          A: 'b + yup_oauth2::GetToken
{
    type Item = Result<google_drive3::File, Error>;

    fn next(&mut self) -> Option<Result<google_drive3::File, Error>> {
        // If we set this flag, we either errored out or are totally done.

        if self.finished {
            return None;
        }

        // Are we currently in the midst of a page with items left? If so,
        // just return the next one.

        if let Some(iter) = self.cur_page.as_mut() {
            if let Some(file) = iter.next() {
                return Some(Ok(file));
            }
        }

        // Guess not. Was that the last page? If so, hooray -- we successfully
        // iterated over every document.

        if self.final_page {
            self.finished = true;
            return None;
        }

        // Nope. Try issuing a request for the next page of results. Here we
        // force the call to use our single master scope, which we probably
        // shouldn't do if we want to turn this into a reusabe library.

        let call = self.hub.files().list();
        let call = (self.customizer)(call);
        let call = call.set_scope(SCOPE);

        let call = if let Some(page_token) = self.next_page_token.take() {
            call.page_token(&page_token)
        } else {
            call
        };

        let (_resp, listing) = match call.doit() {
            Ok(t) => t,
            Err(e) => {
                self.finished = true;
                return Some(Err(format_err!("API call failed: {}", e)));
            }
        };

        // The listing contains (1) maybe a token that we can use to get the
        // next page of results and (2) a vector of information about the files
        // in this page.
        //
        // XXX: ignoring `incomplete_search` flag

        if let Some(page_token) = listing.next_page_token {
            self.next_page_token = Some(page_token);
        } else {
            // If there's no next page, this is the last page.
            self.final_page = true;
        }

        let mut files_iter = match listing.files {
            Some(f) => f.into_iter(),
            None => {
                self.finished = true;
                return Some(Err(format_err!("API call failed: no 'files' returned")));
            }
        };

        // OK, we finally have a iterator over a vector of files.

        let the_file = match files_iter.next() {
            Some(f) => f,
            None => {
                // This page was empty. This can of course happen if the user
                // has no documents, and it's OK if this was the final page.
                // If this wasn't the final page, we're in trouble because we
                // really ought to return Some(page). We could in principle
                // loop back and reissue the API call, and maybe the next page
                // *will* have items ... but that's a pain. So if this case
                // happens, error out. Either way, though, we're done.

                self.finished = true;

                return if self.final_page {
                    None
                } else {
                    Some(Err(format_err!("API call failed: empty page in midst of query")))
                };
            }
        };

        self.cur_page = Some(files_iter);
        Some(Ok(the_file))
    }
}

impl<'a, 'b, C, A, F> std::iter::FusedIterator for FileListing<'a, 'b, C, A, F>
    where 'b: 'a,
          F: FnMut(google_drive3::FileListCall<'a, C, A>) -> google_drive3::FileListCall<'a, C, A>,
          C: 'b + std::borrow::BorrowMut<hyper::Client>,
          A: 'b + yup_oauth2::GetToken
{}
