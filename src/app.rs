// Copyright 2018 Peter Williams <peter@newton.cx>
// Licensed under the MIT License.

//! The main application state.

use chrono::{DateTime, Duration, Utc};
use diesel::prelude::*;
use diesel::sqlite::SqliteConnection;
use petgraph::prelude::*;
use std::collections::HashMap;
use structopt::StructOpt;
use tcprint::ColorPrintState;
use yup_oauth2::ApplicationSecret;

use accounts::{self, Account};
use colors::Colors;
use database::{self, Doc};
use errors::Result;
use google_apis;
use schema;

arg_enum! {
    /// An enum for specifying how we should synchronize with the servers
    #[derive(Clone, Debug, Eq, PartialEq)]
    pub enum SyncOption {
        No,
        Auto,
        Yes,
    }
}

/// Global options for the application.
#[derive(Debug, StructOpt)]
pub struct ApplicationOptions {
    #[structopt(
        long = "sync",
        help = "Whether to synchronize with the Google Drive servers",
        parse(try_from_str),
        default_value = "auto",
        raw(possible_values = r#"&["auto", "no", "yes"]"#)
    )]
    pub sync: SyncOption,
}

/// The runtime state of the application.
pub struct Application {
    /// The global options provided on the command line.
    pub options: ApplicationOptions,

    /// The secret we use to identify this client to Google.
    pub secret: ApplicationSecret,

    /// Our connection to the database of document information.
    pub conn: SqliteConnection,

    /// The state object for colorized terminal output.
    pub ps: ColorPrintState<Colors>,
}

impl Application {
    /// Initialize the application.
    pub fn initialize(options: ApplicationOptions) -> Result<Application> {
        let secret = google_apis::get_app_secret()?;
        let conn = database::get_db_connection()?;
        let ps = ColorPrintState::default();

        Ok(Application {
            options,
            secret,
            conn,
            ps,
        })
    }

    /// Fill the database with records for all of the documents associated
    /// with an account.
    pub fn import_documents(&mut self, account: &mut Account) -> Result<()> {
        let the_account_id = account.data.db_id; // borrowck fun

        let root_id: String = account.with_drive_hub(&self.secret, |hub| {
            // This redundant codepath feels kind of ugly, but so far it seems
            // like the least-bad way to make sure we get info about the root
            // document.
            let root_id = {
                let file = google_apis::get_file(&hub, "root", |call| {
                    call.param(
                        "fields",
                        "id,mimeType,modifiedTime,name,parents,\
                         size,starred,trashed",
                    )
                })?;
                let new_doc = database::NewDoc::from_api_object(&file)?;
                diesel::replace_into(schema::docs::table)
                    .values(&new_doc)
                    .execute(&self.conn)?;

                let new_assn = database::NewAccountAssociation::new(&new_doc.id, the_account_id);
                diesel::replace_into(schema::account_associations::table)
                    .values(&new_assn)
                    .execute(&self.conn)?;

                new_doc.id.to_owned()
            };

            for maybe_file in google_apis::list_files(&hub, |call| {
                call.spaces("drive").param(
                    "fields",
                    "files(id,mimeType,modifiedTime,name,parents,\
                     size,starred,trashed),nextPageToken",
                )
            }) {
                let file = maybe_file?;
                let new_doc = database::NewDoc::from_api_object(&file)?;
                diesel::replace_into(schema::docs::table)
                    .values(&new_doc)
                    .execute(&self.conn)?;

                let new_assn = database::NewAccountAssociation::new(&new_doc.id, the_account_id);
                diesel::replace_into(schema::account_associations::table)
                    .values(&new_assn)
                    .execute(&self.conn)?;

                // Note that we make no effort to delete any parent-child
                // links in the database that don't correspond to items
                // returned here:

                if let Some(parents) = file.parents.as_ref() {
                    for pid in parents {
                        let new_link = database::NewLink::new(the_account_id, pid, &new_doc.id);
                        diesel::replace_into(schema::links::table)
                            .values(&new_link)
                            .execute(&self.conn)?;
                    }
                }
            }

            Ok(root_id)
        })?;

        account.data.root_folder_id = root_id;
        account.data.last_sync = Some(Utc::now());
        account.save_to_json()?;
        Ok(())
    }

