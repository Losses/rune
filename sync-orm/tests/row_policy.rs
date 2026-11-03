#[path = "support/fixtures.rs"]
mod fixtures;

use sea_orm::sea_query as sea_orm_sea_query;
use sea_orm::{ConnectionTrait, Database, EntityTrait, Schema, TransactionTrait, sea_query::Expr};
use sync::protocol::{ObjectKey, SyncModel};
use uuid::Uuid;

#[tokio::test]
async fn metadata_reads_do_not_evaluate_unselected_local_columns() -> anyhow::Result<()> {
    use sea_orm::sea_query::{Alias, Query};
    let db = Database::connect("sqlite::memory:").await?;
    let mut table =
        Schema::new(db.get_database_backend()).create_table_from_entity(folders::Entity);
    table.table(Alias::new("folder_values"));
    db.execute(&table).await?;
    let id = Uuid::new_v4();
    db.execute(
        &Query::insert()
            .into_table(Alias::new("folder_values"))
            .columns(["id", "uuid", "label", "local"].map(Alias::new))
            .values_panic([
                1.into(),
                id.to_string().into(),
                "metadata".into(),
                "".into(),
            ])
            .to_owned(),
    )
    .await?;
    // SQLite evaluates this expression only if the local column is selected.
    // The view makes an accidental full-row read fail without allocating a blob.
    db.execute_unprepared("CREATE VIEW folders AS SELECT id, uuid, label, json_extract('invalid', '$') AS local FROM folder_values").await?;
    assert!(folders::Entity::find().one(&db).await.is_err());
    let key = ObjectKey::<Catalog> {
        collection: Collection::Folder,
        id,
    };
    assert_eq!(
        Catalog::encode_on(&db, &key).await?,
        Some(Payload::Folder(Folder {
            label: "metadata".into()
        }))
    );
    let tx = db.begin().await?;
    let page = Catalog::scan_on(
        &tx,
        &Collection::Folder,
        None,
        sync_orm::row_policy::ScanLimits {
            max_records: 128,
            max_rows: 2,
            max_bytes: 256,
            max_mutations: 0,
        },
    )
    .await?;
    assert_eq!(page.rows_read, 1);
    assert_eq!(page.records.len(), 1);
    tx.commit().await?;
    Ok(())
}

sync_orm::owned_list_mapping! { pub struct OrderedClause, OrderedClauseMap for clauses::Entity {
    max_items: 3, owners: [clauses::Column::Owner], constants: { clauses::Column::Kind => "composed" },
    key: stable: Uuid => clauses::Column::Stable,
    fields: { position: i32 => clauses::Column::Position, label: String => clauses::Column::Label },
    defaults: { clauses::Column::LocalPath => "" },
    order: position => clauses::Column::Position,
    limits: { position => (0, 2), label => (0, 128) },
} }
fn derive_folder_search(
    _: &sync_orm::row_policy::RowView<folders::Entity>,
    value: Option<&ComposedFolder>,
) -> Result<Vec<SearchDoc>, sea_orm::DbErr> {
    if value.is_some_and(|value| value.label == "fail-effect") {
        return Err(sea_orm::DbErr::Custom(
            "injected derived-effect failure".into(),
        ));
    }
    Ok(value
        .into_iter()
        .map(|value| SearchDoc {
            text: format!(
                "{}:{}",
                value.label,
                value
                    .clauses
                    .iter()
                    .map(|item| item.label.as_str())
                    .collect::<Vec<_>>()
                    .join(",")
            ),
        })
        .collect())
}
sync_orm::row_policies! { pub Composed, ComposedCollection, ComposedPayload {
    model: "composed", schema: "1", collections: {
        Folder => { tag: "folders", entity: folders::Entity, value: ComposedFolder, mapping: ComposedFolderMap,
            policy: {
                uuid: folders::Column::Uuid,
                fields: { label: String => folders::Column::Label },
                defaults: { folders::Column::Local => "" },
                local_reads: [folders::Column::Local],
                lists: { clauses: OrderedClause => {
                    entity: clauses::Entity, mapping: OrderedClauseMap,
                    owner: |row: &sync_orm::row_policy::RowView<folders::Entity>| -> Result<_, sea_orm::DbErr> { Ok(vec![row.read::<String>(folders::Column::Uuid)?.into()]) }
                } },
                effects: { search => {
                    entity: search_docs::Entity, mapping: SearchMap,
                    owner: |row: &sync_orm::row_policy::RowView<folders::Entity>| -> Result<_, sea_orm::DbErr> { Ok(vec![row.read::<i32>(folders::Column::Id)?.to_string().into()]) },
                    derive: derive_folder_search
                } },
                local_updates: |before: Option<&sync_orm::row_policy::RowView<folders::Entity>>, value: &ComposedFolder| -> Result<_, sea_orm::DbErr> {
                    let local = match before {
                        Some(row) if row.read::<String>(folders::Column::Label)? == value.label => row.read::<String>(folders::Column::Local)?,
                        _ => String::new(),
                    };
                    Ok(vec![(folders::Column::Local, local.into())])
                },
                limits: { label => (1, 128) },
            }
        }
    }
} }

