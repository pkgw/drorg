// Copyright 2018 Peter Williams <peter@newton.cx>
// Licensed under the MIT License.

//! The main CLI driver logic.

#![deny(missing_docs)]

extern crate app_dirs;
#[macro_use] extern crate diesel;
#[macro_use] extern crate failure;
extern crate google_drive3;
extern crate google_people1;
extern crate hyper;
extern crate hyper_native_tls;
extern crate serde;
#[macro_use] extern crate serde_derive;
extern crate serde_json;
extern crate structopt;
extern crate tempfile;
extern crate url;
extern crate yup_oauth2;

use diesel::prelude::*;
use std::ffi::OsStr;
use std::process;
use structopt::StructOpt;

mod accounts;
mod app;
mod database;
mod errors;
mod google_apis;
mod schema;
mod token_storage;

use app::Application;
use errors::Result;


/// Information used to find out app-specific config files, e.g. the
/// application secret.
const APP_INFO: app_dirs::AppInfo = app_dirs::AppInfo { name: "goodriver", author: "PKGW" };


/// Open a URL in a browser.
///
/// HACK: I'm sure there's a nice cross-platform crate to do this, but
/// I customize it to use my Google-specific Firefox profile.
fn open_url<S: AsRef<OsStr>>(url: S) -> Result<()> {
    use std::process::Command;

    let status = Command::new("firefox")
        .args(&["-P", "google", "--new-window"])
        .arg(url)
        .status()?;

    if status.success() {
        Ok(())
    } else {
        Err(format_err!("browser command exited with an error code"))
    }
}


/// Temp? List documents.
#[derive(Debug, StructOpt)]
pub struct DriverListOptions {
    #[structopt(long = "no-sync", help = "Do not attempt to synchronize with the Google servers")]
    no_sync: bool,
}

impl DriverListOptions {
    fn cli(self, mut app: Application) -> Result<i32> {
        use schema::docs::dsl::*;

        if !self.no_sync {
            app.sync_all_accounts()?;
        }

        for doc in docs.load::<database::Doc>(&app.conn)? {
            let star = if doc.starred { "*" } else { " " };
            let trash = if doc.trashed { "T" } else { " " };
            println!("   {}{} {} ({})", star, trash, doc.name, doc.id);
        }

        Ok(0)
    }
}


/// The command-line action to add a login to the credentials DB.
///
/// Note that "email" doesn't really have to be an email address -- it can be
/// any random string; the user chooses which account to login-to
/// interactively during the login process. But I think it makes sense from a
/// UI perspective to just call it "email" and let the user figure out for
/// themselves that they can give it some other value if they feel like it.
#[derive(Debug, StructOpt)]
pub struct DriverLoginOptions {}

impl DriverLoginOptions {
    /// The auth flow here will print out a message on the console, asking the
    /// user to go to a URL, following instructions, and paste a string back
    /// into the client.
    ///
    /// We want to allow the user to login to multiple accounts
    /// simultaneously. Therefore we set up the authenticator flow with a null
    /// storage, and then add the resulting token to the disk storage.
    fn cli(self, mut app: Application) -> Result<i32> {
        let mut account = accounts::Account::default();

        // First we need to get authorization.
        account.authorize_interactively(&app.secret)?;

        // Now, for bookkeeping, we look up the email address associated with
        // it. We could just have the user specify an identifier, but I went
        // to the trouble to figure out how to do this right, so ...
        let email = account.fetch_email_address(&app.secret)?;
        println!("Successfully logged in to {}.", email);

        // Initialize our token for checking for changes to the documents. We
        // do this *before* scanning the complete listing; there's going to be
        // a race condition either way, but the one that happens with this
        // ordering seems like it would be more benign.
        account.acquire_change_page_token(&app.secret)?;

        // OK, now actually slurp in the list of documents.
        println!("Scanning documents ...");
        app.import_documents(&mut account)?;

        // All done.
        println!("Done.");
        Ok(0)
    }
}


/// Open a document.
#[derive(Debug, StructOpt)]
pub struct DriverOpenOptions {
    #[structopt(help = "A piece of the document name")]
    stem: String,
}

impl DriverOpenOptions {
    fn cli(self, app: Application) -> Result<i32> {
        // TODO: synchronize the database if needed, or something
        let pattern = format!("%{}%", self.stem);

        use schema::docs::dsl::*;

        let results = docs.filter(name.like(&pattern))
            .load::<database::Doc>(&app.conn)?;

        let url = match results.len() {
            0 => {
                println!("No known document names matched the pattern \"{}\"", self.stem);
                return Ok(1);
            },

            1 => results[0].open_url(),

            _n => {
                println!("Multiple documents matched the pattern \"{}\":", self.stem);
                println!("");
                for r in results {
                    println!("   {}", r.name);
                }
                println!("");
                println!("Please use a more specific filter.");
                return Ok(1);
            }
        };

        open_url(url)?;
        Ok(0)
    }
}


/// Resynchronize with an account.
#[derive(Debug, StructOpt)]
pub struct DriverResyncOptions {}

impl DriverResyncOptions {
    fn cli(self, mut app: Application) -> Result<i32> {
        for maybe_info in accounts::get_accounts()? {
            let (email, mut account) = maybe_info?;

            // Redo the initialization rigamarole from the "login" command.
            println!("Re-initializing {} ...", email);
            account.acquire_change_page_token(&app.secret)?;
            app.import_documents(&mut account)?;
        }

        Ok(0)
    }
}


/// Temp debugging
#[derive(Debug, StructOpt)]
pub struct DriverTempOptions {}

impl DriverTempOptions {
    fn cli(self, app: Application) -> Result<i32> {
        for maybe_info in accounts::get_accounts()? {
            let (email, mut account) = maybe_info?;

            let token = account.data.change_page_token.take().ok_or(
                format_err!("no paging token for {}", email)
            )?;

            let token = account.with_drive_hub(&app.secret, |hub| {
                let mut lister = google_apis::list_changes(
                    &hub, &token,
                    |call| call.spaces("drive")
                        .supports_team_drives(true)
                        .include_team_drive_items(true)
                        .include_removed(true)
                        .include_corpus_removals(true)
                );

                for maybe_change in lister.iter() {
                    let change = maybe_change?;
                    println!("{:?}", change);
                }

                Ok(lister.into_change_page_token())
            })?;

            account.data.change_page_token = Some(token);
            account.save_to_json()?;
        }

        Ok(0)
    }
}


/// The main StructOpt type for dispatching subcommands.
#[derive(Debug, StructOpt)]
#[structopt(name = "driver", about = "Deal with Google Drive.")]
pub enum DriverCli {
    #[structopt(name = "list")]
    /// List documents
    List(DriverListOptions),

    #[structopt(name = "login")]
    /// Add a Google account to be monitored
    Login(DriverLoginOptions),

    #[structopt(name = "open")]
    /// Open a document in a web browser
    Open(DriverOpenOptions),

    #[structopt(name = "resync")]
    /// Re-synchronize with an account
    Resync(DriverResyncOptions),

    #[structopt(name = "temp")]
    /// Temporary dev work
    Temp(DriverTempOptions),
}

impl DriverCli {
    fn cli(self) -> Result<i32> {
        let app = Application::initialize()?;

        match self {
            DriverCli::List(opts) => opts.cli(app),
            DriverCli::Login(opts) => opts.cli(app),
            DriverCli::Open(opts) => opts.cli(app),
            DriverCli::Resync(opts) => opts.cli(app),
            DriverCli::Temp(opts) => opts.cli(app),
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
