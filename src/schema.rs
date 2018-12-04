table! {
    docs (id) {
        id -> Text,
        name -> Text,
        starred -> Bool,
        trashed -> Bool,
        modified_time -> Timestamp,
        mime_type -> Text,
    }
}

table! {
    links (parent_id, child_id) {
        parent_id -> Text,
        child_id -> Text,
    }
}

allow_tables_to_appear_in_same_query!(
    docs,
    links,
);
