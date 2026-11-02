#[path = "support/causal_library.rs"]
mod support;
use anyhow::Result;
use database::sync::model::*;
use support::{exchange_scoped, key, memory, playlist};
use sync::{self as p};
use uuid::Uuid;
fn id(n: u128) -> Uuid {
    Uuid::from_u128(n)
}
fn scope(c: Collection) -> Scope {
    Scope {
        collections: [c].into_iter().collect(),
    }
}
fn checkpoint(source: &p::StoreIdentity, scope: &Scope) -> Result<p::CheckpointKey> {
    Ok(p::CheckpointKey {
        namespace: source.namespace,
        source_replica: source.replica,
        source_generation: source.generation,
        scope_id: p::scope_id(scope)?,
    })
}

#[tokio::test]
async fn scopes_have_independent_progress_and_filtered_pages_advance() -> Result<()> {
    let source = memory(id(1), id(2))?;
    let local = memory(id(1), id(3))?;
    let lists = scope(Collection::Playlists);
    let tracks = scope(Collection::Tracks);
    exchange_scoped(&source, &local, lists.clone()).await?;
    let source_id = source.identity().await?;
    let ck = checkpoint(&source_id, &lists)?;
    let after = local.checkpoint(&ck).await?.unwrap();
    let track = key(Collection::Tracks, id(10));
    let list = key(Collection::Playlists, id(11));
    source
        .write_local(track.clone(), Some(support::track()))
        .await?;
    source.write_local(list.clone(), playlist("a")).await?;
    let p::PageRead::Page(page) = source
        .read_page(p::PageRequest {
            source: source_id,
            scope: lists.clone(),
            after,
            through: source.head().await?,
            limits: p::PageLimits {
                max_scanned: 1,
                max_bytes: 10000,
            },
        })
        .await?
    else {
        anyhow::bail!("fixture exceeded one page");
    };
    assert!(page.links.iter().all(|link| link.event.is_none()));
    assert_eq!(page.next.sequence, 1);
    local.apply_page(&page, support::limits().apply).await?;
    assert_eq!(local.checkpoint(&ck).await?, Some(page.next));
    exchange_scoped(&source, &local, lists).await?;
    assert!(local.get(&track).await?.is_none());
    assert!(local.get(&list).await?.is_some());
    exchange_scoped(&source, &local, tracks).await?;
    assert!(local.get(&track).await?.is_some());
    assert!(local.get(&list).await?.is_some());
    Ok(())
}

#[tokio::test]
async fn rejects_hostile_page_self_and_other_library() -> Result<()> {
    let source = memory(id(1), id(2))?;
    let local = memory(id(1), id(3))?;
    let selected = scope(Collection::Playlists);
    exchange_scoped(&source, &local, selected.clone()).await?;
    let source_id = source.identity().await?;
    let ck = checkpoint(&source_id, &selected)?;
    let after = local.checkpoint(&ck).await?.unwrap();
    source
        .write_local(key(Collection::Playlists, id(5)), playlist("real value"))
        .await?;
    let p::PageRead::Page(mut page) = source
        .read_page(p::PageRequest {
            source: source_id,
            scope: selected.clone(),
            after: after.clone(),
            through: source.head().await?,
            limits: support::limits().pages,
        })
        .await?
    else {
        anyhow::bail!("fixture exceeded one page");
    };
    let head = local.head().await?;
    page.request.source.generation = id(999);
    assert!(
        local
            .apply_page(&page, support::limits().apply)
            .await
            .is_err()
    );
    assert_eq!(local.head().await?, head);
    assert_eq!(local.checkpoint(&ck).await?, Some(after));
    assert!(
        exchange_scoped(&source, &source, selected.clone())
            .await
            .is_err()
    );
    assert!(
        exchange_scoped(&source, &memory(id(50), id(4))?, selected)
            .await
            .is_err()
    );
    Ok(())
}

#[tokio::test]
async fn rejects_malformed_pages_without_committing() -> Result<()> {
    let source = memory(id(1), id(2))?;
    let selected = scope(Collection::Playlists);
    let mut locals = Vec::new();
    for n in 0..7 {
        let local = memory(id(1), id(3 + n))?;
        exchange_scoped(&source, &local, selected.clone()).await?;
        locals.push(local);
    }
    let source_id = source.identity().await?;
    let ck = checkpoint(&source_id, &selected)?;
    let after = locals[0].checkpoint(&ck).await?.unwrap();
    source
        .write_local(key(Collection::Playlists, id(50)), playlist("one"))
        .await?;
    let p::PageRead::Page(original) = source
        .read_page(p::PageRequest {
            source: source_id,
            scope: selected,
            after: after.clone(),
            through: source.head().await?,
            limits: support::limits().pages,
        })
        .await?
    else {
        anyhow::bail!("fixture exceeded one page");
    };
    for (fault, local) in locals.iter().enumerate() {
        let mut page = original.clone();
        match fault {
            0 => page.done = false,
            1 => {
                page.next = page.request.after.clone();
                page.done = false;
                page.links.clear();
            }
            2 => page.request.after.sequence += 1,
            3 => page.request.source.namespace = id(999),
            4 => page.request.source.replica = id(999),
            5 => page.next.anchor[0] ^= 1,
            6 => page.links.clear(),
            _ => unreachable!(),
        }
        let head = local.head().await?;
        assert!(
            local
                .apply_page(&page, support::limits().apply)
                .await
                .is_err(),
            "fault {fault}"
        );
        assert_eq!(local.checkpoint(&ck).await?, Some(after.clone()));
        assert_eq!(local.head().await?, head);
        assert!(
            local
                .get(&key(Collection::Playlists, id(50)))
                .await?
                .is_none()
        );
    }
    Ok(())
}
