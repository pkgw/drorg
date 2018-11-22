// Copyright 2018 Peter Williams <peter@newton.cx>
// Licensed under the MIT License.

//! Our interface with the Google Drive web API.
//!
//! Debugging tip: if API calls are failing mysteriously, break on
//! `http_failure` in GDB and look at the JSON output being returned. I can't
//! figure out a more convenient way to access the API server's error
//! explanations.

use hyper::Client;
use std::cell::RefCell;
use std::fs;
use std::rc::Rc;
use yup_oauth2::{
    Authenticator as YupAuthenticator, ApplicationSecret,
    ConsoleApplicationSecret, DefaultAuthenticatorDelegate,
    FlowType, GetToken, NullStorage, TokenStorage,
};

use errors::{AdaptExternalResult, Result};
use token_storage::{ScopeList, SerdeMemoryStorage};

/// The app-specific token storage type.
pub type TokenStore<'a> = &'a mut SerdeMemoryStorage;

/// The app-specific authenticator type.
pub type Authenticator<'a> = YupAuthenticator<DefaultAuthenticatorDelegate,
                                              TokenStore<'a>,
                                              Client>;

/// The app-specific Drive API "hub" type.
pub type Drive<'a> = google_drive3::Drive<Client, Authenticator<'a>>;

/// The app-specific People Service API "hub" type.
pub type People<'a> = google_people1::PeopleService<Client, Authenticator<'a>>;


/// Get the "application secret" needed to authenticate against Google APIs.
///
/// TODO: can we automate the creation and retrieval of this file? That would
/// be cool but not something to spend time on right now.
///
/// On Linux the desired filepath is `~/.config/goodriver/client_id.json`.
pub fn get_app_secret() -> Result<ApplicationSecret> {
    let p = app_dirs::get_app_dir(app_dirs::AppDataType::UserConfig, &::APP_INFO, "client_id.json")?;
    let f = fs::File::open(p)?;
    let cfg: ConsoleApplicationSecret = serde_json::from_reader(f)?;
    cfg.installed.ok_or_else(|| format_err!("no installed-application secret"))
}


/// Get an HTTP client with all the bells and whistles we need.
pub fn get_http_client() -> Result<hyper::Client> {
    Ok(hyper::Client::with_connector(
        hyper::net::HttpsConnector::new(
            hyper_native_tls::NativeTlsClient::new()?
        )
    ))
}


/// The first of tese strings is `google_drive3::Scope::Full.as_ref(). It's
/// convenient to have this scope as a static string constant. The other
/// scopes are needed to figure out the email address associted with each
/// account on login.
pub const SCOPES: &[&str] = &[
    "https://www.googleapis.com/auth/drive",
    "profile",
    "email",
];


/// Get a ScopeList representing the scopes that we need.
///
/// This list is specific to this application.
pub fn get_scopes() -> ScopeList<'static> {
    ScopeList::new(SCOPES)
}


/// Helper trait for generic operations on API calls
///
/// Every API call implements these features, but not as a trait, so we can't
/// access them generically without adding a helper trait.
pub trait CallBuilderExt: Sized {
    /// Set the authorization scope to be used for this API call.
    ///
    /// This just wraps the `add_scope` call implemented for every CallBuilder
    /// type. Note that the auto-generated documentation for those functions
    /// is not accurate.
    fn set_scope<S: AsRef<str>>(self, scope: S) -> Self;

