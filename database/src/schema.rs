use crate::entities::*;
use async_graphql::{ComplexObject, Context, Object, Result};
use sea_orm::*;
use sea_orm::{DatabaseConnection, DbErr, EntityTrait};

pub(crate) struct QueryRoot;
pub(crate) struct MutationRoot;

#[Object]
impl QueryRoot {
    async fn media_analyses(&self, ctx: &Context<'_>) -> Result<Vec<media_analysis::Model>, DbErr> {
        let db = ctx.data::<DatabaseConnection>().unwrap();
        media_analysis::Entity::find().all(db).await
    }

    async fn media_analysis(
        &self,
        ctx: &Context<'_>,
        id: i32,
    ) -> Result<Option<media_analysis::Model>, DbErr> {
        let db = ctx.data::<DatabaseConnection>().unwrap();
        media_analysis::Entity::find_by_id(id).one(db).await
    }

    async fn media_files(&self, ctx: &Context<'_>) -> Result<Vec<media_files::Model>, DbErr> {
        let db = ctx.data::<DatabaseConnection>().unwrap();
        media_files::Entity::find().all(db).await
    }

    async fn media_file(
        &self,
        ctx: &Context<'_>,
        id: i32,
    ) -> Result<Option<media_files::Model>, DbErr> {
        let db = ctx.data::<DatabaseConnection>().unwrap();
        media_files::Entity::find_by_id(id).one(db).await
    }

    async fn media_metadatas(
        &self,
        ctx: &Context<'_>,
    ) -> Result<Vec<media_metadata::Model>, DbErr> {
        let db = ctx.data::<DatabaseConnection>().unwrap();
        media_metadata::Entity::find().all(db).await
    }

    async fn media_metadata(
        &self,
        ctx: &Context<'_>,
        id: i32,
    ) -> Result<Option<media_metadata::Model>, DbErr> {
        let db = ctx.data::<DatabaseConnection>().unwrap();
        media_metadata::Entity::find_by_id(id).one(db).await
    }

    async fn media_cover_art(
        &self,
        ctx: &Context<'_>,
        id: i32,
    ) -> Result<Option<media_cover_art::Model>, sea_orm::DbErr> {
        let db = ctx.data::<DatabaseConnection>().unwrap();
        media_cover_art::Entity::find_by_id(id).one(db).await
    }

    async fn playlist_items(
        &self,
        ctx: &Context<'_>,
    ) -> Result<Vec<media_file_playlists::Model>, DbErr> {
        let db = ctx.data::<DatabaseConnection>().unwrap();
        media_file_playlists::Entity::find().all(db).await
    }

    async fn playlist_item(
        &self,
        ctx: &Context<'_>,
        id: i32,
    ) -> Result<Option<media_file_playlists::Model>, DbErr> {
        let db = ctx.data::<DatabaseConnection>().unwrap();
        media_file_playlists::Entity::find_by_id(id).one(db).await
    }

    async fn playlists(&self, ctx: &Context<'_>) -> Result<Vec<playlists::Model>, DbErr> {
        let db = ctx.data::<DatabaseConnection>().unwrap();
        playlists::Entity::find().all(db).await
    }

    async fn playlist(
        &self,
        ctx: &Context<'_>,
        id: i32,
    ) -> Result<Option<playlists::Model>, DbErr> {
        let db = ctx.data::<DatabaseConnection>().unwrap();
        playlists::Entity::find_by_id(id).one(db).await
    }
}

#[ComplexObject]
impl media_files::Model {
    async fn analyses(
        &self,
        ctx: &Context<'_>,
    ) -> Result<Vec<media_analysis::Model>, sea_orm::DbErr> {
        let db = ctx.data::<DatabaseConnection>().unwrap();
        self.find_related(media_analysis::Entity).all(db).await
    }

    async fn metadata(
        &self,
        ctx: &Context<'_>,
    ) -> Result<Vec<media_metadata::Model>, sea_orm::DbErr> {
        let db = ctx.data::<DatabaseConnection>().unwrap();
        self.find_related(media_metadata::Entity).all(db).await
    }