#[tokio::test]
async fn registered_lists_and_effects_share_encoding_projection_scan_and_rollback()
-> anyhow::Result<()> {
    use sea_orm::{
        ActiveModelTrait, ColumnTrait, IntoActiveModel, QueryFilter, QueryOrder, Set,
        sea_query::Index,
    };
    use std::sync::Arc;
    use sync::protocol::SyncStore;
    use sync_orm::{
        causal_capabilities::CausalEndpoint,
        causal_schema,
        causal_store::CausalReplica,
        registry::{ProjectionBudget, RegistryProjection},
    };
    let db = Database::connect("sqlite::memory:").await?;
    let schema = Schema::new(db.get_database_backend());
    db.execute(&schema.create_table_from_entity(folders::Entity))
        .await?;
    db.execute(&schema.create_table_from_entity(clauses::Entity))
        .await?;
    db.execute(&schema.create_table_from_entity(search_docs::Entity))
        .await?;
    db.execute(
        &Index::create()
            .name("clause_order")
            .table(clauses::Entity)
            .col(clauses::Column::Owner)
            .col(clauses::Column::Kind)
            .col(clauses::Column::Position)
            .unique()
            .to_owned(),
    )
    .await?;
    causal_schema::install_fresh(&db).await?;
    Composed::validate_database_on(&db).await?;
    let replica =
        CausalReplica::<Composed>::create_empty(db.clone(), Uuid::new_v4(), Uuid::new_v4()).await?;
    let endpoint = CausalEndpoint::with_projection(
        replica,
        Arc::new(RegistryProjection::<Composed>::new(ProjectionBudget {
            max_reads: 9,
            max_mutations: 14,
        })?),
    );
    let key = ObjectKey::<Composed> {
        collection: ComposedCollection::Folder,
        id: Uuid::new_v4(),
    };
    let first = Uuid::new_v4();
    let second = Uuid::new_v4();
    let initial = ComposedPayload::ComposedFolder(ComposedFolder {
        label: "Alpha".into(),
        clauses: vec![
            OrderedClause {
                stable: first,
                position: 0,
                label: "first".into(),
            },
            OrderedClause {
                stable: second,
                position: 1,
                label: "second".into(),
            },
        ],
    });
    endpoint
        .write_local(key.clone(), Some(initial.clone()))
        .await?;
    assert_eq!(Composed::encode_on(&db, &key).await?, Some(initial));
    let row = folders::Entity::find().one(&db).await?.unwrap();
    let folder_id = row.id;
    let mut local = row.into_active_model();
    local.local = Set("device-private".into());
    local.update(&db).await?;
    let row = clauses::Entity::find()
        .filter(clauses::Column::Stable.eq(first.to_string()))
        .one(&db)
        .await?
        .unwrap();
    let clause_id = row.id;
    let mut local = row.into_active_model();
    local.local_path = Set("/device/path".into());
    local.update(&db).await?;
    let swapped = ComposedPayload::ComposedFolder(ComposedFolder {
        label: "Alpha".into(),
        clauses: vec![
            OrderedClause {
                stable: second,
                position: 0,
                label: "second".into(),
            },
            OrderedClause {
                stable: first,
                position: 1,
                label: "first".into(),
            },
        ],
    });
    endpoint
        .write_local(key.clone(), Some(swapped.clone()))
        .await?;
    assert_eq!(Composed::encode_on(&db, &key).await?, Some(swapped.clone()));
    let row = clauses::Entity::find_by_id(clause_id)
        .one(&db)
        .await?
        .unwrap();
    assert_eq!((row.position, row.local_path.as_str()), (1, "/device/path"));
    assert_eq!(
        folders::Entity::find_by_id(folder_id)
            .one(&db)
            .await?
            .unwrap()
            .local,
        "device-private"
    );
    assert_eq!(
        search_docs::Entity::find().one(&db).await?.unwrap().text,
        "Alpha:second,first"
    );
    let tx = db.begin().await?;
    let page = Composed::scan_on(
        &tx,
        &ComposedCollection::Folder,
        None,
        sync_orm::row_policy::ScanLimits {
            max_records: 128,
            max_rows: 10,
            max_bytes: 4096,
            max_mutations: 2,
        },
    )
    .await?;
    assert_eq!(page.records[0].payload, swapped);
    assert_eq!(page.rows_read, 3);
    tx.commit().await?;
    let head = endpoint.head().await?;
    let failed = ComposedPayload::ComposedFolder(ComposedFolder {
        label: "fail-effect".into(),
        clauses: vec![],
    });
    assert!(
        endpoint
            .write_local(key.clone(), Some(failed))
            .await
            .is_err()
    );
    assert_eq!(endpoint.head().await?, head);
    assert_eq!(Composed::encode_on(&db, &key).await?, Some(swapped));
    assert_eq!(
        clauses::Entity::find()
            .order_by_asc(clauses::Column::Position)
            .all(&db)
            .await?
            .len(),
        2
    );
    endpoint.write_local(key.clone(), None).await?;
    assert!(Composed::encode_on(&db, &key).await?.is_none());
    assert!(clauses::Entity::find().all(&db).await?.is_empty());
    assert!(search_docs::Entity::find().all(&db).await?.is_empty());
    Ok(())
}

