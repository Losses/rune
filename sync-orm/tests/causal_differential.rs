use anyhow::Result;
use sea_orm::Database;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use sync::protocol::{self as p, ReceiveStore, SyncStore};
use sync_orm::{causal_capabilities::CausalEndpoint, causal_schema, causal_store::CausalReplica};
use uuid::Uuid;

fn session_limits() -> p::SessionLimits {
    p::SessionLimits {
        pages: p::PageLimits {
            max_scanned: 4,
            max_bytes: 65536,
        },
        stages: p::StageLimits {
            max_total_bytes: p::DEFAULT_STAGE_BYTES,
            max_stages: p::DEFAULT_STAGES,
            max_event_bytes: p::DEFAULT_LOGICAL_BYTES,
            max_event_items: p::DEFAULT_LOGICAL_ITEMS,
            max_fragment_bytes: p::DEFAULT_PAGE_BYTES,
        },
        apply: p::ApplyLimits {
            max_projection_items: 2,
            max_projection_bytes: 65536,
            max_mutations: 1024,
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
#[derive(Clone, Debug, PartialEq, Eq)]
struct Notes;
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Note(String);
impl p::SyncModel for Notes {
    type Collection = u8;
    type Payload = Note;
    fn model_id() -> &'static str {
        "endpoint-notes"
    }
    fn schema_id() -> &'static str {
        "1"
    }
    fn collections() -> Vec<u8> {
        vec![1]
    }
    fn collection_tag(_: &u8) -> &'static str {
        "notes"
    }
    fn collection_from_tag(value: &str) -> Option<u8> {
        (value == "notes").then_some(1)
    }
    fn validate(_: &p::ObjectKey<Self>, _: &Option<Note>) -> p::Result<()> {
        Ok(())
    }
}

async fn sync_sql(src: &CausalEndpoint<Notes>, dst: &CausalEndpoint<Notes>) -> Result<()> {
    let mut c = p::Coordinator::new(
        p::Scope {
            collections: [1].into_iter().collect(),
        },
        session_limits(),
    )?;
    for _ in 0..20000 {
        match c.advance(dst, dst, dst, src, src).await? {
            p::SessionAdvance::Complete(_) => return Ok(()),
            p::SessionAdvance::BudgetExhausted => c.start_pass(),
            p::SessionAdvance::Incomplete => (),
            p::SessionAdvance::Cancelled => anyhow::bail!("cancelled"),
        }
    }
    anyhow::bail!("SQL sync did not finish")
}
async fn sync_memory(
    src: &Arc<p::MemorySyncStore<Notes>>,
    dst: &Arc<p::MemorySyncStore<Notes>>,
) -> Result<()> {
    let mut c = p::Coordinator::new(
        p::Scope {
            collections: [1].into_iter().collect(),
        },
        session_limits(),
    )?;
    let peer = p::StorePeer(src.clone());
    for _ in 0..20000 {
        match c.advance(&**dst, &**dst, &**dst, &peer, &**src).await? {
            p::SessionAdvance::Complete(_) => return Ok(()),
            p::SessionAdvance::BudgetExhausted => c.start_pass(),
            p::SessionAdvance::Incomplete => (),
            p::SessionAdvance::Cancelled => anyhow::bail!("cancelled"),
        }
    }
    anyhow::bail!("memory sync did not finish")
}
fn random(state: &mut u64) -> u64 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    *state
}
#[tokio::test]
async fn seeded_memory_and_sql_match_after_syncs() -> Result<()> {
    for seed in 1..=40 {
        eprintln!("differential seed {seed}");
        let ns = Uuid::new_v4();
        let mut sql = Vec::new();
        let mut memory = Vec::new();
        for _ in 0..3 {
            let db = Database::connect("sqlite::memory:").await?;
            causal_schema::install_fresh(&db).await?;
            let endpoint = CausalEndpoint::<Notes>::new(
                CausalReplica::create_empty(db, ns, Uuid::new_v4()).await?,
            );
            let store = Arc::new(p::MemorySyncStore::new(
                endpoint.identity().await?,
                endpoint.local_author().await?,
            )?);
            sql.push(endpoint);
            memory.push(store);
        }
        let mut rng = seed;
        for step in 0..60 {
            let a = (random(&mut rng) % 3) as usize;
            let n = random(&mut rng) % 4 + 1;
            let key = p::ObjectKey {
                collection: 1,
                id: Uuid::from_u128(n.into()),
            };
            match random(&mut rng) % 3 {
                0 | 1 => {
                    let value = if random(&mut rng).is_multiple_of(3) {
                        None
                    } else {
                        Some(Note(format!("{seed}-{step}")))
                    };
                    sql[a].write_local(key.clone(), value.clone()).await?;
                    memory[a].write_local(key, value).await?;
                }
                _ => {
                    let b = (a + 1 + (random(&mut rng) % 2) as usize) % 3;
                    sync_sql(&sql[a], &sql[b])
                        .await
                        .map_err(|e| anyhow::anyhow!("seed {seed}, step {step}, SQL: {e:#}"))?;
                    sync_memory(&memory[a], &memory[b])
                        .await
                        .map_err(|e| anyhow::anyhow!("seed {seed}, step {step}, memory: {e:#}"))?;
                    for replica in 0..3 {
                        for id in 1..=4 {
                            let key = p::ObjectKey {
                                collection: 1,
                                id: Uuid::from_u128(id),
                            };
                            assert_eq!(
                                sql[replica].get(&key).await?,
                                memory[replica].get(&key).await?,
                                "seed {seed} step {step} replica {replica} key {id}"
                            );
                            let limits = p::PageLimits {
                                max_scanned: 128,
                                max_bytes: 65536,
                            };
                            let actual = sql[replica].candidates(&key, None, limits).await?;
                            let expected = memory[replica].candidates(&key, None, limits).await?;
                            assert!(actual.done && expected.done);
                            let a: Vec<_> = actual.values.iter().map(|c| c.dot).collect();
                            let b: Vec<_> = expected.values.iter().map(|c| c.dot).collect();
                            assert_eq!(a, b, "seed {seed} step {step} replica {replica} key {id}");
                        }
                    }
                }
            }
        }
    }
    Ok(())
}