    /// Synchronize the database with recent changes in this account.
    ///
    /// Note that this doesn't set `data.last_sync`, since its caller has a
    /// `now` object handy — this is pure laziness.
    fn sync_account(&mut self, email: &str, account: &mut Account) -> Result<()> {
        let the_account_id = account.data.db_id; // borrowck fun

        let token = account
            .data
            .change_page_token
            .take()
            .ok_or(format_err!("no change-paging token for {}", email))?;

        let token = account.with_drive_hub(&self.secret, |hub| {
            let mut lister = google_apis::list_changes(&hub, &token, |call| {
                call.spaces("drive")
                    .supports_team_drives(true)
                    .include_team_drive_items(true)
                    .include_removed(true)
                    .include_corpus_removals(true)
                    .param(
                        "fields",
                        "changes(file(id,mimeType,modifiedTime,name,parents,\
                         size,starred,trashed),fileId,removed),newStartPageToken,\
                         nextPageToken",
                    )
            });

            for maybe_change in lister.iter() {
                use schema::docs::dsl::*;

                let change = maybe_change?;

                let file_id = match (&change.file_id).as_ref() {
                    Some(fid) => fid,

                    // I've observed change entries that are filled with Nones
                    // for every item we request. I don't know what that
                    // means, but it seems to work OK if we just ignore them.
                    None => continue,
                };

                let removed = change.removed.unwrap_or(false);

                if removed {
                    // TODO: just save a flag, or something? NOTE: Just
                    // putting a file in the trash doesn't trigger this
                    // action. The user needs to either "Delete forever" the
                    // document from their Trash; or I think this can happen
                    // if they lose access to the document.

                    {
                        use schema::links::dsl::*;
                        diesel::delete(
                            links.filter(account_id.eq(the_account_id).and(parent_id.eq(file_id))),
                        )
                        .execute(&self.conn)?;
                        diesel::delete(
                            links.filter(account_id.eq(the_account_id).and(child_id.eq(file_id))),
                        )
                        .execute(&self.conn)?;
                    }

                    {
                        use schema::account_associations::dsl::*;
                        diesel::delete(account_associations.filter(doc_id.eq(file_id)))
                            .execute(&self.conn)?;
                    }

                    diesel::delete(docs.filter(id.eq(file_id))).execute(&self.conn)?;
                } else {
                    let file = &change.file.as_ref().ok_or_else(|| {
                        format_err!(
                            "server reported file change but did not provide its information"
                        )
                    })?;
                    let new_doc = database::NewDoc::from_api_object(file)?;
                    diesel::replace_into(schema::docs::table)
                        .values(&new_doc)
                        .execute(&self.conn)?;

                    let new_assn =
                        database::NewAccountAssociation::new(&new_doc.id, the_account_id);
                    diesel::replace_into(schema::account_associations::table)
                        .values(&new_assn)
                        .execute(&self.conn)?;

                    // Refresh the parentage information.

                    {
                        use schema::links::dsl::*;
                        diesel::delete(
                            links.filter(account_id.eq(the_account_id).and(child_id.eq(file_id))),
                        )
                        .execute(&self.conn)?;
                    }

                    if let Some(parents) = file.parents.as_ref() {
                        for pid in parents {
                            let new_link = database::NewLink::new(the_account_id, pid, file_id);
                            diesel::replace_into(schema::links::table)
                                .values(&new_link)
                                .execute(&self.conn)?;
                        }
                    }
                }
            }

            Ok(lister.into_change_page_token())
        })?;

        account.data.change_page_token = Some(token);
        account.save_to_json()?;
        Ok(())
    }

