table! {
    docs (id) {
        id -> Text,
        name -> Text,
        mime_type -> Text,
        starred -> Bool,
        trashed -> Bool,
        modified_time -> Timestamp,
    }
}