    fn default_scope(mut self) -> Self {
        for scope in SCOPES {
            self = self.set_scope(scope);
        }

        self
    }
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
impl_call_builder_ext!(google_drive3::ChangeListCall<'a, C, A>);
impl_call_builder_ext!(google_drive3::FileListCall<'a, C, A>);
impl_call_builder_ext!(google_people1::PeopleGetCall<'a, C, A>);


/// Ask the user to authorize our app to use an account, interactively.
///
/// Note that if the user has multiple accounts, they'll be able to choose
/// which one to authorize the app for. We can't have any control over which
/// one it is.
///
/// The `where` clause in the definition here is a mini-hack that allows the
/// compiler to be sure that the `storage.set()` error type can be converted
/// into a failure::Error.
pub fn authorize_interactively<T: TokenStorage>(secret: &ApplicationSecret, storage: &mut T) -> Result<()>
    where <T as TokenStorage>::Error: Sync + Send
{
    let scopes = get_scopes();

    let mut auth = YupAuthenticator::new(
        secret,
        DefaultAuthenticatorDelegate,
        get_http_client()?,
        NullStorage::default(),
        Some(FlowType::InstalledInteractive)
    );

    let token = auth.token(scopes.as_vec()).adapt()?;
    Ok(storage.set(scopes.hash, &scopes.scopes, Some(token))?)
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
pub fn list_files<'a, 'b, F>(hub: &'b Drive<'a>, f: F) -> impl Iterator<Item = Result<google_drive3::File>> + 'a
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
    type Item = Result<google_drive3::File>;

    fn next(&mut self) -> Option<Result<google_drive3::File>> {
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
        let call = call.default_scope();

        let call = if let Some(page_token) = self.next_page_token.take() {
            call.page_token(&page_token)
        } else {
            call
        };

        let (_resp, listing) = match call.doit().adapt() {
            Ok(t) => t,
            Err(e) => {
                self.finished = true;
                return Some(Err(e));
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



/// An app-specific type for the ChangeListCall type from `google_drive3`.
///
/// The main reason for providing this is to make it easier to write the
/// signature of the `list_changes` call.
pub type ChangeListCall<'a, 'b> = google_drive3::ChangeListCall<'a, Client, Authenticator<'b>>;

/// Return a type for iterating over all changes associated with this "hub".
///
/// Because information must be fed back to the caller after the iteration,
/// this function returns a helper type that has an `iter()` method that
/// should be used to do the iteration.
///
/// The function *f* can customize the ChangeListCall instances to tune the
/// query that will be sent to Google's servers. The results for each query
/// may need to be paged, so the function may be called multiple times.
pub fn list_changes<'a, 'b, F>(
    hub: &'b Drive<'a>, page_token: &str, f: F
) -> ChangeListing<'a, 'b, Client, Authenticator<'a>, F>
    where 'b: 'a,
          F: FnMut(ChangeListCall<'a, 'b>) -> ChangeListCall<'a, 'b> + 'a
{
    ChangeListing::new(hub, page_token, f)
}


/// Helper type for `list_changes`.
///
/// After iterating over the changes, we need to retrieve the updated token
/// that the API told us. Since for-loop iteration consumes the thing being
/// iterated over, we need this type to make the retrieval possible.
pub struct ChangeListing<'a, 'b, C, A, F>
    where 'b: 'a,
          F: FnMut(google_drive3::ChangeListCall<'a, C, A>) -> google_drive3::ChangeListCall<'a, C, A>,
          C: 'b + std::borrow::BorrowMut<hyper::Client>,
          A: 'b + yup_oauth2::GetToken
{
    iter: Option<ChangeListingIterator<'a, 'b, C, A, F>>,

    // Ugh, this is so gnarly.
    next_page_token: Rc<RefCell<String>>,
}

impl<'a, 'b, C, A, F> ChangeListing<'a, 'b, C, A, F>
    where 'b: 'a,
          F: FnMut(google_drive3::ChangeListCall<'a, C, A>) -> google_drive3::ChangeListCall<'a, C, A> + 'a,
          C: 'b + std::borrow::BorrowMut<hyper::Client>,
          A: 'b + yup_oauth2::GetToken
{
    fn new(hub: &'b google_drive3::Drive<C, A>, page_token: &str, f: F) -> ChangeListing<'a, 'b, C, A, F> {
        let tok = Rc::new(RefCell::new(page_token.to_owned()));
        let iter = Some(ChangeListingIterator::new(hub, tok.clone(), f));

        ChangeListing {
            iter,
            next_page_token: tok
        }
    }

    pub fn iter(&mut self) -> impl Iterator<Item = Result<google_drive3::Change>> + 'a {
        self.iter.take().unwrap()
    }

    pub fn into_change_page_token(self) -> String {
        Rc::try_unwrap(self.next_page_token).unwrap().into_inner()
    }
}


/// Iteration helper for paging `changes.list` results.
struct ChangeListingIterator<'a, 'b, C, A, F>
    where 'b: 'a,
          F: FnMut(google_drive3::ChangeListCall<'a, C, A>) -> google_drive3::ChangeListCall<'a, C, A>,
          C: 'b + std::borrow::BorrowMut<hyper::Client>,
          A: 'b + yup_oauth2::GetToken
{
    hub: &'b google_drive3::Drive<C, A>,
    next_page_token: Rc<RefCell<String>>,
    customizer: F,
    cur_page: Option<std::vec::IntoIter<google_drive3::Change>>,
    finished: bool,
    final_page: bool,
    phantoma: std::marker::PhantomData<&'a A>,
}

impl<'a, 'b, C, A, F> ChangeListingIterator<'a, 'b, C, A, F>
    where 'b: 'a,
          F: FnMut(google_drive3::ChangeListCall<'a, C, A>) -> google_drive3::ChangeListCall<'a, C, A>,
          C: 'b + std::borrow::BorrowMut<hyper::Client>,
          A: 'b + yup_oauth2::GetToken
{
    fn new(hub: &'b google_drive3::Drive<C, A>, tok: Rc<RefCell<String>>, f: F) -> ChangeListingIterator<'a, 'b, C, A, F> {
        ChangeListingIterator {
            hub,
            next_page_token: tok,
            customizer: f,
            cur_page: None,
            finished: false,
            final_page: false,
            phantoma: std::marker::PhantomData,
        }
    }
}

impl<'a, 'b, C, A, F> Iterator for ChangeListingIterator<'a, 'b, C, A, F>
    where 'b: 'a,
          F: FnMut(google_drive3::ChangeListCall<'a, C, A>) -> google_drive3::ChangeListCall<'a, C, A>,
          C: 'b + std::borrow::BorrowMut<hyper::Client>,
          A: 'b + yup_oauth2::GetToken
{
    type Item = Result<google_drive3::Change>;

    fn next(&mut self) -> Option<Result<google_drive3::Change>> {
        // If we set this flag, we either errored out or are totally done.

        if self.finished {
            return None;
        }

        // Are we currently in the midst of a page with items left? If so,
        // just return the next one.

        if let Some(iter) = self.cur_page.as_mut() {
            if let Some(change) = iter.next() {
                return Some(Ok(change));
            }
        }

        // Guess not. Was that the last page? If so, hooray -- we successfully
        // iterated over every document.

        if self.final_page {
            self.finished = true;
            return None;
        }

        // Nope. Try issuing a request for the next page of results.

        let call = self.hub.changes().list(&(*self.next_page_token).borrow());
        let call = (self.customizer)(call);
        let call = call.default_scope();

        let (_resp, listing) = match call.doit().adapt() {
            Ok(t) => t,
            Err(e) => {
                self.finished = true;
                return Some(Err(e));
            }
        };

        // The listing contains (1) maybe a token that we can use to get the
        // next page of results, (2) if not that, then a token for us to ask
        // about changes next time, and (3) a vector of information about the
        // changes in this page.

        if let Some(page_token) = listing.next_page_token {
            (*self.next_page_token).replace(page_token);
        } else {
            if let Some(start_page_token) = listing.new_start_page_token {
                (*self.next_page_token).replace(start_page_token);
                self.final_page = true;
            } else {
                self.finished = true;
                return Some(Err(format_err!("API call failed: Neither next_page_token nor \
                                             new_start_page_token provided")));
            }
        }

        let mut changes_iter = match listing.changes {
            Some(f) => f.into_iter(),
            None => {
                self.finished = true;
                return Some(Err(format_err!("API call failed: no 'changes' returned")));
            }
        };

        // OK, we finally have a iterator over a vector of changes.

        let the_change = match changes_iter.next() {
            Some(f) => f,
            None => {
                // This page was empty. This can of course happen there are no
                // changes to report, and it's OK if this was the final page.
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

        self.cur_page = Some(changes_iter);
        Some(Ok(the_change))
    }
}

impl<'a, 'b, C, A, F> std::iter::FusedIterator for ChangeListingIterator<'a, 'b, C, A, F>
    where 'b: 'a,
          F: FnMut(google_drive3::ChangeListCall<'a, C, A>) -> google_drive3::ChangeListCall<'a, C, A>,
          C: 'b + std::borrow::BorrowMut<hyper::Client>,
          A: 'b + yup_oauth2::GetToken
{}
