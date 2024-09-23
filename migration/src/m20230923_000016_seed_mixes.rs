use chrono::Utc;
use sea_orm_migration::prelude::*;

use crate::m20230912_000013_create_mixes_table::Mixes;

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m20230923_000016_seed_mixes"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let insert = Query::insert()
            .into_table(Mixes::Table)
            .columns([
                Mixes::Name,
                Mixes::Group,
                Mixes::Mode,
                Mixes::Locked,
                Mixes::ScriptletMode,
                Mixes::CreatedAt,
                Mixes::UpdatedAt,
            ])
            .values_panic([
                "\u{200B}Liked".into(),
                "\u{200B}Rune".into(),
                99.into(),
                true.into(),
                false.into(),
                Utc::now().to_rfc3339().into(),
                Utc::now().to_rfc3339().into(),
            ])
            .values_panic([
                "\u{200B}Mix 1".into(),
                "\u{200B}Rune".into(),
                99.into(),
                true.into(),
                false.into(),
                Utc::now().to_rfc3339().into(),
                Utc::now().to_rfc3339().into(),
            ])
            .values_panic([
                "\u{200B}Mix 2".into(),
                "\u{200B}Rune".into(),
                99.into(),
                true.into(),
                false.into(),
                Utc::now().to_rfc3339().into(),
                Utc::now().to_rfc3339().into(),
            ])
            .values_panic([
                "\u{200B}Mix 3".into(),
                "\u{200B}Rune".into(),
                99.into(),
                true.into(),
                false.into(),
                Utc::now().to_rfc3339().into(),
                Utc::now().to_rfc3339().into(),
            ])
            .values_panic([
                "\u{200B}Mix 4".into(),
                "\u{200B}Rune".into(),
                99.into(),
                true.into(),
                false.into(),
                Utc::now().to_rfc3339().into(),
                Utc::now().to_rfc3339().into(),
            ])
            .values_panic([
                "\u{200B}Mix 5".into(),
                "\u{200B}Rune".into(),
                99.into(),
                true.into(),
                false.into(),
                Utc::now().to_rfc3339().into(),
                Utc::now().to_rfc3339().into(),
            ])
            .values_panic([
                "\u{200B}Mix 6".into(),
                "\u{200B}Rune".into(),
                99.into(),
                true.into(),
                false.into(),
                Utc::now().to_rfc3339().into(),
                Utc::now().to_rfc3339().into(),
            ])
            .values_panic([
                "\u{200B}Mix 7".into(),
                "\u{200B}Rune".into(),
                99.into(),
                true.into(),
                false.into(),
                Utc::now().to_rfc3339().into(),
                Utc::now().to_rfc3339().into(),
            ])
            .values_panic([
                "\u{200B}Mix 8".into(),
                "\u{200B}Rune".into(),
                99.into(),
                true.into(),
                false.into(),
                Utc::now().to_rfc3339().into(),
                Utc::now().to_rfc3339().into(),
            ])
            .values_panic([
                "\u{200B}Mix 9".into(),
                "\u{200B}Rune".into(),
                99.into(),
                true.into(),
                false.into(),
                Utc::now().to_rfc3339().into(),
                Utc::now().to_rfc3339().into(),
            ])
            .to_owned();

        manager.exec_stmt(insert).await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let delete = Query::delete()
            .from_table(Mixes::Table)
            .and_where(Expr::col(Mixes::Group).eq("\u{200B}Rune"))
            .and_where(Expr::col(Mixes::Locked).eq(true))
            .and_where(Expr::col(Mixes::Mode).eq(99))
            .to_owned();

        manager.exec_stmt(delete).await?;

        Ok(())
    }
}
