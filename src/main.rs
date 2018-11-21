// Copyright 2018 Peter Williams <peter@newton.cx>
// Licensed under the MIT License.

//! The main CLI driver logic.

extern crate app_dirs;
#[macro_use] extern crate diesel;
#[macro_use] extern crate failure;
extern crate google_drive3;
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
use failure::Error;
use std::ffi::OsStr;
use std::process;
use structopt::StructOpt;

mod accounts;
mod database;
mod gdrive;
mod schema;
mod token_storage;


/// Information used to find out app-specific config files, e.g. the
/// application secret.
const APP_INFO: app_dirs::AppInfo = app_dirs::AppInfo { name: "goodriver", author: "PKGW" };


/// Open a URL in a browser.
///
/// HACK: I'm sure there's a nice cross-platform crate to do this, but
/// I customize it to use my Google-specific Firefox profile.
fn open_url<S: AsRef<OsStr>>(url: S) -> Result<(), Error> {
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
pub struct DriverListOptions {}

impl DriverListOptions {
    fn cli(self) -> Result<i32, Error> {
        // TODO: sync with Google, or something
        let conn = database::get_db_connection()?;

        use schema::docs::dsl::*;

        for doc in docs.load::<database::Doc>(&conn)? {
            println!("   {} ({})", doc.name, doc.id);
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
        let mut accounts = accounts::get_accounts()?;
        accounts.authorize_interactively(&self.email)?;
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
    fn cli(self) -> Result<i32, Error> {
        // TODO: synchronize the database if needed, or something
        let conn = database::get_db_connection()?;
        let pattern = format!("%{}%", self.stem);

        use schema::docs::dsl::*;

        let results = docs.filter(name.like(&pattern))
            .load::<database::Doc>(&conn)?;

        let id_to_open = match results.len() {
            0 => {
                println!("No known document names matched the pattern \"{}\"", self.stem);
                return Ok(1);
            },

            1 => &results[0].id,

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

        let mut url = hyper::Url::parse("https://drive.google.com/open").unwrap();
        use url::percent_encoding::{utf8_percent_encode, QUERY_ENCODE_SET};
        let q = utf8_percent_encode(id_to_open, QUERY_ENCODE_SET).to_string();
        url.set_query(Some(&format!("id={}", q)));

        open_url(url.as_str())?;
        Ok(0)
    }
}


/// Temp? Fill the database with the current set of remote documents.
#[derive(Debug, StructOpt)]
pub struct DriverSyncOptions {}

impl DriverSyncOptions {
    fn cli(self) -> Result<i32, Error> {
        let conn = database::get_db_connection()?;
        let mut accounts = accounts::get_accounts()?;

        accounts.foreach_hub(|email, hub| {
            // TODO we need to delete old records and stuff!
            for maybe_file in gdrive::list_files(&hub, |call| call.spaces("drive")) {
                let file = maybe_file?;
                let name = file.name.as_ref().map_or("???", |s| s);
                let id = match file.id.as_ref() {
                    Some(s) => s,
                    None => {
                        eprintln!("got a document without an ID in account {}; ignoring", email);
                        continue;
                    }
                };

                let new_doc = database::NewDoc {
                    id: id,
                    name: name,
                };

                diesel::insert_or_ignore_into(schema::docs::table)
                    .values(&new_doc)
                    .execute(&conn)?;
            }

            Ok(())
        })?;

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

    #[structopt(name = "open")]
    /// Open a document in a web browser
    Open(DriverOpenOptions),

    #[structopt(name = "sync")]
    /// Synchronize the local database with Google Drve
    Sync(DriverSyncOptions),
}

impl DriverCli {
    fn cli(self) -> Result<i32, Error> {
        match self {
            DriverCli::List(opts) => opts.cli(),
            DriverCli::Login(opts) => opts.cli(),
            DriverCli::Open(opts) => opts.cli(),
            DriverCli::Sync(opts) => opts.cli(),
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
