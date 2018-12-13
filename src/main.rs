// Copyright 2018 Peter Williams <peter@newton.cx>
// Licensed under the MIT License.

//! The main CLI driver logic.

#![deny(missing_docs)]
#![allow(proc_macro_derive_resolution_fallback)]

extern crate app_dirs;
extern crate chrono;
#[macro_use] extern crate clap; // for arg_enum!
#[macro_use] extern crate diesel;
#[macro_use] extern crate failure;
extern crate google_drive3;
extern crate hyper;
extern crate hyper_native_tls;
extern crate petgraph;
extern crate serde;
#[macro_use] extern crate serde_derive;
extern crate serde_json;
extern crate structopt;
#[macro_use] extern crate tcprint;
extern crate tempfile;
extern crate timeago;
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
const APP_INFO: app_dirs::AppInfo = app_dirs::AppInfo { name: "drorg", author: "drorg" };


/// Open a URL in a browser.
///
/// HACK: I'm sure there's a nice cross-platform crate to do this, but
/// I customize it to use my Google-specific Firefox profile.
fn open_url<S: AsRef<OsStr>>(url: S) -> Result<()> {
    use std::process::Command;

    let status = Command::new("firefox-wayland")
        .args(&["-P", "google", "--new-window"])
        .arg(url)
        .status()?;

    if status.success() {
        Ok(())
    } else {
        Err(format_err!("browser command exited with an error code"))
    }
}


/// Show detailed information about one or more documents.
#[derive(Debug, StructOpt)]
pub struct DrorgInfoOptions {
    #[structopt(help = "A document specifier (name, ID, ...)")]
    spec: String,
}

impl DrorgInfoOptions {
    fn cli(self, mut app: Application) -> Result<i32> {
        use std::collections::HashMap;

        app.maybe_sync_all_accounts()?;

        let mut linkages = HashMap::new();
        let results = app.get_docs().process(&self.spec)?; // note: avoid name clash with db table
        let mut first = true;

        for doc in results {
            if first {
                first = false;
            } else {
                tcprintln!(app.ps, (""));
            }

            tcprintln!(app.ps, [hl: "Name:"], ("      "), [green: "{}", doc.name]);
            tcprintln!(app.ps, [hl: "MIME-type:"], (" {}", doc.mime_type));
            tcprintln!(app.ps, [hl: "Modified:"], ("  {}", doc.utc_mod_time().to_rfc3339()));
            tcprintln!(app.ps, [hl: "ID:"], ("        {}", doc.id));
            tcprintln!(app.ps, [hl: "Starred?:"], ("  {}", if doc.starred { "yes" } else { "no" }));
            tcprintln!(app.ps, [hl: "Trashed?:"], ("  {}", if doc.trashed { "yes" } else { "no" }));

            // Which accounts is this file associated with? This tells us
            // which linkage tables we need.

            let accounts = {
                use schema::account_associations::dsl::*;
                let associations = account_associations.inner_join(schema::accounts::table)
                    .filter(doc_id.eq(&doc.id))
                    .load::<(database::AccountAssociation, database::Account)>(&app.conn)?;
                let accounts: Vec<_> = associations.iter().map(|(_assoc, account)| account.clone()).collect();
                accounts
            };

            let mut path_reprs = Vec::new();

            for acct in &accounts {
                // This hashmap stuff is inefficient basically because the
                // Entry API doesn't let us work with a value-creation
                // function that returns a Result.

                if !linkages.contains_key(&acct.id) {
                    let table = app.load_linkage_table(acct.id, true)?;
                    linkages.insert(acct.id, table);
                }

                let link_table = linkages.get(&acct.id).unwrap();

                for p in link_table.find_parent_paths(&doc.id).iter().map(|id_path| {
                    // This is not efficient, and it's panicky, but meh.
                    let names: Vec<_> = id_path.iter().map(|docid| {
                        use schema::docs::dsl::*;
                        let elem = docs.filter(id.eq(&docid))
                            .first::<database::Doc>(&app.conn).unwrap();
                        elem.name.clone()
                    }).collect();

                    names.join(" > ")
                }) {
                    path_reprs.push(format!("{}: {}", acct.email, p));
                }
            }

            match path_reprs.len() {
                0 => tcprintln!(app.ps, [hl: "Path:"], ("      [none??]")),
                1 => tcprintln!(app.ps, [hl: "Path:"], ("      {}", path_reprs[0])),
                _n => {
                    tcprintln!(app.ps, [hl: "Paths::"]);
                    for p in path_reprs {
                        tcprintln!(app.ps, ("    {}", p));
                    }
                }
            }

            tcprintln!(app.ps, [hl: "Open-URL:"], ("  {}", doc.open_url()));
        }

        Ok(0)
    }
}