mod folders {
    use sea_orm::entity::prelude::*;
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "folders")]
    pub struct Model {
        #[sea_orm(primary_key)]
        pub id: i32,
        #[sea_orm(unique)]
        pub uuid: String,
        #[sea_orm(unique)]
        pub label: String,
        pub local: String,
    }
    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}
}
mod notes {
    use sea_orm::entity::prelude::*;
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "notes")]
    pub struct Model {
        #[sea_orm(primary_key)]
        pub id: i32,
        #[sea_orm(unique)]
        pub uuid: String,
        pub parent_uuid: String,
        pub title: String,
        pub local: String,
    }
    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}
}
mod ownership {
    use sea_orm::entity::prelude::*;
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "ownership")]
    pub struct Model {
        #[sea_orm(primary_key)]
        pub id: i32,
        #[sea_orm(unique)]
        pub uuid: String,
        pub owner: String,
    }
    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}
}
mod labels {
    use sea_orm::entity::prelude::*;
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "labels")]
    pub struct Model {
        #[sea_orm(primary_key)]
        pub id: i32,
        #[sea_orm(unique)]
        pub uuid: String,
        pub name: String,
    }
    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}
}
mod drafts {
    use sea_orm::entity::prelude::*;
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "drafts")]
    pub struct Model {
        #[sea_orm(primary_key)]
        pub id: i32,
        #[sea_orm(unique)]
        pub uuid: Option<String>,
        pub body: String,
    }
    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}
}
mod associations {
    use sea_orm::entity::prelude::*;
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "associations")]
    pub struct Model {
        #[sea_orm(primary_key)]
        pub id: i32,
        #[sea_orm(unique)]
        pub uuid: String,
        pub left: String,
        pub right: String,
    }
    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}
}
fn association_identity(item: &Association) -> Result<Uuid, sea_orm::DbErr> {
    let mut hasher = blake3::Hasher::new();
    hasher.update(item.left.as_bytes());
    hasher.update(item.right.as_bytes());
    let mut bytes = [0_u8; 16];
    bytes.copy_from_slice(&hasher.finalize().as_bytes()[..16]);
    Ok(Uuid::from_bytes(bytes))
}
mod metrics {
    use sea_orm::entity::prelude::*;
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "metrics")]
    pub struct Model {
        #[sea_orm(primary_key)]
        pub id: i32,
        #[sea_orm(unique)]
        pub uuid: String,
        pub count: i32,
    }
    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}
}
sync_orm::row_policies! { pub Catalog, Collection, Payload {
    model: "example", schema: "1", collections: {
        Metric => { tag: "metrics", entity: metrics::Entity, value: Metric, mapping: MetricMap,
            policy: { uuid: metrics::Column::Uuid, fields: { count: i32 => metrics::Column::Count }, defaults: {}, limits: { count => (0, 1000) }, } },
        Association => { tag: "associations", entity: associations::Entity, value: Association, mapping: AssociationMap,
            policy: { uuid: associations::Column::Uuid, fields: { left: Uuid => associations::Column::Left, right: Uuid => associations::Column::Right }, defaults: {}, references: { left => "folders", right => "folders" }, identity: association_identity, } },
        Draft => { tag: "drafts", entity: drafts::Entity, value: Draft, mapping: DraftMap,
            policy: { uuid: drafts::Column::Uuid, fields: { body: String => drafts::Column::Body }, defaults: {}, } },
        Label => { tag: "labels", entity: labels::Entity, value: Label, mapping: LabelMap,
            policy: { uuid: labels::Column::Uuid, fields: { name: String => labels::Column::Name }, defaults: {}, } },
        Owned => { tag: "owned", entity: ownership::Entity, value: Owned, mapping: OwnedMap,
            policy: { uuid: ownership::Column::Uuid, fields: { owner: Uuid => ownership::Column::Owner }, defaults: {}, owner: { field: owner, allow_removal: false }, } },
        Folder => { tag: "folders", entity: folders::Entity, value: Folder, mapping: FolderMap,
            policy: { uuid: folders::Column::Uuid, fields: { label: String => folders::Column::Label }, defaults: { folders::Column::Local => "local" }, limits: { label => (1, 20) }, } },
        Note => { tag: "notes", entity: notes::Entity, value: Note, mapping: NoteMap,
            policy: { uuid: notes::Column::Uuid, fields: { parent_uuid: Uuid => notes::Column::ParentUuid, title: String => notes::Column::Title }, defaults: { notes::Column::Local => "local" }, references: { parent_uuid => "folders" }, } }
    }
} }
#[tokio::test]
async fn source_projection_uses_shared_mapping_and_transaction() {
    Catalog::validate_registry().unwrap();
    let id = Uuid::new_v4();
    let key = ObjectKey::<Catalog> {
        collection: Collection::Owned,
        id,
    };
    let revision = sync::protocol::Revision {
        actor: Uuid::new_v4(),
        counter: 1,
    };
    assert!(
        Catalog::validate_revision(&key, &Some(Payload::Owned(Owned { owner: id })), &revision)
            .is_err()
    );
    assert!(Catalog::validate_revision(&key, &None, &revision).is_err());
    assert!(Catalog::validate(&key, &None).is_err());
    assert!(Catalog::validate(&key, &Some(Payload::Owned(Owned { owner: Uuid::nil() }))).is_err());
    assert!(
        Catalog::validate_revision(
            &key,
            &Some(Payload::Owned(Owned { owner: id })),
            &sync::protocol::Revision {
                actor: id,
                counter: 1
            }
        )
        .is_ok()
    );
    let db = Database::connect("sqlite::memory:").await.unwrap();
    let schema = Schema::new(db.get_database_backend());
    db.execute(&schema.create_table_from_entity(folders::Entity))
        .await
        .unwrap();
    db.execute(&schema.create_table_from_entity(notes::Entity))
        .await
        .unwrap();
    db.execute(&schema.create_table_from_entity(labels::Entity))
        .await
        .unwrap();
    let label = ObjectKey::<Catalog> {
        collection: Collection::Label,
        id: Uuid::new_v4(),
    };
    let label_value = Payload::Label(Label {
        name: "plain".into(),
    });
    Catalog::project(&db, &label, Some(&label_value))
        .await
        .unwrap();
    assert_eq!(
        Catalog::encode_on(&db, &label).await.unwrap(),
        Some(label_value)
    );
    let parent = ObjectKey::<Catalog> {
        collection: Collection::Folder,
        id: Uuid::new_v4(),
    };
    let child = ObjectKey::<Catalog> {
        collection: Collection::Note,
        id: Uuid::new_v4(),
    };
    assert!(
        Catalog::validate(
            &child,
            &Some(Payload::Note(Note {
                parent_uuid: Uuid::nil(),
                title: "bad".into()
            }))
        )
        .is_err()
    );
    let note = Payload::Note(Note {
        parent_uuid: parent.id,
        title: "child".into(),
    });
    Catalog::project(&db, &child, Some(&note)).await.unwrap();
    assert_eq!(
        Catalog::encode_on(&db, &child).await.unwrap(),
        Some(note.clone())
    );
    let tx = db.begin().await.unwrap();
    Catalog::project_on(
        &tx,
        &parent,
        Some(&Payload::Folder(Folder {
            label: "parent".into(),
        })),
    )
    .await
    .unwrap();
    Catalog::project_on(&tx, &child, None).await.unwrap();
    tx.rollback().await.unwrap();
    assert_eq!(Catalog::encode_on(&db, &child).await.unwrap(), Some(note));
    assert!(Catalog::encode_on(&db, &parent).await.unwrap().is_none());
    assert!(
        Catalog::validate(
            &child,
            &Some(Payload::Folder(Folder {
                label: "wrong".into()
            }))
        )
        .is_err()
    );
    notes::Entity::update_many()
        .col_expr(notes::Column::Local, Expr::value("kept"))
        .exec(&db)
        .await
        .unwrap();
    let updated = Payload::Note(Note {
        parent_uuid: parent.id,
        title: "updated".into(),
    });
    Catalog::project(&db, &child, Some(&updated)).await.unwrap();
    let child_before = notes::Entity::find().one(&db).await.unwrap().unwrap();
    assert_eq!(child_before.local, "kept");
    Catalog::project(
        &db,
        &parent,
        Some(&Payload::Folder(Folder {
            label: "parent".into(),
        })),
    )
    .await
    .unwrap();
    let original = folders::Entity::find().one(&db).await.unwrap().unwrap();
    Catalog::project(&db, &parent, None).await.unwrap();
    assert_eq!(
        notes::Entity::find().one(&db).await.unwrap().unwrap(),
        child_before
    );
    let other = ObjectKey::<Catalog> {
        collection: Collection::Folder,
        id: Uuid::new_v4(),
    };
    Catalog::project(
        &db,
        &other,
        Some(&Payload::Folder(Folder {
            label: "other".into(),
        })),
    )
    .await
    .unwrap();
    Catalog::project(
        &db,
        &parent,
        Some(&Payload::Folder(Folder {
            label: "restored".into(),
        })),
    )
    .await
    .unwrap();
    use sea_orm::{ColumnTrait, QueryFilter};
    let restored = folders::Entity::find()
        .filter(folders::Column::Uuid.eq(parent.id.to_string()))
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    assert_ne!(original.id, restored.id);
    assert_eq!(
        notes::Entity::find()
            .one(&db)
            .await
            .unwrap()
            .unwrap()
            .parent_uuid,
        restored.uuid
    );
    assert_eq!(
        notes::Entity::find().one(&db).await.unwrap().unwrap().local,
        "kept"
    );
}

