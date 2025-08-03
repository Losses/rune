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
                    .col(ColumnDef::new(MediaAnalysis::Rms).double())
                    .col(ColumnDef::new(MediaAnalysis::Zcr).double())
                    .col(ColumnDef::new(MediaAnalysis::Energy).double())
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
                    .col(ColumnDef::new(MediaAnalysis::PerceptualSpread).double())
                    .col(ColumnDef::new(MediaAnalysis::PerceptualSharpness).double())
                    .col(ColumnDef::new(MediaAnalysis::PerceptualLoudness0).double())
                    .col(ColumnDef::new(MediaAnalysis::PerceptualLoudness1).double())
                    .col(ColumnDef::new(MediaAnalysis::PerceptualLoudness2).double())
                    .col(ColumnDef::new(MediaAnalysis::PerceptualLoudness3).double())
                    .col(ColumnDef::new(MediaAnalysis::PerceptualLoudness4).double())
                    .col(ColumnDef::new(MediaAnalysis::PerceptualLoudness5).double())
                    .col(ColumnDef::new(MediaAnalysis::PerceptualLoudness6).double())
                    .col(ColumnDef::new(MediaAnalysis::PerceptualLoudness7).double())
                    .col(ColumnDef::new(MediaAnalysis::PerceptualLoudness8).double())
                    .col(ColumnDef::new(MediaAnalysis::PerceptualLoudness9).double())
                    .col(ColumnDef::new(MediaAnalysis::PerceptualLoudness10).double())
                    .col(ColumnDef::new(MediaAnalysis::PerceptualLoudness11).double())
                    .col(ColumnDef::new(MediaAnalysis::PerceptualLoudness12).double())
                    .col(ColumnDef::new(MediaAnalysis::PerceptualLoudness13).double())
                    .col(ColumnDef::new(MediaAnalysis::PerceptualLoudness14).double())
                    .col(ColumnDef::new(MediaAnalysis::PerceptualLoudness15).double())
                    .col(ColumnDef::new(MediaAnalysis::PerceptualLoudness16).double())
                    .col(ColumnDef::new(MediaAnalysis::PerceptualLoudness17).double())
                    .col(ColumnDef::new(MediaAnalysis::PerceptualLoudness18).double())
                    .col(ColumnDef::new(MediaAnalysis::PerceptualLoudness19).double())
                    .col(ColumnDef::new(MediaAnalysis::PerceptualLoudness20).double())
                    .col(ColumnDef::new(MediaAnalysis::PerceptualLoudness21).double())
                    .col(ColumnDef::new(MediaAnalysis::PerceptualLoudness22).double())
                    .col(ColumnDef::new(MediaAnalysis::PerceptualLoudness23).double())
                    .col(ColumnDef::new(MediaAnalysis::Mfcc0).double())
                    .col(ColumnDef::new(MediaAnalysis::Mfcc1).double())
                    .col(ColumnDef::new(MediaAnalysis::Mfcc2).double())
                    .col(ColumnDef::new(MediaAnalysis::Mfcc3).double())
                    .col(ColumnDef::new(MediaAnalysis::Mfcc4).double())
                    .col(ColumnDef::new(MediaAnalysis::Mfcc5).double())
                    .col(ColumnDef::new(MediaAnalysis::Mfcc6).double())
                    .col(ColumnDef::new(MediaAnalysis::Mfcc7).double())
                    .col(ColumnDef::new(MediaAnalysis::Mfcc8).double())
                    .col(ColumnDef::new(MediaAnalysis::Mfcc9).double())
                    .col(ColumnDef::new(MediaAnalysis::Mfcc10).double())
                    .col(ColumnDef::new(MediaAnalysis::Mfcc11).double())
                    .col(ColumnDef::new(MediaAnalysis::Mfcc12).double())
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk-media_analysis-file_id")
                            .from(MediaAnalysis::Table, MediaAnalysis::FileId)
                            .to(MediaFiles::Table, MediaFiles::Id)
                            .on_delete(ForeignKeyAction::Cascade)
                            .on_update(ForeignKeyAction::Cascade),
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

#[derive(Iden, Clone, Copy)]
pub enum MediaAnalysis {
    Table,
    Id,
    FileId,
    Rms,
    Zcr,
    Energy,
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
    PerceptualSpread,
    PerceptualSharpness,
    PerceptualLoudness0,
    PerceptualLoudness1,
    PerceptualLoudness2,
    PerceptualLoudness3,
    PerceptualLoudness4,
    PerceptualLoudness5,
    PerceptualLoudness6,
    PerceptualLoudness7,
    PerceptualLoudness8,
    PerceptualLoudness9,
    PerceptualLoudness10,
    PerceptualLoudness11,
    PerceptualLoudness12,
    PerceptualLoudness13,
    PerceptualLoudness14,
    PerceptualLoudness15,
    PerceptualLoudness16,
    PerceptualLoudness17,
    PerceptualLoudness18,
    PerceptualLoudness19,
    PerceptualLoudness20,
    PerceptualLoudness21,
    PerceptualLoudness22,
    PerceptualLoudness23,
    Mfcc0,
    Mfcc1,
    Mfcc2,
    Mfcc3,
    Mfcc4,
    Mfcc5,
    Mfcc6,
    Mfcc7,
    Mfcc8,
    Mfcc9,
    Mfcc10,
    Mfcc11,
    Mfcc12,
}
