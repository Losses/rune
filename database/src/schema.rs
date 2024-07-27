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
    ) -> Result<Option<media_cover_arts::Model>, sea_orm::DbErr> {
        let db = ctx.data::<DatabaseConnection>().unwrap();
        media_cover_arts::Entity::find_by_id(id).one(db).await
    }

    async fn playlist_items(&self, ctx: &Context<'_>) -> Result<Vec<playlist_items::Model>, DbErr> {
        let db = ctx.data::<DatabaseConnection>().unwrap();
        playlist_items::Entity::find().all(db).await
    }

    async fn playlist_item(
        &self,
        ctx: &Context<'_>,
        id: i32,
    ) -> Result<Option<playlist_items::Model>, DbErr> {
        let db = ctx.data::<DatabaseConnection>().unwrap();
        playlist_items::Entity::find_by_id(id).one(db).await
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

    async fn smart_playlists(
        &self,
        ctx: &Context<'_>,
    ) -> Result<Vec<smart_playlists::Model>, DbErr> {
        let db = ctx.data::<DatabaseConnection>().unwrap();
        smart_playlists::Entity::find().all(db).await
    }

    async fn smart_playlist(
        &self,
        ctx: &Context<'_>,
        id: i32,
    ) -> Result<Option<smart_playlists::Model>, DbErr> {
        let db = ctx.data::<DatabaseConnection>().unwrap();
        smart_playlists::Entity::find_by_id(id).one(db).await
    }

    async fn user_logs(&self, ctx: &Context<'_>) -> Result<Vec<user_logs::Model>, DbErr> {
        let db = ctx.data::<DatabaseConnection>().unwrap();
        user_logs::Entity::find().all(db).await
    }

    async fn user_log(
        &self,
        ctx: &Context<'_>,
        id: i32,
    ) -> Result<Option<user_logs::Model>, DbErr> {
        let db = ctx.data::<DatabaseConnection>().unwrap();
        user_logs::Entity::find_by_id(id).one(db).await
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
    ) -> Result<Vec<media_cover_arts::Model>, sea_orm::DbErr> {
        let db = ctx.data::<DatabaseConnection>().unwrap();
        self.find_related(media_cover_arts::Entity).all(db).await
    }

    async fn playlist_items(
        &self,
        ctx: &Context<'_>,
    ) -> Result<Vec<playlist_items::Model>, sea_orm::DbErr> {
        let db = ctx.data::<DatabaseConnection>().unwrap();
        self.find_related(playlist_items::Entity).all(db).await
    }

    async fn user_logs(&self, ctx: &Context<'_>) -> Result<Vec<user_logs::Model>, sea_orm::DbErr> {
        let db = ctx.data::<DatabaseConnection>().unwrap();
        self.find_related(user_logs::Entity).all(db).await
    }
}

#[ComplexObject]
impl playlists::Model {
    async fn items(&self, ctx: &Context<'_>) -> Result<Vec<playlist_items::Model>, sea_orm::DbErr> {
        let db = ctx.data::<DatabaseConnection>().unwrap();
        self.find_related(playlist_items::Entity).all(db).await
    }
}

#[Object]
impl MutationRoot {
    async fn add_media_analysis(
        &self,
        ctx: &Context<'_>,
        file_id: i32,
        sample_rate: i32,
        duration: f64,
        spectral_centroid: Option<f64>,
        spectral_flatness: Option<f64>,
        spectral_slope: Option<f64>,
        spectral_rolloff: Option<f64>,
        spectral_spread: Option<f64>,
        spectral_skewness: Option<f64>,
        spectral_kurtosis: Option<f64>,
        chroma0: Option<f64>,
        chroma1: Option<f64>,
        chroma2: Option<f64>,
        chroma3: Option<f64>,
        chroma4: Option<f64>,
        chroma5: Option<f64>,
        chroma6: Option<f64>,
        chroma7: Option<f64>,
        chroma8: Option<f64>,
        chroma9: Option<f64>,
        chroma10: Option<f64>,
        chroma11: Option<f64>
    ) -> Result<media_analysis::Model, DbErr> {
        let db = ctx.data::<DatabaseConnection>().unwrap();

        let new_media_analysis = media_analysis::ActiveModel {
            file_id: ActiveValue::Set(file_id),
            sample_rate: ActiveValue::Set(sample_rate),
            duration: ActiveValue::Set(duration),
            spectral_centroid: ActiveValue::Set(spectral_centroid),
            spectral_flatness: ActiveValue::Set(spectral_flatness),
            spectral_slope: ActiveValue::Set(spectral_slope),
            spectral_rolloff: ActiveValue::Set(spectral_rolloff),
            spectral_spread: ActiveValue::Set(spectral_spread),
            spectral_skewness: ActiveValue::Set(spectral_skewness),
            spectral_kurtosis: ActiveValue::Set(spectral_kurtosis),
            chroma0: ActiveValue::Set(chroma0),
            chroma1: ActiveValue::Set(chroma1),
            chroma2: ActiveValue::Set(chroma2),
            chroma3: ActiveValue::Set(chroma3),
            chroma4: ActiveValue::Set(chroma4),
            chroma5: ActiveValue::Set(chroma5),
            chroma6: ActiveValue::Set(chroma6),
            chroma7: ActiveValue::Set(chroma7),
            chroma8: ActiveValue::Set(chroma8),
            chroma9: ActiveValue::Set(chroma9),
            chroma10: ActiveValue::Set(chroma10),
            chroma11: ActiveValue::Set(chroma11),
            ..Default::default()
        };

        let res = media_analysis::Entity::insert(new_media_analysis).exec(db).await?;

        media_analysis::Entity::find_by_id(res.last_insert_id).one(db).await.map(|b| b.unwrap())
    }