#[tokio::test]
async fn causal_selected_survivor_and_source_rollback() -> anyhow::Result<()> {
    use sync::protocol::Observed;
    use sync_orm::{
        causal_capabilities::TransactionProjection,
        causal_store::CausalReplica,
        registry::{ProjectionBudget, RegistryProjection},
    };
    let db = fixtures::fresh_database().await?;
    let schema = Schema::new(db.get_database_backend());
    db.execute(&schema.create_table_from_entity(folders::Entity))
        .await?;
    let replica =
        CausalReplica::<Catalog>::create_empty(db.clone(), Uuid::new_v4(), Uuid::new_v4()).await?;
    let key = ObjectKey::<Catalog> {
        collection: Collection::Folder,
        id: Uuid::new_v4(),
    };
    let projection = RegistryProjection::<Catalog>::new(ProjectionBudget {
        max_reads: 2,
        max_mutations: 1,
    })?;
    let low = Payload::Folder(Folder {
        label: "low".into(),
    });
    let high = Payload::Folder(Folder {
        label: "high".into(),
    });
    let tx = db.begin().await?;
    let first = replica
        .write_observed_on(&tx, key.clone(), Some(low.clone()), Observed::AllCurrent)
        .await?;
    projection
        .apply(
            &tx,
            &sync::protocol::ProjectionBatch {
                changes: vec![first.projection],
                causal_changes: 1,
                pending: 0,
            },
        )
        .await?;
    tx.commit().await?;
    let tx = db.begin().await?;
    let second = replica
        .write_observed_on(
            &tx,
            key.clone(),
            Some(high.clone()),
            Observed::Exact(Default::default()),
        )
        .await?;
    let high_dot = second.projection.after.as_ref().unwrap().dot;
    let denied = RegistryProjection::<Catalog>::new(ProjectionBudget {
        max_reads: 0,
        max_mutations: 0,
    })?;
    let batch = sync::protocol::ProjectionBatch {
        changes: vec![second.projection.clone()],
        causal_changes: 1,
        pending: 0,
    };
    assert!(denied.apply(&tx, &batch).await.is_err());
    assert_eq!(Catalog::encode_on(&tx, &key).await?, Some(low.clone()));
    projection
        .apply(
            &tx,
            &sync::protocol::ProjectionBatch {
                changes: vec![second.projection],
                causal_changes: 1,
                pending: 0,
            },
        )
        .await?;
    tx.commit().await?;
    let tx = db.begin().await?;
    let reveal = replica
        .write_observed_on(
            &tx,
            key.clone(),
            None,
            Observed::Exact([high_dot].into_iter().collect()),
        )
        .await?;
    Catalog::project_on(
        &tx,
        &key,
        reveal.projection.after.as_ref().map(|c| &c.payload),
    )
    .await?;
    assert_eq!(Catalog::encode_on(&tx, &key).await?, Some(low.clone()));
    tx.rollback().await?;
    assert_eq!(Catalog::encode_on(&db, &key).await?, Some(high));
    let tx = db.begin().await?;
    let reveal = replica
        .write_observed_on(
            &tx,
            key.clone(),
            None,
            Observed::Exact([high_dot].into_iter().collect()),
        )
        .await?;
    Catalog::project_on(
        &tx,
        &key,
        reveal.projection.after.as_ref().map(|c| &c.payload),
    )
    .await?;
    tx.commit().await?;
    assert_eq!(Catalog::encode_on(&db, &key).await?, Some(low));
    Ok(())
}

#[tokio::test]
async fn endpoint_receives_generated_projection_atomically() -> anyhow::Result<()> {
    use std::sync::Arc;
    use sync::protocol::{self as p, ReceiveStore, SyncStore};
    use sync_orm::{
        causal_capabilities::CausalEndpoint,
        causal_progress,
        causal_store::CausalReplica,
        registry::{ProjectionBudget, RegistryProjection},
    };
    let db = fixtures::fresh_database().await?;
    let schema = Schema::new(db.get_database_backend());
    db.execute(&schema.create_table_from_entity(folders::Entity))
        .await?;
    let namespace = Uuid::new_v4();
    let replica =
        CausalReplica::<Catalog>::create_empty(db.clone(), namespace, Uuid::new_v4()).await?;
    let denied = CausalEndpoint::with_projection(
        replica.clone(),
        Arc::new(RegistryProjection::<Catalog>::new(ProjectionBudget {
            max_reads: 0,
            max_mutations: 0,
        })?),
    );
    let endpoint = CausalEndpoint::with_projection(
        replica,
        Arc::new(RegistryProjection::<Catalog>::new(ProjectionBudget {
            max_reads: 8,
            max_mutations: 8,
        })?),
    );
    let source = p::MemorySyncStore::<Catalog>::new(
        p::StoreIdentity {
            namespace,
            model: p::ModelId {
                model: "example".into(),
                schema: "1".into(),
            },
            replica: Uuid::new_v4(),
            generation: Uuid::new_v4(),
        },
        p::LocalAuthor {
            author: Uuid::new_v4(),
            writer_incarnation: Uuid::new_v4(),
        },
    )?;
    let scope = p::Scope {
        collections: [Collection::Folder].into_iter().collect(),
    };
    let identity = source.identity().await?;
    let checkpoint = p::CheckpointKey {
        namespace,
        source_replica: identity.replica,
        source_generation: identity.generation,
        scope_id: p::scope_id(&scope)?,
    };
    let genesis = source.head().await?;
    let local_identity = endpoint.identity().await?;
    let tx = db.begin().await?;
    causal_progress::initialize_checkpoint_on(&tx, &local_identity, &checkpoint, &scope, &genesis)
        .await?;
    tx.commit().await?;
    let key = ObjectKey::<Catalog> {
        collection: Collection::Folder,
        id: Uuid::new_v4(),
    };
    let value = Payload::Folder(Folder {
        label: "received".into(),
    });
    source.write_local(key.clone(), Some(value.clone())).await?;
    let p::PageRead::Page(page) = source
        .read_page(p::PageRequest {
            source: identity,
            scope,
            after: genesis.clone(),
            through: source.head().await?,
            limits: p::PageLimits {
                max_scanned: 8,
                max_bytes: 65536,
            },
        })
        .await?
    else {
        anyhow::bail!("unexpected fragmented page")
    };
    let limits = p::ApplyLimits {
        max_projection_items: 16,
        max_projection_bytes: 65536,
        max_mutations: 1024,
    };
    let head = endpoint.head().await?;
    assert!(denied.apply_page(&page, limits).await.is_err());
    assert_eq!(endpoint.head().await?, head);
    assert_eq!(endpoint.checkpoint(&checkpoint).await?, Some(genesis));
    assert!(Catalog::encode_on(&db, &key).await?.is_none());
    endpoint.apply_page(&page, limits).await?;
    assert_eq!(Catalog::encode_on(&db, &key).await?, Some(value));
    assert_eq!(
        endpoint.checkpoint(&checkpoint).await?,
        Some(page.next.clone())
    );
    let head = endpoint.head().await?;
    endpoint.apply_page(&page, limits).await?;
    assert_eq!(endpoint.head().await?, head);
    Ok(())
}

