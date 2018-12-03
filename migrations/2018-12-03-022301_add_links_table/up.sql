CREATE TABLE links (
  parent_id TEXT NOT NULL,
  child_id TEXT NOT NULL,
  PRIMARY KEY (parent_id, child_id),
  FOREIGN KEY (parent_id) REFERENCES docs(id),
  FOREIGN KEY (child_id) REFERENCES docs(id)
);
