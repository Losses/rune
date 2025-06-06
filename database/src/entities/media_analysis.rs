//! `SeaORM` Entity, @generated by sea-orm-codegen 1.1.0

use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
#[sea_orm(table_name = "media_analysis")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    pub file_id: i32,
    pub rms: Option<Decimal>,
    pub zcr: Option<Decimal>,
    pub energy: Option<Decimal>,
    pub spectral_centroid: Option<Decimal>,
    pub spectral_flatness: Option<Decimal>,
    pub spectral_slope: Option<Decimal>,
    pub spectral_rolloff: Option<Decimal>,
    pub spectral_spread: Option<Decimal>,
    pub spectral_skewness: Option<Decimal>,
    pub spectral_kurtosis: Option<Decimal>,
    pub chroma0: Option<Decimal>,
    pub chroma1: Option<Decimal>,
    pub chroma2: Option<Decimal>,
    pub chroma3: Option<Decimal>,
    pub chroma4: Option<Decimal>,
    pub chroma5: Option<Decimal>,
    pub chroma6: Option<Decimal>,
    pub chroma7: Option<Decimal>,
    pub chroma8: Option<Decimal>,
    pub chroma9: Option<Decimal>,
    pub chroma10: Option<Decimal>,
    pub chroma11: Option<Decimal>,
    pub perceptual_spread: Option<Decimal>,
    pub perceptual_sharpness: Option<Decimal>,
    pub perceptual_loudness0: Option<Decimal>,
    pub perceptual_loudness1: Option<Decimal>,
    pub perceptual_loudness2: Option<Decimal>,
    pub perceptual_loudness3: Option<Decimal>,
    pub perceptual_loudness4: Option<Decimal>,
    pub perceptual_loudness5: Option<Decimal>,
    pub perceptual_loudness6: Option<Decimal>,
    pub perceptual_loudness7: Option<Decimal>,
    pub perceptual_loudness8: Option<Decimal>,
    pub perceptual_loudness9: Option<Decimal>,
    pub perceptual_loudness10: Option<Decimal>,
    pub perceptual_loudness11: Option<Decimal>,
    pub perceptual_loudness12: Option<Decimal>,
    pub perceptual_loudness13: Option<Decimal>,
    pub perceptual_loudness14: Option<Decimal>,
    pub perceptual_loudness15: Option<Decimal>,
    pub perceptual_loudness16: Option<Decimal>,
    pub perceptual_loudness17: Option<Decimal>,
    pub perceptual_loudness18: Option<Decimal>,
    pub perceptual_loudness19: Option<Decimal>,
    pub perceptual_loudness20: Option<Decimal>,
    pub perceptual_loudness21: Option<Decimal>,
    pub perceptual_loudness22: Option<Decimal>,
    pub perceptual_loudness23: Option<Decimal>,
    pub mfcc0: Option<Decimal>,
    pub mfcc1: Option<Decimal>,
    pub mfcc2: Option<Decimal>,
    pub mfcc3: Option<Decimal>,
    pub mfcc4: Option<Decimal>,
    pub mfcc5: Option<Decimal>,
    pub mfcc6: Option<Decimal>,
    pub mfcc7: Option<Decimal>,
    pub mfcc8: Option<Decimal>,
    pub mfcc9: Option<Decimal>,
    pub mfcc10: Option<Decimal>,
    pub mfcc11: Option<Decimal>,
    pub mfcc12: Option<Decimal>,
    pub hlc_uuid: String,
    #[sea_orm(column_type = "Text")]
    pub created_at_hlc_ts: String,
    pub created_at_hlc_ver: i32,
    #[sea_orm(column_type = "Text")]
    pub created_at_hlc_nid: String,
    #[sea_orm(column_type = "Text")]
    pub updated_at_hlc_ts: String,
    pub updated_at_hlc_ver: i32,
    #[sea_orm(column_type = "Text")]
    pub updated_at_hlc_nid: String,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::media_files::Entity",
        from = "Column::FileId",
        to = "super::media_files::Column::Id",
        on_update = "Cascade",
        on_delete = "Cascade"
    )]
    MediaFiles,
}

impl Related<super::media_files::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::MediaFiles.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