#[tokio::test]
async fn endpoint_source_constraint_failure_rolls_back_batch() -> anyhow::Result<()> {
    use std::sync::Arc;
    use sync::protocol::{self as p, ReceiveStore, SyncStore};
    use sync_orm::{
        causal_capabilities::CausalEndpoint,
        causal_progress,
        causal_store::CausalReplica,
        registry::{ProjectionBudget, RegistryProjection},
    };
    let db = fixtures::fresh_database().await?;
    db.execute(&Schema::new(db.get_database_backend()).create_table_from_entity(folders::Entity))
        .await?;
    let namespace = Uuid::new_v4();
    let replica =
        CausalReplica::<Catalog>::create_empty(db.clone(), namespace, Uuid::new_v4()).await?;
    let endpoint = CausalEndpoint::with_projection(
        replica,
        Arc::new(RegistryProjection::<Catalog>::new(ProjectionBudget {
            max_reads: 8,
            max_mutations: 8,
        })?),
    );
    let source = p::MemorySyncStore::<Catalog>::new(
        p::StoreIdentity {
            namespace,
            model: p::ModelId {
                model: "example".into(),
                schema: "1".into(),
            },
            replica: Uuid::new_v4(),
            generation: Uuid::new_v4(),
        },
        p::LocalAuthor {
            author: Uuid::new_v4(),
            writer_incarnation: Uuid::new_v4(),
        },
    )?;
    let scope = p::Scope {
        collections: [Collection::Folder].into_iter().collect(),
    };
    let identity = source.identity().await?;
    let checkpoint = p::CheckpointKey {
        namespace,
        source_replica: identity.replica,
        source_generation: identity.generation,
        scope_id: p::scope_id(&scope)?,
    };
    let genesis = source.head().await?;
    let local_identity = endpoint.identity().await?;
    let tx = db.begin().await?;
    causal_progress::initialize_checkpoint_on(&tx, &local_identity, &checkpoint, &scope, &genesis)
        .await?;
    tx.commit().await?;
    let first = ObjectKey::<Catalog> {
        collection: Collection::Folder,
        id: Uuid::new_v4(),
    };
    let second = ObjectKey::<Catalog> {
        collection: Collection::Folder,
        id: Uuid::new_v4(),
    };
    let value = Payload::Folder(Folder {
        label: "duplicate".into(),
    });
    source
        .write_local(first.clone(), Some(value.clone()))
        .await?;
    source.write_local(second.clone(), Some(value)).await?;
    let p::PageRead::Page(page) = source
        .read_page(p::PageRequest {
            source: identity,
            scope,
            after: genesis.clone(),
            through: source.head().await?,
            limits: p::PageLimits {
                max_scanned: 8,
                max_bytes: 65536,
            },
        })
        .await?
    else {
        anyhow::bail!("unexpected fragmented page")
    };
    let completion_query = sea_orm_sea_query::Query::select()
        .column(sea_orm_sea_query::Alias::new("completed_at"))
        .from(sea_orm_sea_query::Alias::new("sync_checkpoints"))
        .to_owned();
    let completion_before: String = db
        .query_one(&completion_query)
        .await?
        .unwrap()
        .try_get_by_index(0)?;
    let head = endpoint.head().await?;
    assert!(
        endpoint
            .apply_page(
                &page,
                p::ApplyLimits {
                    max_projection_items: 16,
                    max_projection_bytes: 65536,
                    max_mutations: 1024
                }
            )
            .await
            .is_err()
    );
    assert_eq!(endpoint.head().await?, head);
    assert_eq!(endpoint.checkpoint(&checkpoint).await?, Some(genesis));
    let completion_after: String = db
        .query_one(&completion_query)
        .await?
        .unwrap()
        .try_get_by_index(0)?;
    assert_eq!(completion_before, completion_after);
    assert!(Catalog::encode_on(&db, &first).await?.is_none());
    assert!(Catalog::encode_on(&db, &second).await?.is_none());
    assert!(endpoint.get(&first).await?.is_none());
    assert!(endpoint.get(&second).await?.is_none());
    Ok(())
}

#[tokio::test]
async fn parent_removal_does_not_mutate_large_fanout() -> anyhow::Result<()> {
    use sea_orm::{
        PaginatorTrait,
        sea_query::{Alias, Func, Query},
    };
    let db = Database::connect("sqlite::memory:").await?;
    let schema = Schema::new(db.get_database_backend());
    db.execute(&schema.create_table_from_entity(folders::Entity))
        .await?;
    db.execute(&schema.create_table_from_entity(notes::Entity))
        .await?;
    let parent = ObjectKey::<Catalog> {
        collection: Collection::Folder,
        id: Uuid::new_v4(),
    };
    let tx = db.begin().await?;
    Catalog::project_on(
        &tx,
        &parent,
        Some(&Payload::Folder(Folder {
            label: "parent".into(),
        })),
    )
    .await?;
    for _ in 0..3000 {
        let child = ObjectKey::<Catalog> {
            collection: Collection::Note,
            id: Uuid::new_v4(),
        };
        Catalog::project_on(
            &tx,
            &child,
            Some(&Payload::Note(Note {
                parent_uuid: parent.id,
                title: "child".into(),
            })),
        )
        .await?;
    }
    tx.commit().await?;
    let query = Query::select()
        .expr_as(Func::cust(Alias::new("total_changes")), Alias::new("n"))
        .to_owned();
    let before: i64 = db.query_one(&query).await?.unwrap().try_get("", "n")?;
    Catalog::project(&db, &parent, None).await?;
    let after: i64 = db.query_one(&query).await?.unwrap().try_get("", "n")?;
    assert_eq!(after - before, 1);
    assert_eq!(notes::Entity::find().count(&db).await?, 3000);
    Ok(())
}

#[tokio::test]
async fn scan_assigns_uuid_in_bounded_resumable_transaction() -> anyhow::Result<()> {
    use sea_orm::{ActiveModelTrait, Set};
    use sync_orm::row_policy::ScanLimits;
    let db = Database::connect("sqlite::memory:").await?;
    db.execute(&Schema::new(db.get_database_backend()).create_table_from_entity(drafts::Entity))
        .await?;
    for body in ["first", "second", "third"] {
        drafts::ActiveModel {
            uuid: Set(None),
            body: Set(body.into()),
            ..Default::default()
        }
        .insert(&db)
        .await?;
    }
    let limits = ScanLimits {
        max_records: 128,
        max_rows: 2,
        max_bytes: 1024,
        max_mutations: 2,
    };
    let tx = db.begin().await?;
    let page = Catalog::scan_on(&tx, &Collection::Draft, None, limits).await?;
    assert_eq!(page.records.len(), 2);
    assert_eq!(page.mutations, 2);
    assert_eq!(page.rows_read, 2);
    assert!(!page.done);
    tx.rollback().await?;
    assert!(
        drafts::Entity::find()
            .all(&db)
            .await?
            .iter()
            .all(|r| r.uuid.is_none())
    );
    let tx = db.begin().await?;
    let page = Catalog::scan_on(&tx, &Collection::Draft, None, limits).await?;
    let after = page.next;
    let ids: Vec<_> = page.records.iter().map(|r| r.id).collect();
    tx.commit().await?;
    let tx = db.begin().await?;
    let last = Catalog::scan_on(&tx, &Collection::Draft, after, limits).await?;
    assert!(last.done);
    assert_eq!(last.records.len(), 1);
    assert!(!ids.contains(&last.records[0].id));
    tx.commit().await?;
    let tx = db.begin().await?;
    let resumed = Catalog::scan_on(&tx, &Collection::Draft, None, limits).await?;
    assert_eq!(resumed.mutations, 0);
    assert_eq!(
        resumed.records.iter().map(|r| r.id).collect::<Vec<_>>(),
        ids
    );
    tx.rollback().await?;
    Ok(())
}

