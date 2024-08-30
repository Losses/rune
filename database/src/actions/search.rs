use deunicode::deunicode;
use tantivy::doc;
use tantivy::schema::*;

use crate::connection::SearchDbConnection;

#[derive(Debug, PartialEq)]
pub enum CollectionType {
    Track,
    Artist,
    Directory,
    Album,
    Playlist,
}

impl From<CollectionType> for i64 {
    fn from(collection_type: CollectionType) -> Self {
        match collection_type {
            CollectionType::Track => 0,
            CollectionType::Artist => 1,
            CollectionType::Album => 2,
            CollectionType::Directory => 3,
            CollectionType::Playlist => 4,
        }
    }
}

impl TryFrom<i64> for CollectionType {
    type Error = &'static str;

    fn try_from(value: i64) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(CollectionType::Track),
            1 => Ok(CollectionType::Artist),
            2 => Ok(CollectionType::Album),
            3 => Ok(CollectionType::Directory),
            4 => Ok(CollectionType::Playlist),
            _ => Err("Invalid value for CollectionType"),
        }
    }
}

pub fn remove_term(
    search_db: &mut SearchDbConnection,
    r#type: CollectionType,
    id: i32,
) {
    let schema = &search_db.schema;
    let term_tid = schema.get_field("tid").unwrap();

    let tid = format!("{:?}-{:?}", r#type, id);
    let term = Term::from_field_text(term_tid, &tid);

    search_db.w.delete_term(term);
}

pub fn add_term(search_db: &mut SearchDbConnection, r#type: CollectionType, id: i32, name: &str) {
    let schema = &search_db.schema;
    let term_name = schema.get_field("name").unwrap();
    let term_id = schema.get_field("id").unwrap();
    let term_type = schema.get_field("type").unwrap();
    let term_tid = schema.get_field("tid").unwrap();

    let tid = format!("{:?}-{:?}", term_type, term_id);
    let term = Term::from_field_text(term_tid, &tid);

    search_db.w.delete_term(term);

    search_db
        .w
        .add_document(doc!(
            term_name => name,
            term_name => deunicode(name),
            term_type => Into::<i64>::into(r#type),
            term_tid => tid,
            term_id => Into::<i64>::into(id),
        ))
        .unwrap();
}