/// Temp? List documents.
#[derive(Debug, StructOpt)]
pub struct DrorgListOptions {
    #[structopt(help = "A document specifier (name, ID, ...)")]
    spec: String,
}

impl DrorgListOptions {
    fn cli(self, mut app: Application) -> Result<i32> {
        use chrono::Utc;
        let now = Utc::now();

        app.maybe_sync_all_accounts()?;

        let results = app.get_docs().process(&self.spec)?;

        for doc in results {
            let star = if doc.starred { "*" } else { " " };
            let trash = if doc.trashed { "T" } else { " " };
            let is_folder = if doc.is_folder() { "F" } else { " " };

            let ago = now.signed_duration_since(doc.utc_mod_time());
            let ago = ago.to_std().map(
                |stddur| timeago::Formatter::new().convert(stddur)
            ).unwrap_or_else(
                |_err| "[future?]".to_owned()
            );

            tcprintln!(app.ps, ("   {}{}{} {} ({})  {}", star, trash, is_folder, doc.name, doc.id, ago));
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
pub struct DrorgLoginOptions {}

impl DrorgLoginOptions {
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
        let email_addr = account.fetch_email_address(&app.secret)?;
        tcprintln!(app.ps, ("Successfully logged in to "), [hl: "{}", email_addr], ("."));

        // We might need to add this account to the database. To have sensible
        // foreign key relations, the email address is not the primary key of
        // the accounts table, so we need to see whether there's already an
        // existing row for this account (which could happen if the user
        // re-logs-in, etc.) If we add a new row, we have to do this awkward
        // bit where we insert and then immediately query for the row we just
        // added (cf https://github.com/diesel-rs/diesel/issues/771 ).
        {
            use diesel::prelude::*;
            use schema::accounts::dsl::*;

            let maybe_row = accounts.filter(email.eq(&email_addr))
                .first::<database::Account>(&app.conn)
                .optional()?;

            let row_id = if let Some(row) = maybe_row {
                row.id
            } else {
                let new_account = database::NewAccount::new(&email_addr);
                diesel::replace_into(accounts)
                    .values(&new_account)
                    .execute(&app.conn)?;

                let row = accounts.filter(email.eq(&email_addr))
                    .first::<database::Account>(&app.conn)?;
                row.id
            };

            account.data.db_id = row_id;
            // JSON will be rewritten in acquire_change_page_token below.
        }

        // Initialize our token for checking for changes to the documents. We
        // do this *before* scanning the complete listing; there's going to be
        // a race condition either way, but the one that happens with this
        // ordering seems like it would be more benign.
        account.acquire_change_page_token(&app.secret)?;

        // OK, now actually slurp in the list of documents.
        tcprintln!(app.ps, ("Scanning documents ..."));
        app.import_documents(&mut account)?;

        // All done.
        tcprintln!(app.ps, ("Done."));
        Ok(0)
    }
}


/// Open a document.
#[derive(Debug, StructOpt)]
pub struct DrorgOpenOptions {
    #[structopt(help = "A document specifier (name, ID, ...)")]
    spec: String,
}

impl DrorgOpenOptions {
    fn cli(self, mut app: Application) -> Result<i32> {
        app.maybe_sync_all_accounts()?;

        let doc = app.get_docs().process_one(self.spec)?;
        open_url(doc.open_url())?;
        Ok(0)
    }
}


/// List recently-used documents.
#[derive(Debug, StructOpt)]
pub struct DrorgRecentOptions {
    #[structopt(short="n", help="Limit output to this many documents", default_value="10")]
    limit: i64,
}

impl DrorgRecentOptions {
    fn cli(self, mut app: Application) -> Result<i32> {
        use schema::docs::dsl::*;

        app.maybe_sync_all_accounts()?;

        let listing = docs.order(modified_time.desc())
            .limit(self.limit)
            .load::<database::Doc>(&app.conn)?;

        app.print_doc_list(listing);
        Ok(0)
    }
}


/// Synchronize with the cloud.
#[derive(Debug, StructOpt)]
pub struct DrorgSyncOptions {
    #[structopt(long = "rebuild", help = "Rebuild all account data from scratch")]
    rebuild: bool,
}

impl DrorgSyncOptions {
    fn cli(self, mut app: Application) -> Result<i32> {
        if !self.rebuild {
            // Lightweight sync
            app.options.sync = app::SyncOption::Yes;
            app.maybe_sync_all_accounts()?;
        } else {
            // Heavyweight -- rebuild account data from scratch.
            for maybe_info in accounts::get_accounts()? {
                let (email, mut account) = maybe_info?;

                // TODO: delete all links involving documents from this account.
                // To be safest, perhaps we should destroy all database rows
                // associated with this account?

                // Redo the initialization rigamarole from the "login" command.
                tcprintln!(app.ps, ("Rebuilding "), [hl: "{}", email], (" ..."));
                account.acquire_change_page_token(&app.secret)?;
                app.import_documents(&mut account)?;
            }
        }

        Ok(0)
    }
}


/// The main StructOpt type for dispatching subcommands.
#[derive(Debug, StructOpt)]
pub enum DrorgSubcommand {
    #[structopt(name = "info")]
    /// Show detailed information about one or more documents
    Info(DrorgInfoOptions),

    #[structopt(name = "list")]
    /// List documents
    List(DrorgListOptions),

    #[structopt(name = "login")]
    /// Add a Google account to be monitored
    Login(DrorgLoginOptions),

    #[structopt(name = "open")]
    /// Open a document in a web browser
    Open(DrorgOpenOptions),

    #[structopt(name = "recent")]
    /// List recently-used documents
    Recent(DrorgRecentOptions),

    #[structopt(name = "sync")]
    /// Synchronize with the cloud
    Sync(DrorgSyncOptions),
}


/// The main StructOpt argument dispatcher.
#[derive(Debug, StructOpt)]
#[structopt(name = "drorg", about = "Organize documents on Google Drive.")]
pub struct DrorgCli {
    #[structopt(subcommand)]
    command: DrorgSubcommand,

    #[structopt(flatten)]
    app_opts: app::ApplicationOptions,
}


impl DrorgCli {
    fn cli(self) -> Result<i32> {
        let app = Application::initialize(self.app_opts)?;

        match self.command {
            DrorgSubcommand::Info(opts) => opts.cli(app),
            DrorgSubcommand::List(opts) => opts.cli(app),
            DrorgSubcommand::Login(opts) => opts.cli(app),
            DrorgSubcommand::Open(opts) => opts.cli(app),
            DrorgSubcommand::Recent(opts) => opts.cli(app),
            DrorgSubcommand::Sync(opts) => opts.cli(app),
        }
    }
}


fn main() {
    let program = DrorgCli::from_args();

    process::exit(match program.cli() {
        Ok(code) => code,

        Err(e) => {
            eprintln!("fatal error in drorg");
            for cause in e.iter_chain() {
                eprintln!("  caused by: {}", cause);
            }
            1
        },
    });
}
