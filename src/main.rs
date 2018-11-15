// Copyright 2018 Peter Williams <peter@newton.cx>
// Licensed under the MIT License.

//! The main CLI driver logic.

extern crate app_dirs;
#[macro_use] extern crate failure;
extern crate google_drive3;
extern crate hyper;
extern crate hyper_native_tls;
extern crate serde;
#[macro_use] extern crate serde_derive;
extern crate serde_json;
extern crate structopt;
extern crate tempfile;
extern crate yup_oauth2;

use failure::Error;
use std::fs;
use std::process;
use structopt::StructOpt;
use yup_oauth2::{
    Authenticator, ApplicationSecret, ConsoleApplicationSecret,
    DefaultAuthenticatorDelegate, FlowType, GetToken, NullStorage,
};

mod token_storage;


/// Information used to find out app-specific config files, e.g. the
/// application secret.
const APP_INFO: app_dirs::AppInfo = app_dirs::AppInfo { name: "goodriver", author: "PKGW" };


/// Get the "application secret" needed to authenticate against Google APIs.
///
/// TODO: can we automate the creation and retrieval of this file? That would
/// be cool but not something to spend time on right now.
///
/// On Linux the desired filepath is `~/.config/goodriver/client_id.json`.
fn get_app_secret() -> Result<ApplicationSecret, Error> {
    let p = app_dirs::get_app_dir(app_dirs::AppDataType::UserConfig, &APP_INFO, "client_id.json")?;
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
    /// Create a new iterator over files in a Drive.
    ///
    /// The function *f* can customize the FileListCall instances to tune the
    /// query that will be sent to Google's servers. The results for each
    /// query may need to be paged, so the function may be called multiple
    /// times.
    pub fn new(hub: &'b google_drive3::Drive<C, A>, f: F) -> FileListing<'a, 'b, C, A, F> {
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

        // Nope. Try issuing a request for the next page of results.

        let call = self.hub.files().list();
        let call = (self.customizer)(call);

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


/// Temp? List documents.
#[derive(Debug, StructOpt)]
pub struct DriverListOptions {}

impl DriverListOptions {
    fn cli(self) -> Result<i32, Error> {
        let secret = get_app_secret()?;
        let mut multi_storage = token_storage::get_storage()?;

        multi_storage.foreach(|(email, tokens)| {
            let auth = Authenticator::new(
                &secret,
                DefaultAuthenticatorDelegate,
                get_http_client()?,
                tokens,
                None
            );

            let hub = google_drive3::Drive::new(get_http_client()?, auth);

            println!("{}:", email);

            for maybe_file in FileListing::new(&hub, |call| call) {
                let file = maybe_file?;
                let name = file.name.unwrap_or_else(|| "???".to_owned());
                println!("   {}", name);
            }

            Ok(())
        })?;

        // Our token(s) might get updated.
        multi_storage.save_to_json()?;
        Ok(0)
    }
}


/// The command-line action to add a login to the credentials DB.
#[derive(Debug, StructOpt)]
pub struct DriverLoginOptions {
    #[structopt(help = "An email address associated with the account")]
    email: String,
}

impl DriverLoginOptions {
    /// The auth flow here will print out a message on the console, asking the
    /// user to go to a URL, following instructions, and paste a string back
    /// into the client.
    ///
    /// We want to allow the user to login to multiple accounts
    /// simultaneously. Therefore we set up the authenticator flow with a null
    /// storage, and then add the resulting token to the disk storage.
    fn cli(self) -> Result<i32, Error> {
        let scopes = token_storage::get_scopes();
        let mut multi_storage = token_storage::get_storage()?;

        let mut auth = Authenticator::new(
            &get_app_secret()?,
            DefaultAuthenticatorDelegate,
            get_http_client()?,
            NullStorage::default(),
            Some(FlowType::InstalledInteractive)
        );

        let token = match auth.token(scopes.as_vec()) {
            Ok(t) => t,

            // Can't figure out how to adopt `e` into a failure::Error here:
            Err(e) => return Err(format_err!("auth failed: {}", e))
        };

        multi_storage.add_token(&scopes, &self.email, token)?;
        multi_storage.save_to_json()?;
        Ok(0)
    }
}


#[derive(Debug, StructOpt)]
#[structopt(name = "driver", about = "Deal with Google Drive.")]
pub enum DriverCli {
    #[structopt(name = "list")]
    /// List documents
    List(DriverListOptions),

    #[structopt(name = "login")]
    /// Add a Google account to be monitored
    Login(DriverLoginOptions),
}

impl DriverCli {
    fn cli(self) -> Result<i32, Error> {
        match self {
            DriverCli::List(opts) => opts.cli(),
            DriverCli::Login(opts) => opts.cli(),
        }
    }
}


fn main() {
    let program = DriverCli::from_args();

    process::exit(match program.cli() {
        Ok(code) => code,

        Err(e) => {
            eprintln!("fatal error in driver");
            for cause in e.iter_chain() {
                eprintln!("  caused by: {}", cause);
            }
            1
        },
    });
}