    async fn cover_art(
        &self,
        ctx: &Context<'_>,
    ) -> Result<Vec<media_cover_art::Model>, sea_orm::DbErr> {
        let db = ctx.data::<DatabaseConnection>().unwrap();
        self.find_related(media_cover_art::Entity).all(db).await
    }

    async fn playlist_items(
        &self,
        ctx: &Context<'_>,
    ) -> Result<Vec<media_file_playlists::Model>, sea_orm::DbErr> {
        let db = ctx.data::<DatabaseConnection>().unwrap();
        self.find_related(media_file_playlists::Entity)
            .all(db)
            .await
    }
}

#[ComplexObject]
impl playlists::Model {
    async fn items(
        &self,
        ctx: &Context<'_>,
    ) -> Result<Vec<media_file_playlists::Model>, sea_orm::DbErr> {
        let db = ctx.data::<DatabaseConnection>().unwrap();
        self.find_related(media_file_playlists::Entity)
            .all(db)
            .await
    }
}

#[Object]
impl MutationRoot {
    async fn add_media_file(
        &self,
        ctx: &Context<'_>,
        file_name: String,
        directory: String,
        extension: String,
        file_hash: String,
        last_modified: String,
    ) -> Result<media_files::Model, DbErr> {
        let db = ctx.data::<DatabaseConnection>().unwrap();

        let new_media_file = media_files::ActiveModel {
            file_name: ActiveValue::Set(file_name),
            directory: ActiveValue::Set(directory),
            extension: ActiveValue::Set(extension),
            file_hash: ActiveValue::Set(file_hash),
            last_modified: ActiveValue::Set(last_modified),
            ..Default::default()
        };

        let res = media_files::Entity::insert(new_media_file).exec(db).await?;

        media_files::Entity::find_by_id(res.last_insert_id)
            .one(db)
            .await
            .map(|b| b.unwrap())
    }

    async fn add_media_metadata(
        &self,
        ctx: &Context<'_>,
        file_id: i32,
        meta_key: String,
        meta_value: String,
    ) -> Result<media_metadata::Model, DbErr> {
        let db = ctx.data::<DatabaseConnection>().unwrap();

        let new_media_metadata = media_metadata::ActiveModel {
            file_id: ActiveValue::Set(file_id),
            meta_key: ActiveValue::Set(meta_key),
            meta_value: ActiveValue::Set(meta_value),
            ..Default::default()
        };

        let res = media_metadata::Entity::insert(new_media_metadata)
            .exec(db)
            .await?;

        media_metadata::Entity::find_by_id(res.last_insert_id)
            .one(db)
            .await
            .map(|b| b.unwrap())
    }

    async fn add_playlist_item(
        &self,
        ctx: &Context<'_>,
        playlist_id: i32,
        media_file_id: i32,
        position: i32,
    ) -> Result<media_file_playlists::Model, DbErr> {
        let db = ctx.data::<DatabaseConnection>().unwrap();

        let new_playlist_item = media_file_playlists::ActiveModel {
            playlist_id: ActiveValue::Set(playlist_id),
            media_file_id: ActiveValue::Set(media_file_id),
            position: ActiveValue::Set(position),
            ..Default::default()
        };

        let res = media_file_playlists::Entity::insert(new_playlist_item)
            .exec(db)
            .await?;

        media_file_playlists::Entity::find_by_id(res.last_insert_id)
            .one(db)
            .await
            .map(|b| b.unwrap())
    }

    async fn add_playlist(
        &self,
        ctx: &Context<'_>,
        name: String,
        created_at: String,
        updated_at: String,
    ) -> Result<playlists::Model, DbErr> {
        let db = ctx.data::<DatabaseConnection>().unwrap();

        let new_playlist = playlists::ActiveModel {
            name: ActiveValue::Set(name),
            created_at: ActiveValue::Set(created_at),
            updated_at: ActiveValue::Set(updated_at),
            ..Default::default()
        };

        let res = playlists::Entity::insert(new_playlist).exec(db).await?;

        playlists::Entity::find_by_id(res.last_insert_id)
            .one(db)
            .await
            .map(|b| b.unwrap())
    }
}
