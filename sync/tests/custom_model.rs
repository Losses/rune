use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use sync::{self as p, ReceiveStore, SyncModel, SyncStore};
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq)]
struct Inventory;
#[derive(Clone, Debug, PartialEq, Eq)]
struct Unchecked;
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Collection {
    Items,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Item {
    quantity: i32,
}
macro_rules! model {
    ($model:ty, $check:expr) => {
        impl SyncModel for $model {
            type Collection = Collection;
            type Payload = Item;
            fn model_id() -> &'static str {
                "inventory"
            }
            fn schema_id() -> &'static str {
                "1"
            }
            fn collections() -> Vec<Collection> {
                vec![Collection::Items]
            }
            fn collection_tag(_: &Collection) -> &'static str {
                "items"
            }
            fn collection_from_tag(tag: &str) -> Option<Collection> {
                (tag == "items").then_some(Collection::Items)
            }
            fn validate(_: &p::ObjectKey<Self>, value: &Option<Item>) -> p::Result<()> {
                if $check && value.as_ref().is_some_and(|item| item.quantity < 0) {
                    Err(p::SyncError::Invalid("negative inventory".into()))
                } else {
                    Ok(())
                }
            }
        }
    };
}
model!(Inventory, true);
model!(Unchecked, false);
fn id(n: u128) -> Uuid {
    Uuid::from_u128(n)
}
fn key(n: u128) -> p::ObjectKey<Inventory> {
    p::ObjectKey {
        collection: Collection::Items,
        id: id(n),
    }
}
fn scope() -> p::Scope<Inventory> {
    p::Scope {
        collections: [Collection::Items].into_iter().collect(),
    }
}
fn store(n: u128) -> Result<Arc<p::MemorySyncStore<Inventory>>> {
    Ok(Arc::new(p::MemorySyncStore::new(
        p::StoreIdentity {
            namespace: id(1),
            model: p::ModelId {
                model: "inventory".into(),
                schema: "1".into(),
            },
            replica: id(n),
            generation: Uuid::new_v4(),
        },
        p::LocalAuthor {
            author: id(n),
            writer_incarnation: Uuid::new_v4(),
        },
    )?))
}
fn limits() -> p::SessionLimits {
    p::SessionLimits {
        pages: p::PageLimits {
            max_scanned: 4,
            max_bytes: 65536,
        },
        apply: p::ApplyLimits {
            max_projection_items: 4,
            max_projection_bytes: 65536,
            max_mutations: 1024,
        },
        stages: p::StageLimits {
            max_total_bytes: p::DEFAULT_STAGE_BYTES,
            max_stages: p::DEFAULT_STAGES,
            max_event_bytes: p::DEFAULT_LOGICAL_BYTES,
            max_event_items: p::DEFAULT_LOGICAL_ITEMS,
            max_fragment_bytes: 65536,
        },
        work: p::WorkLimits {
            max_requests: 32,
            max_bytes: 1024 * 1024,
            max_depth: 32,
            max_rows_scanned: 16,
            max_mutations: 1024,
        },
    }
}
async fn initialize(
    source: Arc<p::MemorySyncStore<Inventory>>,
    target: &p::MemorySyncStore<Inventory>,
) -> Result<p::CheckpointKey> {
    let mut coordinator = p::Coordinator::new(scope(), limits())?;
    let peer = p::StorePeer(source.clone());
    let identity = source.identity().await?;
    for _ in 0..2000 {
        match coordinator
            .advance(target, target, target, &peer, &*source)
            .await?
        {
            p::SessionAdvance::Complete(_) => {
                return Ok(p::CheckpointKey {
                    namespace: id(1),
                    source_replica: identity.replica,
                    source_generation: identity.generation,
                    scope_id: p::scope_id(&scope())?,
                });
            }
            p::SessionAdvance::BudgetExhausted => coordinator.start_pass(),
            p::SessionAdvance::Incomplete => (),
            p::SessionAdvance::Cancelled => anyhow::bail!("unexpected cancellation"),
        }
    }
    anyhow::bail!("fixture did not initialize")
}
async fn page(
    source: &p::MemorySyncStore<Inventory>,
    after: p::ReceiveProgress,
) -> Result<p::ChangePage<Inventory>> {
    let p::PageRead::Page(page) = source
        .read_page(p::PageRequest {
            source: source.identity().await?,
            scope: scope(),
            after,
            through: source.head().await?,
            limits: limits().pages,
        })
        .await?
    else {
        anyhow::bail!("fixture exceeded one page");
    };
    Ok(page)
}
// A hostile sender can hash a payload without applying the receiver's validation.
fn rehash(page: &mut p::ChangePage<Inventory>) -> Result<()> {
    let mut anchor = page.request.after.anchor;
    for link in &mut page.links {
        let Some(p::EventRef::Inline(event)) = &link.event else {
            anyhow::bail!("expected inline event");
        };
        let unchecked: p::Event<Unchecked> = serde_json::from_slice(&serde_json::to_vec(event)?)?;
        link.event_digest = p::event_digest(&unchecked)?;
        anchor = p::next_anchor::<Inventory>(
            anchor,
            link.sequence,
            &link.collection,
            link.event_digest,
        )?;
    }
    page.next.anchor = anchor;
    page.request.through = page.next.clone();
    Ok(())
}
#[tokio::test]
async fn custom_model_merges_and_rejects_invalid_page_atomically() -> Result<()> {
    let source = store(2)?;
    let target = store(3)?;
    let ck = initialize(source.clone(), &target).await?;
    let after = target.checkpoint(&ck).await?.unwrap();
    let valid = source
        .write_local(key(1), Some(Item { quantity: 9 }))
        .await?;
    source
        .write_local(key(2), Some(Item { quantity: 4 }))
        .await?;
    let original = page(&source, after.clone()).await?;
    let mut invalid = original.clone();
    let Some(p::EventRef::Inline(p::Event::Key(delta))) = &mut invalid.links[1].event else {
        unreachable!()
    };
    delta.added[0].payload.quantity = -1;
    rehash(&mut invalid)?;
    let before = target.head().await?;
    assert!(target.apply_page(&invalid, limits().apply).await.is_err());
    assert_eq!(target.head().await?, before);
    assert!(target.get(&key(1)).await?.is_none());
    assert_eq!(target.checkpoint(&ck).await?, Some(after));
    target.apply_page(&original, limits().apply).await?;
    let committed = target.head().await?;
    target.apply_page(&original, limits().apply).await?;
    assert_eq!(target.head().await?, committed);
    assert_eq!(target.get(&key(1)).await?, Some(valid));
    Ok(())
}
#[test]
fn equal_revision_live_candidates_have_a_total_order() -> Result<()> {
    let candidates: Vec<_> = (1..=3)
        .map(|n| p::Candidate::<Inventory> {
            model: p::ModelId {
                model: "inventory".into(),
                schema: "1".into(),
            },
            namespace: id(1),
            key: key(1),
            dot: p::Dot {
                writer_incarnation: id(n + 10),
                counter: 1,
            },
            revision: p::Revision {
                counter: 7,
                actor: id(2),
            },
            payload: Item { quantity: n as i32 },
        })
        .collect();
    let max =
        |a: &p::Candidate<Inventory>, b: &p::Candidate<Inventory>| -> p::Candidate<Inventory> {
            if p::candidate_rank(a).unwrap() >= p::candidate_rank(b).unwrap() {
                a.clone()
            } else {
                b.clone()
            }
        };
    for a in &candidates {
        for b in &candidates {
            for c in &candidates {
                assert_eq!(max(a, b), max(b, a));
                assert_eq!(max(&max(a, b), c), max(a, &max(b, c)));
                assert_eq!(max(a, a), *a);
            }
        }
    }
    Ok(())
}
#[tokio::test]
async fn observing_maximum_counter_makes_local_overflow_atomic() -> Result<()> {
    let source = store(2)?;
    let target = store(3)?;
    let ck = initialize(source.clone(), &target).await?;
    let after = target.checkpoint(&ck).await?.unwrap();
    source
        .write_local(key(1), Some(Item { quantity: 5 }))
        .await?;
    let mut page = page(&source, after).await?;
    let Some(p::EventRef::Inline(p::Event::Key(delta))) = &mut page.links[0].event else {
        unreachable!()
    };
    delta.author.counter = p::MAX_NUMBER;
    delta.added[0].revision = delta.author;
    rehash(&mut page)?;
    target.apply_page(&page, limits().apply).await?;
    let before = target.head().await?;
    assert!(
        target
            .write_local(key(2), Some(Item { quantity: 1 }))
            .await
            .is_err()
    );
    assert_eq!(target.head().await?, before);
    assert!(target.get(&key(2)).await?.is_none());
    Ok(())
}