    /// Maybe synchronize the database with the cloud, depending on the
    /// `--sync` option.
    ///
    /// Ideally, the UX here would not print anything at first, but if the
    /// sync starts taking more than ~1 second, would print "synchronizing
    /// ...". That way the user knows what's going on if the program stalls,
    /// but we avoid chatter in the (common?) case that the sync is quick.
    pub fn maybe_sync_all_accounts(&mut self) -> Result<()> {
        // Could make this configurable?
        let resync_delay = Duration::minutes(5);
        let mut printed_sync_notice = false;

        for maybe_info in accounts::get_accounts()? {
            let now: DateTime<Utc> = Utc::now();
            let (email, mut account) = maybe_info?;

            let should_sync = match self.options.sync {
                SyncOption::No => false,
                SyncOption::Yes => true,
                SyncOption::Auto => {
                    if let Some(last_sync) = account.data.last_sync.as_ref() {
                        now.signed_duration_since(*last_sync) > resync_delay
                    } else {
                        true
                    }
                }
            };

            if should_sync {
                if !printed_sync_notice {
                    tcreport!(self.ps, info: "synchronizing accounts ...");
                    printed_sync_notice = true;
                }
                account.data.last_sync = Some(now);
                self.sync_account(&email, &mut account)?;
            }
        }

        Ok(())
    }

    /// Convert an iterator of document IDs into Doc structures
    ///
    /// ## Panics
    ///
    /// If any of the IDs are not found in the database!
    pub fn ids_to_docs<I: IntoIterator<Item = V>, V: AsRef<str>>(&mut self, ids: I) -> Vec<Doc> {
        ids.into_iter()
            .map(|docid| {
                use schema::docs::dsl::*;
                docs.filter(id.eq(&docid.as_ref()))
                    .first::<database::Doc>(&self.conn)
                    .unwrap()
            })
            .collect()
    }

    /// Set the virtual working directory that helps provide continuity from
    /// one CLI invocation to the next.
    pub fn set_cwd(&mut self, doc: &Doc) -> Result<()> {
        if !doc.is_folder() {
            // Maybe this should just be a panic? But we have to return Result anyway
            return Err(format_err!(
                "cannot set virtual CWD to non-folder \"{}\"",
                doc.name
            ));
        }

        use database::{NewListItem, CLI_CWD_ID};
        use schema::listitems::dsl::*;

        diesel::delete(listitems.filter(listing_id.eq(CLI_CWD_ID))).execute(&self.conn)?;

        let item = NewListItem::new(CLI_CWD_ID, 0, &doc.id);
        diesel::insert_into(listitems)
            .values(&item)
            .execute(&self.conn)?;

        Ok(())
    }

    /// Print out a list of documents.
    ///
    /// Many TODOs!
    pub fn print_doc_list(&mut self, docs: Vec<Doc>) -> Result<()> {
        // If nothing, return -- without clearing the previous cli-last-print
        // listing, if it exists.

        if docs.len() == 0 {
            return Ok(());
        }

        // Get it all into the database first.

        {
            use database::{NewListItem, CLI_LAST_PRINT_ID};
            use schema::listitems::dsl::*;

            diesel::delete(listitems.filter(listing_id.eq(CLI_LAST_PRINT_ID)))
                .execute(&self.conn)?;

            let rows: Vec<_> = docs
                .iter()
                .enumerate()
                .map(|(i, doc)| NewListItem::new(CLI_LAST_PRINT_ID, i as i32, &doc.id))
                .collect();

            diesel::insert_into(listitems)
                .values(&rows)
                .execute(&self.conn)?;
        }

        // Now print it out.

        let now = Utc::now();

        let n = docs.len();
        let n_width = format!("{}", n).len(); // <= lame
        let mut max_name_len = 0;

        for doc in &docs {
            max_name_len = std::cmp::max(max_name_len, doc.name.len());
        }

        let mut i = 1;

        for doc in &docs {
            let ago = now.signed_duration_since(doc.utc_mod_time());
            let ago = ago
                .to_std()
                .map(|stddur| timeago::Formatter::new().convert(stddur))
                .unwrap_or_else(|_err| "[future?]".to_owned());

            tcprintln!(self.ps,
                       [percent_tag: "%{1:<0$}", n_width, i],
                       ("  "),
                       {colors, {
                           if doc.trashed {
                               &colors.red
                           } else if doc.starred {
                               &colors.yellow
                           } else if doc.is_folder() {
                               &colors.folder
                           } else {
                               &colors.plain
                           }
                       }: "{1:<0$}", max_name_len, doc.name},
                       ("  {}", ago)
            );

            i += 1;
        }

        Ok(())
    }
}

