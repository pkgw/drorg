CREATE TABLE listitems (
  listing_id INTEGER NOT NULL,
  position INTEGER NOT NULL,
  doc_id TEXT NOT NULL,

  PRIMARY KEY (listing_id, position),
  FOREIGN KEY (doc_id) REFERENCES docs(id)
);
