use sea_orm_migration::prelude::*;

use super::m20230701_000001_create_media_files_table::MediaFiles;

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m20230701_000003_create_media_analysis_table"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(MediaAnalysis::Table)
                    .col(
                        ColumnDef::new(MediaAnalysis::Id)
                            .integer()
                            .not_null()
                            .auto_increment()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(MediaAnalysis::FileId).integer().not_null())
                    .col(ColumnDef::new(MediaAnalysis::SampleRate).integer().not_null())
                    .col(ColumnDef::new(MediaAnalysis::Duration).double().not_null())
                    .col(ColumnDef::new(MediaAnalysis::SpectralCentroid).double())
                    .col(ColumnDef::new(MediaAnalysis::SpectralFlatness).double())
                    .col(ColumnDef::new(MediaAnalysis::SpectralSlope).double())
                    .col(ColumnDef::new(MediaAnalysis::SpectralRolloff).double())
                    .col(ColumnDef::new(MediaAnalysis::SpectralSpread).double())
                    .col(ColumnDef::new(MediaAnalysis::SpectralSkewness).double())
                    .col(ColumnDef::new(MediaAnalysis::SpectralKurtosis).double())
                    .col(ColumnDef::new(MediaAnalysis::Chroma0).double())
                    .col(ColumnDef::new(MediaAnalysis::Chroma1).double())
                    .col(ColumnDef::new(MediaAnalysis::Chroma2).double())
                    .col(ColumnDef::new(MediaAnalysis::Chroma3).double())
                    .col(ColumnDef::new(MediaAnalysis::Chroma4).double())
                    .col(ColumnDef::new(MediaAnalysis::Chroma5).double())
                    .col(ColumnDef::new(MediaAnalysis::Chroma6).double())
                    .col(ColumnDef::new(MediaAnalysis::Chroma7).double())
                    .col(ColumnDef::new(MediaAnalysis::Chroma8).double())
                    .col(ColumnDef::new(MediaAnalysis::Chroma9).double())
                    .col(ColumnDef::new(MediaAnalysis::Chroma10).double())
                    .col(ColumnDef::new(MediaAnalysis::Chroma11).double())
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk-media_analysis-file_id")
                            .from(MediaAnalysis::Table, MediaAnalysis::FileId)
                            .to(MediaFiles::Table, MediaFiles::Id),
                    )
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(MediaAnalysis::Table).to_owned())
            .await
    }
}

#[derive(Iden)]
pub enum MediaAnalysis {
    Table,
    Id,
    FileId,
    SampleRate,
    Duration,
    SpectralCentroid,
    SpectralFlatness,
    SpectralSlope,
    SpectralRolloff,
    SpectralSpread,
    SpectralSkewness,
    SpectralKurtosis,
    Chroma0,
    Chroma1,
    Chroma2,
    Chroma3,
    Chroma4,
    Chroma5,
    Chroma6,
    Chroma7,
    Chroma8,
    Chroma9,
    Chroma10,
    Chroma11,
}
