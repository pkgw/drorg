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

            let (_resp, listing) = match hub.files().list().doit() {
                Ok(x) => x,
                Err(e) => return Err(format_err!("API call failed: {}", e))
            };

            let files = listing.files.unwrap_or_else(|| Vec::new());

            for file in files {
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