/// Data about inter-document linkages.
///
/// We have a database table that can store the inter-document linkage
/// information, but I'm pretty sure that for Real Computations it quickly
/// becomes very unhealthy to do them using the database. So instead, when
/// asked we construct an in-memory `petgraph` graph from the database
/// contents.
pub struct LinkageTable {
    /// The ID of the account for which this table was constructed.
    pub account_id: i32,

    /// If true, edges point from children to parents; otherwise, they point
    /// from parents to children.
    pub transposed: bool,

    /// The graph of linkages between documents.
    pub graph: petgraph::Graph<String, (), Directed, u32>,

    /// A map from document IDs to node indices in the graph.
    pub nodes: HashMap<String, NodeIndex<u32>>,
}

impl Application {
    /// Load the table of inter-document linkages.
    ///
    /// The underlying graph is directed. If `transposed` is false, links will
    /// point from parents to children. If true, links will point from
    /// children to parents.
    pub fn load_linkage_table(&self, acct_id: i32, transposed: bool) -> Result<LinkageTable> {
        // as a dumb aliasing workaround, `acct_id` is the argument whereas
        // `account_id` is the column in the database.
        use schema::links::dsl::*;

        let mut graph = petgraph::Graph::new();
        let mut nodes = HashMap::new();

        let q = links
            .filter(account_id.eq(acct_id))
            .load::<database::Link>(&self.conn)?;

        for link in q {
            let pix = *nodes
                .entry(link.parent_id.clone())
                .or_insert_with(|| graph.add_node(link.parent_id.clone()));
            let cix = *nodes
                .entry(link.child_id.clone())
                .or_insert_with(|| graph.add_node(link.child_id.clone()));

            // The `update_edge()` function prevents duplicate edges from
            // being formed, but because the database has a primary key
            // constraint on the pair (parent_id, child_id), it should be
            // impossible for this function to attempt to create such
            // duplications.

            if transposed {
                graph.add_edge(cix, pix, ());
            } else {
                graph.add_edge(pix, cix, ());
            }
        }

        Ok(LinkageTable {
            account_id: acct_id,
            transposed,
            graph,
            nodes,
        })
    }
}