#[tokio::test]
async fn validates_actual_source_identity_constraints() -> anyhow::Result<()> {
    use sync_orm::row_policy::RowPolicy;
    let db = Database::connect("sqlite::memory:").await?;
    db.execute(&Schema::new(db.get_database_backend()).create_table_from_entity(drafts::Entity))
        .await?;
    RowPolicy::<drafts::Entity, DraftMap>::validate_database_on(&db).await?;
    db.execute_unprepared(
        "CREATE TABLE labels (id INTEGER PRIMARY KEY, uuid TEXT, name TEXT NOT NULL)",
    )
    .await?;
    assert!(
        RowPolicy::<labels::Entity, LabelMap>::validate_database_on(&db)
            .await
            .is_err()
    );
    db.execute_unprepared("CREATE UNIQUE INDEX labels_uuid ON labels(uuid)")
        .await?;
    RowPolicy::<labels::Entity, LabelMap>::validate_database_on(&db).await?;
    Ok(())
}

mod clauses {
    use sea_orm::entity::prelude::*;
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "clauses")]
    pub struct Model {
        #[sea_orm(primary_key)]
        pub id: i32,
        pub owner: String,
        pub kind: String,
        pub stable: String,
        pub position: i32,
        pub label: String,
        pub local_path: String,
    }
    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}
}
sync_orm::owned_list_mapping! { pub struct Clause, ClauseMap for clauses::Entity {
    max_items: 3, owners: [clauses::Column::Owner], constants: { clauses::Column::Kind => "mix" },
    key: stable: String => clauses::Column::Stable,
    fields: { position: i32 => clauses::Column::Position, label: String => clauses::Column::Label },
    defaults: { clauses::Column::LocalPath => "" },
} }
#[tokio::test]
async fn owned_list_diff_preserves_local_identity_and_fields() -> anyhow::Result<()> {
    use sea_orm::{ActiveModelTrait, IntoActiveModel, Set};
    use sync_orm::owned_list::{ListBudget, OwnedListPolicy};
    type Clauses = OwnedListPolicy<clauses::Entity, ClauseMap>;
    let db = Database::connect("sqlite::memory:").await?;
    db.execute(&Schema::new(db.get_database_backend()).create_table_from_entity(clauses::Entity))
        .await?;
    let owner = ["mix-a".into()];
    let other = ["mix-b".into()];
    let budget = ListBudget {
        max_reads: 4,
        max_mutations: 3,
    };
    let tx = db.begin().await?;
    Clauses::reconcile_on(
        &tx,
        &owner,
        &[
            Clause {
                stable: "one".into(),
                position: 1,
                label: "old".into(),
            },
            Clause {
                stable: "two".into(),
                position: 2,
                label: "remove".into(),
            },
        ],
        budget,
    )
    .await?;
    Clauses::reconcile_on(
        &tx,
        &other,
        &[Clause {
            stable: "one".into(),
            position: 1,
            label: "other".into(),
        }],
        budget,
    )
    .await?;
    tx.commit().await?;
    use sea_orm::{ColumnTrait, QueryFilter};
    let row = clauses::Entity::find()
        .filter(clauses::Column::Owner.eq("mix-a"))
        .filter(clauses::Column::Stable.eq("one"))
        .one(&db)
        .await?
        .unwrap();
    let old_id = row.id;
    let mut active = row.into_active_model();
    active.local_path = Set("device-only".into());
    active.update(&db).await?;
    let tx = db.begin().await?;
    let desired = [
        Clause {
            stable: "one".into(),
            position: 4,
            label: "new".into(),
        },
        Clause {
            stable: "three".into(),
            position: 5,
            label: "added".into(),
        },
    ];
    assert!(
        Clauses::reconcile_on(
            &tx,
            &owner,
            &desired,
            ListBudget {
                max_reads: 4,
                max_mutations: 2
            }
        )
        .await
        .is_err()
    );
    let work = Clauses::reconcile_on(&tx, &owner, &desired, budget).await?;
    assert_eq!(work.mutations, 3);
    tx.commit().await?;
    let row = clauses::Entity::find()
        .filter(clauses::Column::Owner.eq("mix-a"))
        .filter(clauses::Column::Stable.eq("one"))
        .one(&db)
        .await?
        .unwrap();
    assert_eq!(row.id, old_id);
    assert_eq!(row.local_path, "device-only");
    assert_eq!(row.label, "new");
    assert_eq!(Clauses::read_on(&db, &other, 4).await?[0].label, "other");
    let tx = db.begin().await?;
    assert!(
        Clauses::reconcile_on(
            &tx,
            &owner,
            &[desired[0].clone(), desired[0].clone()],
            budget
        )
        .await
        .is_err()
    );
    tx.rollback().await?;
    for stable in ["four", "five"] {
        clauses::ActiveModel {
            owner: Set("mix-a".into()),
            kind: Set("mix".into()),
            stable: Set(stable.into()),
            position: Set(6),
            label: Set("extra".into()),
            local_path: Set(String::new()),
            ..Default::default()
        }
        .insert(&db)
        .await?;
    }
    assert!(Clauses::read_on(&db, &owner, 4).await.is_err());
    let tx = db.begin().await?;
    assert!(
        Clauses::prepare_on(&tx, &owner, &desired, budget)
            .await
            .is_err()
    );
    tx.rollback().await?;
    Ok(())
}

#[tokio::test]
async fn scan_derives_noninvertible_identity_for_empty_source_uuid() -> anyhow::Result<()> {
    use sea_orm::{ActiveModelTrait, Set};
    use sync_orm::row_policy::ScanLimits;
    let db = Database::connect("sqlite::memory:").await?;
    db.execute(
        &Schema::new(db.get_database_backend()).create_table_from_entity(associations::Entity),
    )
    .await?;
    let left = Uuid::new_v4();
    let right = Uuid::new_v4();
    associations::ActiveModel {
        uuid: Set(String::new()),
        left: Set(left.to_string()),
        right: Set(right.to_string()),
        ..Default::default()
    }
    .insert(&db)
    .await?;
    let tx = db.begin().await?;
    let page = Catalog::scan_on(
        &tx,
        &Collection::Association,
        None,
        ScanLimits {
            max_records: 128,
            max_rows: 2,
            max_bytes: 4096,
            max_mutations: 1,
        },
    )
    .await?;
    let expected = association_identity(&Association { left, right })?;
    assert_eq!(page.records[0].id, expected);
    assert_eq!(page.mutations, 1);
    tx.commit().await?;
    let key = ObjectKey::<Catalog> {
        collection: Collection::Association,
        id: Uuid::new_v4(),
    };
    assert!(
        Catalog::validate(
            &key,
            &Some(Payload::Association(Association { left, right }))
        )
        .is_err()
    );
    Ok(())
}

