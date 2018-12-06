table! {
    account_associations (doc_id, account_id) {
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
        mime_type -> Text,
        modified_time -> Timestamp,
        starred -> Bool,
        trashed -> Bool,
    }
}

table! {
    links (account_id, parent_id, child_id) {
        account_id -> Integer,
        parent_id -> Text,
        child_id -> Text,
    }
}

joinable!(account_associations -> accounts (account_id));
joinable!(account_associations -> docs (doc_id));
joinable!(links -> accounts (account_id));

allow_tables_to_appear_in_same_query!(
    account_associations,
    accounts,
    docs,
    links,
);
