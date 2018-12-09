// Copyright 2018 Peter Williams <peter@newton.cx>
// Licensed under the MIT License.

//! The main application state.

use diesel::prelude::*;
use diesel::sqlite::SqliteConnection;
use petgraph::prelude::*;
use std::collections::HashMap;
use tcprint::{BasicColors, ColorPrintState};
use yup_oauth2::ApplicationSecret;

use accounts::{self, Account};
use database;
use errors::Result;
use google_apis;
use schema;


/// The state of the application.
pub struct Application {
    /// The secret we use to identify this client to Google.
    pub secret: ApplicationSecret,

    /// Our connection to the database of document information.
    pub conn: SqliteConnection,

    /// The state object for colorized terminal output.
    pub ps: ColorPrintState<BasicColors>,
}


impl Application {
    /// Initialize the application.
    pub fn initialize() -> Result<Application> {
        let secret = google_apis::get_app_secret()?;
        let conn = database::get_db_connection()?;
        let ps = ColorPrintState::default();

        Ok(Application {
            secret,
            conn,
            ps
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
                    call.param("fields", "id,mimeType,modifiedTime,name,parents,\
                                          starred,trashed")
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
                call.spaces("drive")
                    .param("fields", "files(id,mimeType,modifiedTime,name,parents,\
                                      starred,trashed),nextPageToken")
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
        account.save_to_json()?;
        Ok(())
    }


    /// Synchronize the database with recent changes in this account.
    pub fn sync_account(&mut self, email: &str, account: &mut Account) -> Result<()> {
        let the_account_id = account.data.db_id; // borrowck fun

        let token = account.data.change_page_token.take().ok_or(
            format_err!("no change-paging token for {}", email)
        )?;

        let token = account.with_drive_hub(&self.secret, |hub| {
            let mut lister = google_apis::list_changes(
                &hub, &token,
                |call| call.spaces("drive")
                    .supports_team_drives(true)
                    .include_team_drive_items(true)
                    .include_removed(true)
                    .include_corpus_removals(true)
                    .param("fields", "changes(file(id,mimeType,modifiedTime,name,parents,\
                                      starred,trashed),fileId,removed),newStartPageToken,\
                                      nextPageToken")
            );

            for maybe_change in lister.iter() {
                use schema::docs::dsl::*;

                let change = maybe_change?;

                let removed = change.removed.unwrap_or(false);
                let file_id = (&change.file_id).as_ref().ok_or_else(
                    || format_err!("no file_id provided with change reported by the server")
                )?;

                if removed {
                    // TODO: just save a flag, or something? NOTE: Just
                    // putting a file in the trash doesn't trigger this
                    // action. The user needs to either "Delete forever" the
                    // document from their Trash; or I think this can happen
                    // if they lose access to the document.

                    {
                        use schema::links::dsl::*;
                        diesel::delete(links.filter(account_id.eq(the_account_id)
                                                    .and(parent_id.eq(file_id))))
                            .execute(&self.conn)?;
                        diesel::delete(links.filter(account_id.eq(the_account_id)
                                                    .and(child_id.eq(file_id))))
                            .execute(&self.conn)?;
                    }

                    {
                        use schema::account_associations::dsl::*;
                        diesel::delete(account_associations.filter(doc_id.eq(file_id)))
                            .execute(&self.conn)?;
                    }

                    diesel::delete(docs.filter(id.eq(file_id)))
                        .execute(&self.conn)?;
                } else {
                    let file = &change.file.as_ref().ok_or_else(
                        || format_err!("server reported file change but did not provide its information")
                    )?;
                    let new_doc = database::NewDoc::from_api_object(file)?;
                    diesel::replace_into(schema::docs::table)
                        .values(&new_doc)
                        .execute(&self.conn)?;

                    let new_assn = database::NewAccountAssociation::new(&new_doc.id, the_account_id);
                    diesel::replace_into(schema::account_associations::table)
                        .values(&new_assn)
                        .execute(&self.conn)?;

                    // Refresh the parentage information.

                    {
                        use schema::links::dsl::*;
                        diesel::delete(links.filter(account_id.eq(the_account_id)
                                                    .and(child_id.eq(file_id))))
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


    /// Synchronize the database with recent changes to all accounts.
    pub fn sync_all_accounts(&mut self) -> Result<()> {
        for maybe_info in accounts::get_accounts()? {
            let (email, mut account) = maybe_info?;
            self.sync_account(&email, &mut account)?;
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
    pub nodes: HashMap<String, NodeIndex<u32>>
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

        let q = links.filter(account_id.eq(acct_id))
            .load::<database::Link>(&self.conn)?;

        for link in q {
            let pix = *nodes.entry(link.parent_id.clone()).or_insert_with(
                || graph.add_node(link.parent_id.clone())
            );
            let cix = *nodes.entry(link.child_id.clone()).or_insert_with(
                || graph.add_node(link.child_id.clone())
            );

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

        Ok(LinkageTable { account_id: acct_id, transposed, graph, nodes })
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
            None => return Vec::new()
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