#[test]
fn payload_rejects_wrong_scalar_null_overflow_and_extra_fields() {
    for raw in [
        r#"{"collection":"Metric","value":{"count":"1"}}"#,
        r#"{"collection":"Metric","value":{"count":null}}"#,
        r#"{"collection":"Metric","value":{"count":2147483648}}"#,
        r#"{"collection":"Metric","value":{"count":1,"extra":true}}"#,
    ] {
        assert!(serde_json::from_str::<Payload>(raw).is_err());
    }
    let key = ObjectKey::<Catalog> {
        collection: Collection::Metric,
        id: Uuid::new_v4(),
    };
    assert!(Catalog::validate(&key, &Some(Payload::Metric(Metric { count: 1001 }))).is_err());
    assert!(Catalog::validate(&key, &Some(Payload::Metric(Metric { count: -1 }))).is_err());
}

mod search_docs {
    use sea_orm::entity::prelude::*;
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "search_docs")]
    pub struct Model {
        #[sea_orm(primary_key)]
        pub id: i32,
        pub owner: String,
        pub kind: String,
        pub text: String,
    }
    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}
}
sync_orm::derived_effect_mapping! { pub struct SearchDoc, SearchMap for search_docs::Entity {
    max_items: 2, owners: [search_docs::Column::Owner], constants: { search_docs::Column::Kind => "folder" },
    fields: { text: String => search_docs::Column::Text }, defaults: {},
} }
#[tokio::test]
async fn derived_effect_is_bounded_and_scoped_to_owner() -> anyhow::Result<()> {
    use sync_orm::{derived_effect::DerivedEffect, owned_list::ListBudget};
    type Search = DerivedEffect<search_docs::Entity, SearchMap>;
    let db = Database::connect("sqlite::memory:").await?;
    db.execute(
        &Schema::new(db.get_database_backend()).create_table_from_entity(search_docs::Entity),
    )
    .await?;
    let derive =
        |_: &Option<String>, after: &Option<String>| -> Result<Vec<SearchDoc>, sea_orm::DbErr> {
            Ok(after
                .iter()
                .flat_map(|text| {
                    [
                        SearchDoc { text: text.clone() },
                        SearchDoc {
                            text: text.to_ascii_lowercase(),
                        },
                    ]
                })
                .collect())
        };
    let budget = ListBudget {
        max_reads: 3,
        max_mutations: 4,
    };
    let tx = db.begin().await?;
    Search::prepare_on(
        &tx,
        &["a".into()],
        &None,
        &Some("ALPHA".into()),
        derive,
        budget,
    )
    .await?
    .apply_on(&tx)
    .await?;
    Search::prepare_on(
        &tx,
        &["b".into()],
        &None,
        &Some("BETA".into()),
        derive,
        budget,
    )
    .await?
    .apply_on(&tx)
    .await?;
    tx.commit().await?;
    let tx = db.begin().await?;
    assert!(
        Search::prepare_on(
            &tx,
            &["a".into()],
            &Some("ALPHA".into()),
            &Some("NEW".into()),
            derive,
            ListBudget {
                max_reads: 3,
                max_mutations: 3
            }
        )
        .await
        .is_err()
    );
    let plan = Search::prepare_on(
        &tx,
        &["a".into()],
        &Some("ALPHA".into()),
        &Some("NEW".into()),
        derive,
        budget,
    )
    .await?;
    assert_eq!(plan.work.mutations, 4);
    plan.apply_on(&tx).await?;
    tx.commit().await?;
    use sea_orm::{ColumnTrait, QueryFilter};
    let other = search_docs::Entity::find()
        .filter(search_docs::Column::Owner.eq("b"))
        .all(&db)
        .await?;
    assert_eq!(other.len(), 2);
    assert!(other.iter().any(|row| row.text == "BETA"));
    Ok(())
}

mod search_fts {
    use sea_orm::entity::prelude::*;
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "search_fts")]
    pub struct Model {
        #[sea_orm(primary_key)]
        pub id: String,
        pub owner: Option<String>,
        pub kind: Option<String>,
        pub doc: Option<String>,
    }
    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}
}
sync_orm::derived_effect_mapping! { pub struct FtsDoc, FtsMap for search_fts::Entity {
    max_items: 2, owners: [search_fts::Column::Owner], constants: { search_fts::Column::Kind => Some("folder".to_string()) },
    fields: { doc: Option<String> => search_fts::Column::Doc }, defaults: { search_fts::Column::Id => "" },
} }
#[tokio::test]
async fn derived_effect_replaces_fts_rows_with_duplicate_ids() -> anyhow::Result<()> {
    use sync_orm::{derived_effect::DerivedEffect, owned_list::ListBudget};
    type Search = DerivedEffect<search_fts::Entity, FtsMap>;
    let db = Database::connect("sqlite::memory:").await?;
    db.execute_unprepared("CREATE VIRTUAL TABLE search_fts USING fts5(id, owner, kind, doc)")
        .await?;
    let derive =
        |_: &Option<String>, after: &Option<String>| -> Result<Vec<FtsDoc>, sea_orm::DbErr> {
            Ok(after
                .iter()
                .flat_map(|text| {
                    [
                        FtsDoc {
                            doc: Some(text.clone()),
                        },
                        FtsDoc {
                            doc: Some(text.to_ascii_lowercase()),
                        },
                    ]
                })
                .collect())
        };
    let budget = ListBudget {
        max_reads: 3,
        max_mutations: 4,
    };
    let tx = db.begin().await?;
    Search::prepare_on(
        &tx,
        &[Some("7".to_string()).into()],
        &None,
        &Some("First".into()),
        derive,
        budget,
    )
    .await?
    .apply_on(&tx)
    .await?;
    tx.commit().await?;
    let tx = db.begin().await?;
    let plan = Search::prepare_on(
        &tx,
        &[Some("7".to_string()).into()],
        &Some("First".into()),
        &Some("Second".into()),
        derive,
        budget,
    )
    .await?;
    assert_eq!(plan.work.rows_read, 2);
    plan.apply_on(&tx).await?;
    tx.commit().await?;
    let docs = search_fts::Entity::find().all(&db).await?;
    assert_eq!(docs.len(), 2);
    assert!(docs.iter().any(|row| row.doc.as_deref() == Some("Second")));
    Ok(())
}

