CREATE TABLE accounts (
  id INTEGER PRIMARY KEY NOT NULL,
  email TEXT NOT NULL
);

CREATE TABLE docs (
  id TEXT PRIMARY KEY NOT NULL,
  name TEXT NOT NULL,
  mime_type TEXT NOT NULL,
  modified_time DATETIME NOT NULL,
  starred BOOLEAN NOT NULL,
  trashed BOOLEAN NOT NULL
);

CREATE TABLE account_associations (
  doc_id TEXT NOT NULL,
  account_id INTEGER NOT NULL,
  PRIMARY KEY (doc_id, account_id),
  FOREIGN KEY (account_id) REFERENCES accounts(id),
  FOREIGN KEY (doc_id) REFERENCES docs(id)
);

CREATE TABLE links (
  account_id INTEGER NOT NULL,
  parent_id TEXT NOT NULL,
  child_id TEXT NOT NULL,
  PRIMARY KEY (account_id, parent_id, child_id),
  FOREIGN KEY (account_id) REFERENCES accounts(id),
  FOREIGN KEY (parent_id) REFERENCES docs(id),
  FOREIGN KEY (child_id) REFERENCES docs(id)
);