impl LinkageTable {
    /// Given a document ID, find the set of folders that contain it.
    ///
    /// This is nontrivial because in Google Drive, the folder structure can
    /// basically be an arbitrary graph -- including cycles.
    ///
    /// The *self* linkage table must have been loaded with *transpose* set to
    /// true, so that the graph edges point from children to parents.
    ///
    /// The return value is a vector of paths, because the document can have
    /// multiple parents, or one of its parents might have multiple parents.
    /// Each path is itself a vector of document IDs. The first item in the
    /// vector is the outermost folder, while the last item is the folder
    /// containing the target document. The ID of the target document is not
    /// included in the return values. The path may be an empty vector, if the
    /// document has been shared with the user's account but not "added to My
    /// Drive". This can happen in other circumstances that I do not
    /// understand (e.g. folder JupiterExample for wwt@aas.org).
    ///
    /// The algorithm here is homebrewed because I couldn't find any serious
    /// discussion of the relevant graph-thory problem. It's basically a
    /// breadth-first iteration, but it is willing to revisit nodes so long as
    /// they do not create a cycle within the path being considered.
    pub fn find_parent_paths(&self, start_id: &str) -> Vec<Vec<String>> {
        use std::collections::HashSet;

        assert_eq!(self.transposed, true);

        let roots: HashSet<NodeIndex> = self.graph.externals(Direction::Outgoing).collect();

        let start_ix = match self.nodes.get(start_id) {
            Some(ix) => *ix,
            None => return Vec::new(),
        };

        let mut queue = Vec::new();
        queue.push(start_ix);

        let mut path_data = HashMap::new();
        path_data.insert(start_ix, None);

        let mut results = Vec::new();

        while queue.len() > 0 {
            let cur_ix = queue.pop().unwrap();

            if roots.contains(&cur_ix) {
                // We finished a path!
                let mut path = Vec::new();
                let mut ix = cur_ix;

                // Can't do this as a `while let` loop since the bindings shadow
                loop {
                    if let Some(new_ix) = path_data.get(&ix).unwrap() {
                        path.push(self.graph.node_weight(ix).unwrap().clone());
                        ix = *new_ix;
                    } else {
                        break;
                    }
                }

                results.push(path);
            }

            for next_ix in self.graph.neighbors(cur_ix) {
                // Already enqueued?
                if queue.contains(&next_ix) {
                    continue;
                }

                // Check for loops.
                let mut ix = cur_ix;

                let found_loop = loop {
                    if ix == next_ix {
                        break true;
                    }

                    if let Some(new_ix) = path_data.get(&ix).unwrap() {
                        ix = *new_ix;
                    } else {
                        break false;
                    }
                };

                if found_loop {
                    continue;
                }

                // Looks like we should consider this node.

                path_data.insert(next_ix, Some(cur_ix));
                queue.push(next_ix);
            }
        }

        results
    }
}

/// A struct for specifying how we might parse command-line arguments
/// specifying zero or more documents.
pub struct GetDocBuilder<'a> {
    app: &'a mut Application,
    zero_ok: bool,
}

impl Application {
    /// Start the process of parsing some text into a list of zero or more
    /// documents.
    ///
    /// The default setting is that at least one document must match.
    pub fn get_docs<'a>(&'a mut self) -> GetDocBuilder<'a> {
        GetDocBuilder {
            app: self,
            zero_ok: false,
        }
    }
}

impl<'a> GetDocBuilder<'a> {
    /// Specify whether it is OK if the specification matches no documents.
    ///
    /// If this setting is false, an Err outcome will be returned by
    /// the `process*` functions if nothing matches.
    #[allow(unused)]
    pub fn zero_ok(mut self, setting: bool) -> Self {
        self.zero_ok = setting;
        self
    }

