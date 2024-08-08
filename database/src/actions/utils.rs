use deunicode::deunicode;
use sea_orm::{DatabaseConnection, DatabaseTransaction};

pub trait DatabaseExecutor: Send + Sync {}

impl DatabaseExecutor for DatabaseConnection {}
impl DatabaseExecutor for DatabaseTransaction {}

pub fn first_char(s: &str) -> char {
    if let Some(first_char) = deunicode(s).chars().next() {
        first_char
    } else {
        '#'
    }
}

pub fn generate_group_name(x: &str) -> String {
    let c = first_char(x);

    if c.is_lowercase() {
        c.to_ascii_uppercase().to_string()
    } else if c.is_ascii_digit() || !c.is_alphabetic() {
        '#'.to_string()
    } else {
        c.to_string()
    }
}
