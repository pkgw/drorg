table! {
    account_assns (doc_id, account_id) {
        doc_id -> Text,
        account_id -> Integer,
    }
}

table! {
    accounts (id) {
        id -> Integer,
        email -> Text,
    }
}

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

joinable!(account_assns -> accounts (account_id));
joinable!(account_assns -> docs (doc_id));

allow_tables_to_appear_in_same_query!(
    account_assns,
    accounts,
    docs,
    links,
);