    /// Convert a single specification string into a list of documents,
    /// without applying any validation.
    ///
    /// If this function returns `Err`, it is because of a genuine problem
    /// talking to the database or something.
    fn process_impl(&mut self, spec: &str) -> Result<Vec<Doc>> {
        use schema::docs::dsl::*;

        // Docid exact match?
        let maybe_doc = docs.filter(id.eq(spec)).first(&self.app.conn).optional()?;

        if let Some(doc) = maybe_doc {
            return Ok(vec![doc]);
        }

        // CWD reference?
        if spec == "." {
            use database::{ListItem, CLI_CWD_ID};
            use schema::docs;
            use schema::listitems::dsl::*;

            let mut matches = listitems
                .inner_join(docs::table)
                .filter(listing_id.eq(CLI_CWD_ID))
                .load::<(ListItem, Doc)>(&self.app.conn)?;
            let matches: Vec<_> = matches.drain(0..).map(|(_row, doc)| doc).collect();

            // Note: we don't explicitly handle more than one match. Under the
            // current architecture that should never happen.
            if matches.len() < 1 {
                return Err(format_err!(
                    "the virtual CWD (\"{}\") is not currently defined",
                    spec
                ));
            }

            return Ok(matches);
        }

        // CWD-parent reference? As usual this is more annoying than you might
        // think because folders can have multiple parents.
        if spec == ".." {
            use std::collections::HashSet;

            // note: if no CWD, we'll get Err, not Ok(vec![]).
            let cwd = self.process_impl(".")?.pop().unwrap();
            let accounts = cwd.accounts(self.app)?;
            let mut parent_ids = HashSet::new();

            for acct in &accounts {
                let table = self.app.load_linkage_table(acct.id, true)?;
                for pid in table
                    .find_parent_paths(&cwd.id)
                    .iter_mut()
                    .map(|id_path| id_path.pop())
                {
                    if let Some(pid) = pid {
                        parent_ids.insert(pid);
                    }
                }
            }

            return Ok(self.app.ids_to_docs(parent_ids));
        }

        // recent-listing reference?
        if spec.starts_with("%") {
            use database::{ListItem, CLI_LAST_PRINT_ID};
            use schema::listitems::dsl::*;

            let number_text = &spec[1..];
            let index = number_text.parse::<i32>()? - 1; // 1-based to 0-based
            let maybe_row = listitems
                .filter(listing_id.eq(CLI_LAST_PRINT_ID).and(position.eq(index)))
                .first::<ListItem>(&self.app.conn)
                .optional()?;

            let matched_id = match maybe_row {
                Some(row) => row.doc_id,
                None => {
                    return Err(format_err!(
                        "\"{}\" is not a valid recent-document reference",
                        spec
                    ));
                }
            };

            let doc = docs.filter(id.eq(matched_id)).first(&self.app.conn)?;
            return Ok(vec![doc]);
        }

        // Partial doc name match?
        // TODO: ESCAPING
        let pattern = format!("%{}%", spec);
        let results = docs
            .filter(name.like(&pattern))
            .load::<Doc>(&self.app.conn)?;
        Ok(results)
    }

    /// Convert a single specification string into a list of documents.
    pub fn process<S: AsRef<str>>(mut self, spec: S) -> Result<Vec<Doc>> {
        let spec = spec.as_ref();
        let mut r = self.process_impl(spec)?;

        if !self.zero_ok && r.len() == 0 {
            return Err(format_err!(
                "no documents matched the specification \"{}\"",
                spec
            ));
        }

        // Show most recent modification first. This code could be extended to provide more
        // possibilities if so desired.
        r.sort_by_key(|d| d.utc_mod_time());
        r.reverse();

        Ok(r)
    }

    /// Convert a single specification string into a single document.
    ///
    /// If not exactly one document matches, an error is raised. In the
    /// multiple-match case, a listing is printed that is intended to help the
    /// user narrow down their search.
    pub fn process_one<S: AsRef<str>>(mut self, spec: S) -> Result<Doc> {
        let spec = spec.as_ref();
        let mut r = self.process_impl(spec)?;

        if r.len() == 0 {
            return Err(format_err!(
                "no documents matched the specification \"{}\"",
                spec
            ));
        }

        if r.len() == 1 {
            return Ok(r.pop().unwrap());
        }

        // Multiple documents matched. Print a listing, limiting the number of
        // printed results in case the listing would be super long.

        let n = r.len();
        const MAX_TO_PRINT: usize = 20;
        let truncated = n > MAX_TO_PRINT;

        if truncated {
            r.truncate(MAX_TO_PRINT);
        }

        if truncated {
            tcreport!(self.app.ps, error: "{} documents matched the specification \"{}\"; \
                                           only printing first {}\n", n, spec, MAX_TO_PRINT);
        } else {
            tcreport!(self.app.ps, error: "{} documents matched the specification \"{}\"\n", n, spec);
        }

        if let Err(e) = self.app.print_doc_list(r) {
            tcreport!(self.app.ps, error: "furthermore, could not access database: {}", e);
        }

        tcprintln!(self.app.ps, (""));
        return Err(format_err!(
            "specification should have matched exactly one document; \
             please be more specific"
        ));
    }

    /// Return a vector of all documents.
    ///
    /// This is just some syntactic sugar.
    pub fn all(self) -> Result<Vec<Doc>> {
        use schema::docs::dsl::*;
        Ok(docs.load(&self.app.conn)?)
    }
}