mod alternate_folders {
    use sea_orm::entity::prelude::*;
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "folders")]
    pub struct Model {
        #[sea_orm(primary_key)]
        pub id: i32,
        #[sea_orm(unique)]
        pub uuid: String,
        pub label: String,
        pub local: String,
    }
    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}
}
sync_orm::row_policies! { pub Conflicting, ConflictingCollection, ConflictingPayload {
    model: "conflicting", schema: "1", collections: {
        One => { tag: "one", entity: folders::Entity, value: ConflictOne, mapping: ConflictOneMap,
            policy: { uuid: folders::Column::Uuid, fields: { label: String => folders::Column::Label }, defaults: { folders::Column::Local => "" }, } },
        Two => { tag: "two", entity: alternate_folders::Entity, value: ConflictTwo, mapping: ConflictTwoMap,
            policy: { uuid: alternate_folders::Column::Uuid, fields: { local: String => alternate_folders::Column::Local }, defaults: { alternate_folders::Column::Label => "" }, } }
    }
} }
#[test]
fn duplicate_primary_table_owners_are_rejected_at_construction() {
    use sync_orm::registry::{ProjectionBudget, RegistryProjection};
    let error = Conflicting::validate_registry().unwrap_err().to_string();
    assert!(error.contains("duplicate primary source table"));
    assert!(error.contains("one") && error.contains("two"));
    assert!(
        RegistryProjection::<Conflicting>::new(ProjectionBudget {
            max_reads: 100,
            max_mutations: 100
        })
        .is_err()
    );
    assert!(
        RegistryProjection::<Catalog>::new(ProjectionBudget {
            max_reads: 100,
            max_mutations: 100
        })
        .is_ok()
    );
}

sync_orm::derived_effect_mapping! { pub struct NoteSearchDoc, NoteSearchMap for search_docs::Entity {
    max_items: 2, owners: [search_docs::Column::Owner], constants: { search_docs::Column::Kind => "note" },
    fields: { text: String => search_docs::Column::Text }, defaults: {},
} }
fn derive_note_search(
    _: &sync_orm::row_policy::RowView<notes::Entity>,
    value: Option<&SharedNote>,
) -> Result<Vec<NoteSearchDoc>, sea_orm::DbErr> {
    Ok(value
        .into_iter()
        .map(|value| NoteSearchDoc {
            text: value.title.clone(),
        })
        .collect())
}
sync_orm::row_mapping! { pub struct SharedNote, SharedNoteMap for notes::Entity {
    uuid: notes::Column::Uuid,
    fields: { parent_uuid: Uuid => notes::Column::ParentUuid, title: String => notes::Column::Title },
    defaults: { notes::Column::Local => "" },
    effects: { search => {
        entity: search_docs::Entity, mapping: NoteSearchMap,
        owner: |row: &sync_orm::row_policy::RowView<notes::Entity>| -> Result<_, sea_orm::DbErr> { Ok(vec![row.read::<i32>(notes::Column::Id)?.to_string().into()]) },
        derive: derive_note_search
    } },
} }
sync_orm::row_registry! { pub SharedEffects, SharedCollection, SharedPayload {
    model: "shared-effects", schema: "1", collections: {
        Folder => { tag: "folders", entity: folders::Entity, mapping: ComposedFolderMap, value: ComposedFolder },
        Note => { tag: "notes", entity: notes::Entity, mapping: SharedNoteMap, value: SharedNote }
    }
} }
#[tokio::test]
async fn distinct_primary_owners_share_scoped_derived_table() -> anyhow::Result<()> {
    use sea_orm::{ColumnTrait, QueryFilter};
    use sync_orm::registry::{ProjectionBudget, RegistryProjection};
    let _projection = RegistryProjection::<SharedEffects>::new(ProjectionBudget {
        max_reads: 100,
        max_mutations: 100,
    })?;
    let db = Database::connect("sqlite::memory:").await?;
    let schema = Schema::new(db.get_database_backend());
    db.execute(&schema.create_table_from_entity(folders::Entity))
        .await?;
    db.execute(&schema.create_table_from_entity(notes::Entity))
        .await?;
    db.execute(&schema.create_table_from_entity(clauses::Entity))
        .await?;
    db.execute(&schema.create_table_from_entity(search_docs::Entity))
        .await?;
    SharedEffects::validate_database_on(&db).await?;
    let folder = ObjectKey::<SharedEffects> {
        collection: SharedCollection::Folder,
        id: Uuid::new_v4(),
    };
    let note = ObjectKey::<SharedEffects> {
        collection: SharedCollection::Note,
        id: Uuid::new_v4(),
    };
    SharedEffects::project(
        &db,
        &folder,
        Some(&SharedPayload::ComposedFolder(ComposedFolder {
            label: "folder".into(),
            clauses: vec![],
        })),
    )
    .await?;
    SharedEffects::project(
        &db,
        &note,
        Some(&SharedPayload::SharedNote(SharedNote {
            parent_uuid: folder.id,
            title: "note".into(),
        })),
    )
    .await?;
    SharedEffects::project(&db, &folder, None).await?;
    let remaining = search_docs::Entity::find()
        .filter(search_docs::Column::Kind.eq("note"))
        .all(&db)
        .await?;
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].text, "note");
    assert_eq!(search_docs::Entity::find().all(&db).await?.len(), 1);
    Ok(())
}

#[tokio::test]
async fn scan_canonicalizes_uppercase_uuid() -> anyhow::Result<()> {
    use sea_orm::{ActiveModelTrait, Set};
    let db = Database::connect("sqlite::memory:").await?;
    db.execute(&Schema::new(db.get_database_backend()).create_table_from_entity(folders::Entity))
        .await?;
    let id = Uuid::new_v4();
    folders::ActiveModel {
        uuid: Set(id.to_string().to_uppercase()),
        label: Set("old".into()),
        local: Set(String::new()),
        ..Default::default()
    }
    .insert(&db)
    .await?;
    let tx = db.begin().await?;
    Catalog::scan_on(
        &tx,
        &Collection::Folder,
        None,
        sync_orm::row_policy::ScanLimits {
            max_records: 128,
            max_rows: 2,
            max_bytes: 4096,
            max_mutations: 1,
        },
    )
    .await?;
    tx.commit().await?;
    let key = ObjectKey::<Catalog> {
        collection: Collection::Folder,
        id,
    };
    Catalog::project(
        &db,
        &key,
        Some(&Payload::Folder(Folder {
            label: "new".into(),
        })),
    )
    .await?;
    let rows = folders::Entity::find().all(&db).await?;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].uuid, id.to_string());
    Ok(())
}

#[tokio::test]
async fn scan_canonicalizes_uuid_references() -> anyhow::Result<()> {
    use sea_orm::{ActiveModelTrait, Set};
    let db = Database::connect("sqlite::memory:").await?;
    db.execute(
        &Schema::new(db.get_database_backend()).create_table_from_entity(associations::Entity),
    )
    .await?;
    let left = Uuid::new_v4();
    let right = Uuid::new_v4();
    let id = association_identity(&Association { left, right })?;
    associations::ActiveModel {
        uuid: Set(id.to_string()),
        left: Set(left.to_string().to_uppercase()),
        right: Set(right.to_string().to_uppercase()),
        ..Default::default()
    }
    .insert(&db)
    .await?;
    let tx = db.begin().await?;
    Catalog::scan_on(
        &tx,
        &Collection::Association,
        None,
        sync_orm::row_policy::ScanLimits {
            max_records: 128,
            max_rows: 2,
            max_bytes: 4096,
            max_mutations: 1,
        },
    )
    .await?;
    tx.commit().await?;
    let row = associations::Entity::find().one(&db).await?.unwrap();
    assert_eq!(row.left, left.to_string());
    assert_eq!(row.right, right.to_string());
    Ok(())
}