    async fn add_media_file(
        &self,
        ctx: &Context<'_>,
        file_name: String,
        directory: String,
        extension: String,
        file_hash: String,
        last_modified: String
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

        media_files::Entity::find_by_id(res.last_insert_id).one(db).await.map(|b| b.unwrap())
    }

    async fn add_media_metadata(
        &self,
        ctx: &Context<'_>,
        file_id: i32,
        meta_key: String,
        meta_value: String
    ) -> Result<media_metadata::Model, DbErr> {
        let db = ctx.data::<DatabaseConnection>().unwrap();

        let new_media_metadata = media_metadata::ActiveModel {
            file_id: ActiveValue::Set(file_id),
            meta_key: ActiveValue::Set(meta_key),
            meta_value: ActiveValue::Set(meta_value),
            ..Default::default()
        };

        let res = media_metadata::Entity::insert(new_media_metadata).exec(db).await?;

        media_metadata::Entity::find_by_id(res.last_insert_id).one(db).await.map(|b| b.unwrap())
    }

    async fn add_playlist_item(
        &self,
        ctx: &Context<'_>,
        playlist_id: i32,
        file_id: i32,
        position: i32
    ) -> Result<playlist_items::Model, DbErr> {
        let db = ctx.data::<DatabaseConnection>().unwrap();

        let new_playlist_item = playlist_items::ActiveModel {
            playlist_id: ActiveValue::Set(playlist_id),
            file_id: ActiveValue::Set(file_id),
            position: ActiveValue::Set(position),
            ..Default::default()
        };

        let res = playlist_items::Entity::insert(new_playlist_item).exec(db).await?;

        playlist_items::Entity::find_by_id(res.last_insert_id).one(db).await.map(|b| b.unwrap())
    }

    async fn add_playlist(
        &self,
        ctx: &Context<'_>,
        name: String,
        created_at: String,
        updated_at: String
    ) -> Result<playlists::Model, DbErr> {
        let db = ctx.data::<DatabaseConnection>().unwrap();

        let new_playlist = playlists::ActiveModel {
            name: ActiveValue::Set(name),
            created_at: ActiveValue::Set(created_at),
            updated_at: ActiveValue::Set(updated_at),
            ..Default::default()
        };

        let res = playlists::Entity::insert(new_playlist).exec(db).await?;

        playlists::Entity::find_by_id(res.last_insert_id).one(db).await.map(|b| b.unwrap())
    }

    async fn add_smart_playlist(
        &self,
        ctx: &Context<'_>,
        name: String,
        query: String,
        created_at: String,
        updated_at: String
    ) -> Result<smart_playlists::Model, DbErr> {
        let db = ctx.data::<DatabaseConnection>().unwrap();

        let new_smart_playlist = smart_playlists::ActiveModel {
            name: ActiveValue::Set(name),
            query: ActiveValue::Set(query),
            created_at: ActiveValue::Set(created_at),
            updated_at: ActiveValue::Set(updated_at),
            ..Default::default()
        };

        let res = smart_playlists::Entity::insert(new_smart_playlist).exec(db).await?;

        smart_playlists::Entity::find_by_id(res.last_insert_id).one(db).await.map(|b| b.unwrap())
    }

    async fn add_user_log(
        &self,
        ctx: &Context<'_>,
        file_id: i32,
        listen_time: String,
        progress: f64
    ) -> Result<user_logs::Model, DbErr> {
        let db = ctx.data::<DatabaseConnection>().unwrap();

        let new_user_log = user_logs::ActiveModel {
            file_id: ActiveValue::Set(file_id),
            listen_time: ActiveValue::Set(listen_time),
            progress: ActiveValue::Set(progress),
            ..Default::default()
        };

        let res = user_logs::Entity::insert(new_user_log).exec(db).await?;

        user_logs::Entity::find_by_id(res.last_insert_id).one(db).await.map(|b| b.unwrap())
    }
}