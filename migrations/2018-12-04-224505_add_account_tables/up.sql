CREATE TABLE accounts (
  id INTEGER PRIMARY KEY NOT NULL,
  email TEXT NOT NULL
);

CREATE TABLE account_assns (
  doc_id TEXT NOT NULL,
  account_id INTEGER NOT NULL,
  PRIMARY KEY (doc_id, account_id),
  FOREIGN KEY (account_id) REFERENCES accounts(id),
  FOREIGN KEY (doc_id) REFERENCES docs(id)
);
