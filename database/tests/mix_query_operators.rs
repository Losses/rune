use anyhow::Result;
use database::test_fixtures::{causal_library, fixtures};
use database::{
    actions::mixes::{
        QueryOperator, QueryParseError, add_item_to_mix, create_mix, executable_mix_queries,
        query_mix_media_files, replace_mix_queries,
    },
    connection::RecommendationDbConnection,
    entities::{media_files, mixes},
    error::ExpectedError,
    mix_operator::{MixOperator, UnknownOperator},
};
use sea_orm::{ActiveModelTrait, ActiveValue, Database, DatabaseConnection, EntityTrait};
use std::collections::BTreeSet;
use uuid::Uuid;

/// A parameter each operator accepts.
fn sample_parameter(operator: MixOperator) -> &'static str {
    use database::mix_operator::ParameterKind;
    match operator.parameter_kind() {
        ParameterKind::Flag => "true",
        ParameterKind::Number | ParameterKind::Limit => "7",
        ParameterKind::Reference(_) => "3",
        ParameterKind::Directory => "/music",
    }
}

#[test]
fn every_operator_round_trips_through_the_syntax() {
    for operator in MixOperator::ALL {
        let parameter = sample_parameter(operator);
        let parsed = QueryOperator::parse(operator.syntax(), parameter).unwrap();
        assert_eq!(parsed.operator(), operator);
        assert_eq!(
            parsed.syntax(),
            (operator.syntax(), parameter.to_owned()),
            "{}",
            operator.syntax()
        );
    }
}

#[test]
fn unknown_operators_and_bad_parameters_are_typed_errors() {
    assert_eq!(
        QueryOperator::parse("lib::nothing", "1"),
        Err(QueryParseError::UnknownOperator(UnknownOperator(
            "lib::nothing".into()
        )))
    );
    assert_eq!(
        QueryOperator::parse("lib::artist", "not a number"),
        Err(QueryParseError::InvalidParameter {
            operator: MixOperator::LibArtist,
            parameter: "not a number".into(),
        })
    );
    assert_eq!(
        QueryOperator::parse("pipe::limit", "-1"),
        Err(QueryParseError::InvalidParameter {
            operator: MixOperator::PipeLimit,
            parameter: "-1".into(),
        })
    );
}

#[tokio::test]
async fn clauses_come_back_from_storage_as_written_and_unknown_ones_are_refused() -> Result<()> {
    let db = Database::connect("sqlite::memory:").await?;
    let actor = Uuid::new_v4();
    fixtures::migrate(&db, actor).await?;
    let library = fixtures::open_library(db.clone(), actor, Some(Uuid::new_v4())).await?;
    let store = library.record_store();

    // Every operator that points at no stored row is written and read back unchanged.
    let clauses: Vec<(String, String)> = MixOperator::ALL
        .into_iter()
        .filter(|operator| operator.target_collection().is_none())
        .map(|operator| {
            (
                operator.syntax().to_owned(),
                sample_parameter(operator).to_owned(),
            )
        })
        .collect();
    let mix = create_mix(
        &db,
        &store,
        "operators".into(),
        "m".into(),
        None,
        false,
        clauses.clone(),
    )
    .await?;
    assert_eq!(executable_mix_queries(&db, mix.id).await?, clauses);
    // No mix is a scriptlet; the synced flag is stored as false.
    let stored = mixes::Entity::find_by_id(mix.id).one(&db).await?.unwrap();
    assert!(!stored.scriptlet_mode);

    // Text that is no operator, or an operator with a parameter it cannot read, is refused
    // before anything is stored.
    let error = replace_mix_queries(
        &db,
        &store,
        mix.id,
        &mix.source_uuid,
        vec![("lib::nothing".into(), "1".into())],
        None,
    )
    .await
    .unwrap_err();
    assert!(
        error
            .downcast_ref::<ExpectedError>()
            .is_some_and(|error| matches!(error, ExpectedError::InvalidInput { .. })),
        "{error:#}"
    );
    assert!(
        replace_mix_queries(
            &db,
            &store,
            mix.id,
            &mix.source_uuid,
            vec![("lib::all".into(), "maybe".into())],
            None,
        )
        .await
        .is_err()
    );
    assert_eq!(executable_mix_queries(&db, mix.id).await?, clauses);
    let error = add_item_to_mix(&db, &store, mix.id, "lib::nothing".into(), "1".into())
        .await
        .unwrap_err();
    assert!(
        error
            .downcast_ref::<ExpectedError>()
            .is_some_and(|error| matches!(error, ExpectedError::InvalidInput { .. }))
    );

    // A query with an operator nobody knows fails instead of being skipped.
    let error = query_mix_media_files(
        &db,
        &RecommendationDbConnection::new(),
        vec![("lib::nothing".into(), "1".into())],
        0,
        10,
        &[],
    )
    .await
    .unwrap_err();
    assert!(
        error
            .downcast_ref::<ExpectedError>()
            .is_some_and(|error| matches!(error, ExpectedError::InvalidInput { .. }))
    );
    Ok(())
}

/// The file names a single directory clause selects.
async fn directory_selection(
    db: &DatabaseConnection,
    operator: &str,
    directory: &str,
) -> Result<BTreeSet<String>> {
    let files = query_mix_media_files(
        db,
        &RecommendationDbConnection::new(),
        vec![(operator.into(), directory.into())],
        0,
        100,
        &[],
    )
    .await?;
    Ok(files.into_iter().map(|file| file.file_name).collect())
}

fn names(names: &[&str]) -> BTreeSet<String> {
    names.iter().map(|name| (*name).to_owned()).collect()
}

#[tokio::test]
async fn directory_names_are_matched_literally() -> Result<()> {
    let db = Database::connect("sqlite::memory:").await?;
    fixtures::migrate(&db, Uuid::new_v4()).await?;
    // Each file is named after its directory, so a result set reads as the directories selected.
    let directories = [
        "A%_ space",
        "A%_ space/Live",
        "Axx space",
        "Axx space/Live",
        "Rock_1",
        "Rock_1/Live",
        "RockA1/Live",
        "Esc\\%",
        "Esc\\%/Live",
        "Esc\\x/Live",
        "Music/Rock",
        "Music/Rock/Live",
        "Music/Rockabilly",
    ];
    for (id, directory) in (1..).zip(directories) {
        let file = causal_library::local_file(&db, id, Uuid::new_v4(), directory).await?;
        let mut update: media_files::ActiveModel = file.into();
        update.directory = ActiveValue::Set(directory.into());
        update.update(&db).await?;
    }

    let deep = "lib::directory.deep";
    assert_eq!(
        directory_selection(&db, deep, "/A%_ space").await?,
        names(&["A%_ space", "A%_ space/Live"])
    );
    assert_eq!(
        directory_selection(&db, deep, "/Rock_1").await?,
        names(&["Rock_1", "Rock_1/Live"])
    );
    assert_eq!(
        directory_selection(&db, deep, "/Esc\\%").await?,
        names(&["Esc\\%", "Esc\\%/Live"])
    );
    assert_eq!(
        directory_selection(&db, deep, "/Music/Rock").await?,
        names(&["Music/Rock", "Music/Rock/Live"])
    );
    assert_eq!(
        directory_selection(&db, "lib::directory.shallow", "/A%_ space").await?,
        names(&["A%_ space"])
    );
    Ok(())
}
