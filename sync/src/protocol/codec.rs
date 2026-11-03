use super::*;
use crate::causal::Context;
use serde_json::{Value, json};
use std::collections;
fn err<E: std::fmt::Display>(e: E) -> SyncError {
    SyncError::Storage(e.to_string())
}
fn valid_tag(s: &str) -> Result<()> {
    if s.is_empty() || s.len() > MAX_TAG_BYTES {
        Err(SyncError::Invalid("invalid tag".into()))
    } else {
        Ok(())
    }
}
pub fn validate_model<M: SyncModel>() -> Result<()> {
    valid_tag(M::model_id())?;
    valid_tag(M::schema_id())?;
    let all = M::collections();
    if all.is_empty() || all.len() > MAX_COLLECTIONS {
        return Err(SyncError::Invalid("invalid collection registry".into()));
    }
    let mut tags = collections::BTreeSet::new();
    for c in all {
        let tag = M::collection_tag(&c);
        valid_tag(tag)?;
        if !tags.insert(tag) || M::collection_from_tag(tag) != Some(c) {
            return Err(SyncError::Invalid("non-bijective collection tag".into()));
        }
    }
    Ok(())
}
fn sorted_json(v: &Value, out: &mut Vec<u8>) -> Result<()> {
    match v {
        Value::Object(m) => {
            out.push(b'{');
            let mut keys: Vec<_> = m.keys().collect();
            keys.sort_unstable();
            for (i, k) in keys.iter().enumerate() {
                if i > 0 {
                    out.push(b',')
                }
                serde_json::to_writer(&mut *out, k).map_err(err)?;
                out.push(b':');
                sorted_json(&m[*k], out)?;
            }
            out.push(b'}');
        }
        Value::Array(a) => {
            out.push(b'[');
            for (i, v) in a.iter().enumerate() {
                if i > 0 {
                    out.push(b',')
                }
                sorted_json(v, out)?;
            }
            out.push(b']');
        }
        _ => serde_json::to_writer(out, v).map_err(err)?,
    }
    Ok(())
}
fn encode(v: Value) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    sorted_json(&v, &mut out)?;
    Ok(out)
}
fn hex(d: &Digest) -> String {
    d.iter().map(|b| format!("{b:02x}")).collect()
}
fn lp(out: &mut Vec<u8>, bytes: &[u8]) -> Result<()> {
    let len = u32::try_from(bytes.len()).map_err(|_| SyncError::LogicalLimit)?;
    out.extend(len.to_be_bytes());
    out.extend(bytes);
    Ok(())
}
fn tagged(tag: &str, bytes: &[u8]) -> Result<Digest> {
    let mut out = Vec::new();
    lp(&mut out, tag.as_bytes())?;
    lp(&mut out, bytes)?;
    Ok(*blake3::hash(&out).as_bytes())
}
fn key_value<M: SyncModel>(k: &ObjectKey<M>) -> Value {
    json!({"collection":M::collection_tag(&k.collection),"id":k.id})
}
fn dot_value(d: Dot) -> Value {
    json!({"writer_incarnation":d.writer_incarnation,"counter":d.counter})
}
fn revision_value(r: Revision) -> Value {
    json!({"counter":r.counter,"actor":r.actor})
}
fn valid_uuid(id: Uuid) -> bool {
    !id.is_nil()
}
fn valid_revision(r: Revision) -> bool {
    valid_uuid(r.actor) && r.counter > 0 && r.counter <= MAX_NUMBER
}
pub fn validate_candidate<M: SyncModel>(c: &Candidate<M>) -> Result<()> {
    validate_model::<M>()?;
    if c.namespace.is_nil()
        || c.key.id.is_nil()
        || c.dot.writer_incarnation.is_nil()
        || c.dot.counter == 0
        || c.dot.counter > MAX_NUMBER
        || !valid_revision(c.revision)
    {
        return Err(SyncError::Invalid("invalid candidate identity".into()));
    }
    if c.model.model != M::model_id() || c.model.schema != M::schema_id() {
        return Err(SyncError::IncompatibleModel);
    }
    if !M::collections().contains(&c.key.collection)
        || M::collection_from_tag(M::collection_tag(&c.key.collection))
            != Some(c.key.collection.clone())
    {
        return Err(SyncError::Invalid("unknown collection".into()));
    }
    let v = Some(c.payload.clone());
    M::validate(&c.key, &v)?;
    M::validate_revision(&c.key, &v, &c.revision)?;
    Ok(())
}
fn candidate_value<M: SyncModel>(c: &Candidate<M>) -> Result<Value> {
    validate_candidate(c)?;
    Ok(
        json!({"model":{"model":c.model.model,"schema":c.model.schema},"namespace":c.namespace,"key":key_value(&c.key),"dot":dot_value(c.dot),"revision":revision_value(c.revision),"payload":serde_json::to_value(&c.payload).map_err(err)?}),
    )
}
pub fn canonical_candidate<M: SyncModel>(c: &Candidate<M>) -> Result<Vec<u8>> {
    let bytes = encode(candidate_value(c)?)?;
    if bytes.len() as u64 > MAX_CANDIDATE_BYTES {
        return Err(SyncError::LogicalLimit);
    }
    Ok(bytes)
}
pub fn candidate_digest<M: SyncModel>(c: &Candidate<M>) -> Result<Digest> {
    tagged("sync.candidate", &canonical_candidate(c)?)
}
/// Deterministic displayed-value ordering. All surviving candidates remain stored.
pub fn candidate_rank<M: SyncModel>(c: &Candidate<M>) -> Result<(Revision, Digest, Dot)> {
    Ok((c.revision, candidate_digest(c)?, c.dot))
}
fn kind(k: ManifestKind) -> &'static str {
    match k {
        ManifestKind::KeyEvent => "key_event",
        ManifestKind::MergeEvent => "merge_event",
        ManifestKind::Context => "context",
        ManifestKind::ContextRoot => "context_root",
    }
}
fn manifest(m: &ManifestRef) -> Value {
    json!({"kind":kind(m.kind),"manifest_digest":hex(&m.manifest_digest),"content_digest":hex(&m.content_digest),"encoded_bytes":m.encoded_bytes,"item_count":m.item_count,"fragment_count":m.fragment_count})
}
fn range<M: SyncModel>(r: &KeyRange<M>) -> Result<Value> {
    if !M::collections().contains(&r.collection) {
        return Err(SyncError::Invalid("unknown collection".into()));
    }
    if r.lower.zip(r.upper).is_some_and(|(a, b)| a >= b) {
        return Err(SyncError::Invalid("invalid range".into()));
    }
    Ok(json!({"collection":M::collection_tag(&r.collection),"lower":r.lower,"upper":r.upper}))
}
pub fn canonical_event<M: SyncModel>(e: &Event<M>) -> Result<Vec<u8>> {
    let value = match e {
        Event::Key(k) => {
            if k.added.len() > 1 {
                return Err(SyncError::Invalid(
                    "key event has multiple additions".into(),
                ));
            }
            if k.removed
                .len()
                .checked_add(k.added.len())
                .is_none_or(|n| n as u64 > MAX_LOGICAL_ITEMS)
            {
                return Err(SyncError::LogicalLimit);
            }
            if !valid_revision(k.author) || k.key.id.is_nil() {
                return Err(SyncError::Invalid("invalid key event".into()));
            }
            if k.removed.iter().any(|dot| {
                dot.writer_incarnation.is_nil() || dot.counter == 0 || dot.counter > MAX_NUMBER
            }) {
                return Err(SyncError::Invalid("invalid removed dot".into()));
            }
            if k.removed.windows(2).any(|x| x[0] >= x[1])
                || k.added.windows(2).any(|x| x[0].dot >= x[1].dot)
            {
                return Err(SyncError::Invalid("unordered event".into()));
            }
            let mut added = Vec::new();
            for c in &k.added {
                if c.key != k.key {
                    return Err(SyncError::Invalid("key mismatch".into()));
                }
                if c.revision != k.author {
                    return Err(SyncError::Invalid(
                        "candidate revision differs from operation".into(),
                    ));
                }
                canonical_candidate(c)?;
                added.push(candidate_value(c)?);
            }
            let removed: Vec<_> = k.removed.iter().copied().map(dot_value).collect();
            json!({"kind":"key","key":key_value(&k.key),"author":revision_value(k.author),"removed":removed,"added":added})
        }
        Event::Merge(m) => {
            validate_model::<M>()?;
            valid_tag(M::collection_tag(&m.collection))?;
            if !M::collections().contains(&m.collection) {
                return Err(SyncError::Invalid("unknown collection".into()));
            }
            canonical_context(&m.observed)?;
            let observed = causal_context(&m.observed)?;
            if m.added.is_empty() && observed.is_empty() {
                return Err(SyncError::Invalid("empty merge event".into()));
            }
            if m.added
                .len()
                .checked_add(observed.intervals().count())
                .is_none_or(|n| n as u64 > MAX_LOGICAL_ITEMS)
            {
                return Err(SyncError::LogicalLimit);
            }
            let mut previous = None;
            let mut dots = BTreeSet::new();
            let mut added = Vec::with_capacity(m.added.len());
            for c in &m.added {
                let cursor = CandidateCursor {
                    key: c.key.clone(),
                    dot: c.dot,
                };
                if c.key.collection != m.collection
                    || previous.as_ref().is_some_and(|p| p >= &cursor)
                    || !dots.insert(c.dot)
                    || observed.contains(crate::causal::Dot {
                        writer: c.dot.writer_incarnation,
                        counter: c.dot.counter,
                    })
                {
                    return Err(SyncError::Invalid("invalid merge additions".into()));
                }
                canonical_candidate(c)?;
                added.push(candidate_value(c)?);
                previous = Some(cursor);
            }
            json!({"kind":"merge","collection":M::collection_tag(&m.collection),"added":added,"observed":serde_json::from_slice::<Value>(&canonical_context(&m.observed)?).map_err(err)?})
        }
    };
    let bytes = encode(value)?;
    if bytes.len() as u64 > MAX_LOGICAL_BYTES {
        return Err(SyncError::LogicalLimit);
    }
    Ok(bytes)
}
pub fn event_digest<M: SyncModel>(e: &Event<M>) -> Result<Digest> {
    tagged("sync.event", &canonical_event(e)?)
}
pub fn scope_id<M: SyncModel>(scope: &Scope<M>) -> Result<Digest> {
    validate_model::<M>()?;
    if scope.collections.is_empty() || scope.collections.len() > MAX_COLLECTIONS {
        return Err(SyncError::Invalid("invalid scope".into()));
    }
    let mut tags: Vec<_> = scope.collections.iter().map(M::collection_tag).collect();
    if scope
        .collections
        .iter()
        .any(|collection| !M::collections().contains(collection))
    {
        return Err(SyncError::Invalid("unknown scoped collection".into()));
    }
    tags.sort_unstable();
    let mut out = Vec::new();
    lp(&mut out, b"sync.scope")?;
    lp(&mut out, M::model_id().as_bytes())?;
    lp(&mut out, M::schema_id().as_bytes())?;
    out.extend((tags.len() as u64).to_be_bytes());
    for t in tags {
        lp(&mut out, t.as_bytes())?;
    }
    Ok(*blake3::hash(&out).as_bytes())
}
pub fn initial_anchor(i: &StoreIdentity) -> Result<Digest> {
    if i.namespace.is_nil() || i.replica.is_nil() || i.generation.is_nil() {
        return Err(SyncError::Invalid("invalid source identity".into()));
    }
    valid_tag(&i.model.model)?;
    valid_tag(&i.model.schema)?;
    let mut v = Vec::new();
    lp(&mut v, b"sync.history.initial")?;
    v.extend(i.namespace.as_bytes());
    v.extend(i.replica.as_bytes());
    v.extend(i.generation.as_bytes());
    lp(&mut v, i.model.model.as_bytes())?;
    lp(&mut v, i.model.schema.as_bytes())?;
    Ok(*blake3::hash(&v).as_bytes())
}
pub fn next_anchor<M: SyncModel>(
    previous: Digest,
    sequence: u64,
    collection: &M::Collection,
    digest: Digest,
) -> Result<Digest> {
    if sequence == 0 || sequence > MAX_NUMBER {
        return Err(SyncError::Invalid("invalid sequence".into()));
    }
    let mut v = Vec::new();
    lp(&mut v, b"sync.history.link")?;
    v.extend(previous);
    v.extend(sequence.to_be_bytes());
    lp(&mut v, M::collection_tag(collection).as_bytes())?;
    v.extend(digest);
    Ok(*blake3::hash(&v).as_bytes())
}
fn fields<'a>(v: &'a Value, names: &[&str]) -> Result<&'a serde_json::Map<String, Value>> {
    let m = v
        .as_object()
        .ok_or(SyncError::Invalid("expected object".into()))?;
    if m.len() != names.len() || names.iter().any(|n| !m.contains_key(*n)) {
        return Err(SyncError::Invalid("unknown or missing field".into()));
    }
    Ok(m)
}
fn parse_uuid(v: &Value) -> Result<Uuid> {
    let s = v
        .as_str()
        .ok_or(SyncError::Invalid("invalid UUID".into()))?;
    let id = Uuid::parse_str(s).map_err(|_| SyncError::Invalid("invalid UUID".into()))?;
    if id.to_string() != s {
        return Err(SyncError::Invalid("noncanonical UUID".into()));
    }
    Ok(id)
}
fn number(v: &Value) -> Result<u64> {
    v.as_u64()
        .filter(|n| *n <= MAX_NUMBER)
        .ok_or(SyncError::Invalid("invalid number".into()))
}
fn decode_key<M: SyncModel>(v: &Value) -> Result<ObjectKey<M>> {
    let m = fields(v, &["collection", "id"])?;
    let tag = m["collection"]
        .as_str()
        .ok_or(SyncError::Invalid("invalid collection".into()))?;
    let collection =
        M::collection_from_tag(tag).ok_or(SyncError::Invalid("unknown collection".into()))?;
    if M::collection_tag(&collection) != tag {
        return Err(SyncError::Invalid("unstable collection".into()));
    }
    Ok(ObjectKey {
        collection,
        id: parse_uuid(&m["id"])?,
    })
}
fn decode_dot(v: &Value) -> Result<Dot> {
    let m = fields(v, &["writer_incarnation", "counter"])?;
    Ok(Dot {
        writer_incarnation: parse_uuid(&m["writer_incarnation"])?,
        counter: number(&m["counter"])?,
    })
}
fn decode_revision(v: &Value) -> Result<Revision> {
    let m = fields(v, &["counter", "actor"])?;
    Ok(Revision {
        counter: number(&m["counter"])?,
        actor: parse_uuid(&m["actor"])?,
    })
}
fn candidate_from_value<M: SyncModel>(v: &Value) -> Result<Candidate<M>> {
    let m = fields(
        v,
        &["model", "namespace", "key", "dot", "revision", "payload"],
    )?;
    let model = fields(&m["model"], &["model", "schema"])?;
    let model = ModelId {
        model: model["model"]
            .as_str()
            .ok_or(SyncError::Invalid("invalid model".into()))?
            .into(),
        schema: model["schema"]
            .as_str()
            .ok_or(SyncError::Invalid("invalid schema".into()))?
            .into(),
    };
    let candidate = Candidate {
        model,
        namespace: parse_uuid(&m["namespace"])?,
        key: decode_key(&m["key"])?,
        dot: decode_dot(&m["dot"])?,
        revision: decode_revision(&m["revision"])?,
        payload: serde_json::from_value(m["payload"].clone()).map_err(err)?,
    };
    validate_candidate(&candidate)?;
    Ok(candidate)
}
/// Decode only the one canonical form; rejects alternate encoding, duplicate and unknown fields.
pub fn decode_candidate<M: SyncModel>(bytes: &[u8]) -> Result<Candidate<M>> {
    if bytes.len() as u64 > MAX_CANDIDATE_BYTES {
        return Err(SyncError::LogicalLimit);
    }
    let value: Value = serde_json::from_slice(bytes).map_err(err)?;
    let candidate = candidate_from_value(&value)?;
    if canonical_candidate(&candidate)? != bytes {
        return Err(SyncError::Invalid("noncanonical candidate encoding".into()));
    }
    Ok(candidate)
}
fn parse_digest(v: &Value) -> Result<Digest> {
    let s = v
        .as_str()
        .ok_or(SyncError::Invalid("invalid digest".into()))?;
    if s.len() != 64
        || !s
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(SyncError::Invalid("noncanonical digest".into()));
    }
    let mut d = [0u8; 32];
    for (i, x) in d.iter_mut().enumerate() {
        *x = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16)
            .map_err(|_| SyncError::Invalid("invalid digest".into()))?;
    }
    Ok(d)
}
fn decode_manifest(v: &Value) -> Result<ManifestRef> {
    let m = fields(
        v,
        &[
            "kind",
            "manifest_digest",
            "content_digest",
            "encoded_bytes",
            "item_count",
            "fragment_count",
        ],
    )?;
    let kind = match m["kind"].as_str() {
        Some("key_event") => ManifestKind::KeyEvent,
        Some("merge_event") => ManifestKind::MergeEvent,
        Some("context") => ManifestKind::Context,
        Some("context_root") => ManifestKind::ContextRoot,
        _ => return Err(SyncError::Invalid("invalid manifest kind".into())),
    };
    Ok(ManifestRef {
        kind,
        manifest_digest: parse_digest(&m["manifest_digest"])?,
        content_digest: parse_digest(&m["content_digest"])?,
        encoded_bytes: number(&m["encoded_bytes"])?,
        item_count: number(&m["item_count"])?,
        fragment_count: u32::try_from(number(&m["fragment_count"])?).map_err(err)?,
    })
}
fn decode_range<M: SyncModel>(v: &Value) -> Result<KeyRange<M>> {
    let m = fields(v, &["collection", "lower", "upper"])?;
    let tag = m["collection"]
        .as_str()
        .ok_or(SyncError::Invalid("invalid collection".into()))?;
    let collection =
        M::collection_from_tag(tag).ok_or(SyncError::Invalid("unknown collection".into()))?;
    let lower = if m["lower"].is_null() {
        None
    } else {
        Some(parse_uuid(&m["lower"])?)
    };
    let upper = if m["upper"].is_null() {
        None
    } else {
        Some(parse_uuid(&m["upper"])?)
    };
    let value = KeyRange {
        collection,
        lower,
        upper,
    };
    range(&value)?;
    Ok(value)
}
/// Decode an exact canonical logical event; unknown, duplicate or noncanonical fields fail.
pub fn decode_event<M: SyncModel>(bytes: &[u8]) -> Result<Event<M>> {
    if bytes.len() as u64 > MAX_LOGICAL_BYTES {
        return Err(SyncError::LogicalLimit);
    }
    let v: Value = serde_json::from_slice(bytes).map_err(err)?;
    let kind = v
        .get("kind")
        .and_then(Value::as_str)
        .ok_or(SyncError::Invalid("missing event kind".into()))?;
    let event = match kind {
        "key" => {
            let m = fields(&v, &["kind", "key", "author", "removed", "added"])?;
            let removed = m["removed"]
                .as_array()
                .ok_or(SyncError::Invalid("invalid removals".into()))?
                .iter()
                .map(decode_dot)
                .collect::<Result<Vec<_>>>()?;
            let added = m["added"]
                .as_array()
                .ok_or(SyncError::Invalid("invalid additions".into()))?
                .iter()
                .map(candidate_from_value)
                .collect::<Result<Vec<_>>>()?;
            Event::Key(KeyDelta {
                key: decode_key(&m["key"])?,
                author: decode_revision(&m["author"])?,
                removed,
                added,
            })
        }
        "merge" => {
            let m = fields(&v, &["kind", "collection", "added", "observed"])?;
            let tag = m["collection"]
                .as_str()
                .ok_or(SyncError::Invalid("invalid collection".into()))?;
            let collection = M::collection_from_tag(tag)
                .filter(|c| M::collection_tag(c) == tag)
                .ok_or(SyncError::Invalid("unknown collection".into()))?;
            let added = m["added"]
                .as_array()
                .ok_or(SyncError::Invalid("invalid additions".into()))?
                .iter()
                .map(candidate_from_value)
                .collect::<Result<Vec<_>>>()?;
            let observed = decode_context(&encode(m["observed"].clone())?)?;
            Event::Merge(Box::new(MergeDelta {
                collection,
                added,
                observed,
            }))
        }
        _ => return Err(SyncError::Invalid("unknown event kind".into())),
    };
    if canonical_event(&event)? != bytes {
        return Err(SyncError::Invalid("noncanonical event encoding".into()));
    }
    Ok(event)
}
/// Canonical exact context; writer and exception order must already be canonical.
pub fn canonical_context(context: &ExactContext) -> Result<Vec<u8>> {
    if context.writers.len() as u64 > MAX_LOGICAL_ITEMS {
        return Err(SyncError::LogicalLimit);
    }
    let mut last_writer = None;
    let mut count = 0u64;
    let mut writers = Vec::with_capacity(context.writers.len());
    for w in &context.writers {
        if w.writer_incarnation.is_nil()
            || w.prefix > MAX_NUMBER
            || last_writer.is_some_and(|last| last >= w.writer_incarnation)
            || (w.prefix == 0 && w.exceptions.is_empty())
        {
            return Err(SyncError::Invalid("noncanonical context writer".into()));
        }
        last_writer = Some(w.writer_incarnation);
        let mut previous = w.prefix;
        let mut exceptions = Vec::with_capacity(w.exceptions.len());
        for interval in &w.exceptions {
            count = count.checked_add(1).ok_or(SyncError::LogicalLimit)?;
            if count > MAX_LOGICAL_ITEMS
                || interval.first <= previous + 1
                || interval.first > interval.last_inclusive
                || interval.last_inclusive > MAX_NUMBER
            {
                return Err(SyncError::Invalid("noncanonical context interval".into()));
            }
            previous = interval.last_inclusive;
            exceptions
                .push(json!({"first":interval.first,"last_inclusive":interval.last_inclusive}));
        }
        writers.push(json!({"writer_incarnation":w.writer_incarnation,"prefix":w.prefix,"exceptions":exceptions}));
    }
    let bytes = encode(json!({"writers":writers}))?;
    if bytes.len() as u64 > MAX_LOGICAL_BYTES {
        return Err(SyncError::LogicalLimit);
    }
    Ok(bytes)
}
pub fn context_digest(context: &ExactContext) -> Result<Digest> {
    tagged("sync.context", &canonical_context(context)?)
}
pub fn decode_context(bytes: &[u8]) -> Result<ExactContext> {
    if bytes.len() as u64 > MAX_LOGICAL_BYTES {
        return Err(SyncError::LogicalLimit);
    }
    let v: Value = serde_json::from_slice(bytes).map_err(err)?;
    let m = fields(&v, &["writers"])?;
    let mut writers = Vec::new();
    for w in m["writers"]
        .as_array()
        .ok_or(SyncError::Invalid("invalid context".into()))?
    {
        let item = fields(w, &["writer_incarnation", "prefix", "exceptions"])?;
        let mut exceptions = Vec::new();
        for interval in item["exceptions"]
            .as_array()
            .ok_or(SyncError::Invalid("invalid exceptions".into()))?
        {
            let e = fields(interval, &["first", "last_inclusive"])?;
            exceptions.push(CounterInterval {
                first: number(&e["first"])?,
                last_inclusive: number(&e["last_inclusive"])?,
            });
        }
        writers.push(WriterContext {
            writer_incarnation: parse_uuid(&item["writer_incarnation"])?,
            prefix: number(&item["prefix"])?,
            exceptions,
        });
    }
    let context = ExactContext { writers };
    if canonical_context(&context)? != bytes {
        return Err(SyncError::Invalid("noncanonical context encoding".into()));
    }
    Ok(context)
}
/// Convert verified protocol context to the exact causal primitive.
pub fn causal_context(context: &ExactContext) -> Result<crate::causal::Context> {
    canonical_context(context)?;
    let mut out = Context::default();
    for writer in &context.writers {
        if writer.prefix > 0 {
            out.insert_interval(writer.writer_incarnation, 1, writer.prefix)
                .map_err(err)?;
        }
        for interval in &writer.exceptions {
            out.insert_interval(
                writer.writer_incarnation,
                interval.first,
                interval.last_inclusive,
            )
            .map_err(err)?;
        }
    }
    Ok(out)
}
/// Convert the causal primitive to its canonical protocol representation.
pub fn exact_context(context: &crate::causal::Context) -> ExactContext {
    ExactContext {
        writers: context
            .entries()
            .map(|(writer, prefix, intervals)| WriterContext {
                writer_incarnation: writer,
                prefix,
                exceptions: intervals
                    .iter()
                    .map(|&(first, last_inclusive)| CounterInterval {
                        first,
                        last_inclusive,
                    })
                    .collect(),
            })
            .collect(),
    }
}

/// Encode the exact scoped collection contexts.
pub fn canonical_context_root<M: SyncModel>(root: &ContextRoot<M>) -> Result<Vec<u8>> {
    let digest = scope_id(&root.scope)?;
    if root.contexts.len() != root.scope.collections.len() {
        return Err(SyncError::Invalid("context root coverage mismatch".into()));
    }
    let mut previous: Option<&str> = None;
    let mut refs = Vec::with_capacity(root.contexts.len());
    for item in &root.contexts {
        let tag = M::collection_tag(&item.collection);
        if !root.scope.collections.contains(&item.collection) || previous.is_some_and(|p| p >= tag)
        {
            return Err(SyncError::Invalid(
                "invalid context root ordering or collection".into(),
            ));
        }
        previous = Some(tag);
        let m = &item.context.manifest;
        if m.kind != ManifestKind::Context
            || m.encoded_bytes == 0
            || m.encoded_bytes > MAX_LOGICAL_BYTES
            || m.item_count > MAX_LOGICAL_ITEMS
            || m.fragment_count == 0
            || m.fragment_count > MAX_FRAGMENTS
            || manifest_digest(m)? != m.manifest_digest
        {
            return Err(SyncError::Invalid("invalid context manifest".into()));
        }
        refs.push(json!({"collection":tag,"context":{"digest":hex(&item.context.digest),"manifest":manifest(m)}}));
    }
    let bytes = encode(json!({"scope":hex(&digest),"contexts":refs}))?;
    if bytes.len() as u64 > MAX_LOGICAL_BYTES {
        return Err(SyncError::LogicalLimit);
    }
    Ok(bytes)
}
/// Decode only against the caller's expected scope; verify exact canonical bytes.
pub fn decode_context_root<M: SyncModel>(scope: &Scope<M>, bytes: &[u8]) -> Result<ContextRoot<M>> {
    if bytes.len() as u64 > MAX_LOGICAL_BYTES {
        return Err(SyncError::LogicalLimit);
    }
    let v: Value = serde_json::from_slice(bytes).map_err(err)?;
    let fields = fields(&v, &["scope", "contexts"])?;
    if parse_digest(&fields["scope"])? != scope_id(scope)? {
        return Err(SyncError::Invalid("context root scope mismatch".into()));
    }
    let mut contexts = Vec::new();
    for entry in fields["contexts"]
        .as_array()
        .ok_or(SyncError::Invalid("invalid context root contexts".into()))?
    {
        let entry = self::fields(entry, &["collection", "context"])?;
        let tag = entry["collection"]
            .as_str()
            .ok_or(SyncError::Invalid("invalid collection tag".into()))?;
        let collection = M::collection_from_tag(tag)
            .ok_or(SyncError::Invalid("unknown collection tag".into()))?;
        let context = self::fields(&entry["context"], &["digest", "manifest"])?;
        contexts.push(CollectionContext {
            collection,
            context: ContextRef {
                digest: parse_digest(&context["digest"])?,
                manifest: decode_manifest(&context["manifest"])?,
            },
        });
    }
    let root = ContextRoot {
        scope: scope.clone(),
        contexts,
    };
    if canonical_context_root(&root)? != bytes {
        return Err(SyncError::Invalid(
            "noncanonical context root encoding".into(),
        ));
    }
    Ok(root)
}

/// Incremental candidate-only range hash, fed in cursor order.
#[derive(Clone)]
pub struct RangeHasher<M: SyncModel> {
    hash: PortableHasher,
    scope_digest: Digest,
    parent: KeyRange<M>,
    candidates_expected: u64,
    candidate_bytes_expected: u64,
    candidates_seen: u64,
    candidate_bytes_seen: u64,
    last_candidate: Option<CandidateCursor<M>>,
}
fn hash_lp(hasher: &mut PortableHasher, bytes: &[u8]) -> Result<()> {
    let n = u32::try_from(bytes.len()).map_err(|_| SyncError::LogicalLimit)?;
    hasher.update(&n.to_be_bytes())?;
    hasher.update(bytes)?;
    Ok(())
}
impl<M: SyncModel> RangeHasher<M> {
    pub fn new(
        scope: &Scope<M>,
        parent: KeyRange<M>,
        candidate_count: u64,
        candidate_bytes: u64,
    ) -> Result<Self> {
        if !scope.collections.contains(&parent.collection)
            || candidate_count > MAX_NUMBER
            || candidate_bytes > MAX_NUMBER
        {
            return Err(SyncError::LogicalLimit);
        }
        let bounds = encode(range(&parent)?)?;
        let mut hash = PortableHasher::new();
        hash_lp(&mut hash, b"sync.range")?;
        hash_lp(&mut hash, M::model_id().as_bytes())?;
        hash_lp(&mut hash, M::schema_id().as_bytes())?;
        let scope_digest = scope_id(scope)?;
        hash.update(&scope_digest)?;
        hash_lp(&mut hash, &bounds)?;
        hash.update(&candidate_count.to_be_bytes())?;
        hash.update(&candidate_bytes.to_be_bytes())?;
        Ok(Self {
            hash,
            scope_digest,
            parent,
            candidates_expected: candidate_count,
            candidate_bytes_expected: candidate_bytes,
            candidates_seen: 0,
            candidate_bytes_seen: 0,
            last_candidate: None,
        })
    }
    pub fn push_candidate(&mut self, candidate: &Candidate<M>) -> Result<()> {
        if self.candidates_seen >= self.candidates_expected
            || candidate.key.collection != self.parent.collection
            || self.parent.lower.is_some_and(|x| candidate.key.id < x)
            || self.parent.upper.is_some_and(|x| candidate.key.id >= x)
        {
            return Err(SyncError::Invalid(
                "candidate outside range or sequence".into(),
            ));
        }
        let cursor = CandidateCursor {
            key: candidate.key.clone(),
            dot: candidate.dot,
        };
        if self
            .last_candidate
            .as_ref()
            .is_some_and(|last| last >= &cursor)
        {
            return Err(SyncError::Invalid("candidate order".into()));
        }
        let encoded = canonical_candidate(candidate)?;
        let candidate_bytes_seen = self
            .candidate_bytes_seen
            .checked_add(encoded.len() as u64)
            .ok_or(SyncError::LogicalLimit)?;
        if candidate_bytes_seen > self.candidate_bytes_expected {
            return Err(SyncError::Invalid("candidate byte count".into()));
        }
        let mut hash = self.hash.clone();
        hash.update(&tagged("sync.candidate", &encoded)?)?;
        self.hash = hash;
        self.candidate_bytes_seen = candidate_bytes_seen;
        self.candidates_seen += 1;
        self.last_candidate = Some(cursor);
        Ok(())
    }
    /// Versioned portable state for durable bounded source work.
    pub fn checkpoint(&self) -> Result<Vec<u8>> {
        let hash: String = self
            .hash
            .checkpoint()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        encode(
            json!({"version":2,"model":M::model_id(),"schema":M::schema_id(),"scope":hex(&self.scope_digest),"parent":range(&self.parent)?,"candidates_expected":self.candidates_expected,"candidate_bytes_expected":self.candidate_bytes_expected,"candidates_seen":self.candidates_seen,"candidate_bytes_seen":self.candidate_bytes_seen,"last_candidate":self.last_candidate,"hash":hash}),
        )
    }
    /// Restore only for the caller's pinned scope/range; invalid state is rejected.
    pub fn from_checkpoint(scope: &Scope<M>, parent: &KeyRange<M>, bytes: &[u8]) -> Result<Self> {
        if bytes.len() > 64 * 1024 {
            return Err(SyncError::LogicalLimit);
        }
        let value: Value = serde_json::from_slice(bytes).map_err(err)?;
        let m = fields(
            &value,
            &[
                "version",
                "model",
                "schema",
                "scope",
                "parent",
                "candidates_expected",
                "candidate_bytes_expected",
                "candidates_seen",
                "candidate_bytes_seen",
                "last_candidate",
                "hash",
            ],
        )?;
        if number(&m["version"])? != 2
            || m["model"].as_str() != Some(M::model_id())
            || m["schema"].as_str() != Some(M::schema_id())
            || parse_digest(&m["scope"])? != scope_id(scope)?
            || decode_range::<M>(&m["parent"])? != *parent
            || !scope.collections.contains(&parent.collection)
            || encode(value.clone())? != bytes
        {
            return Err(SyncError::Invalid(
                "invalid range checkpoint identity".into(),
            ));
        }
        let last_candidate: Option<CandidateCursor<M>> =
            serde_json::from_value(m["last_candidate"].clone()).map_err(err)?;
        let candidates_expected = number(&m["candidates_expected"])?;
        let candidate_bytes_expected = number(&m["candidate_bytes_expected"])?;
        let candidates_seen = number(&m["candidates_seen"])?;
        let candidate_bytes_seen = number(&m["candidate_bytes_seen"])?;
        if candidates_seen > candidates_expected
            || candidate_bytes_seen > candidate_bytes_expected
            || (candidates_seen == 0 && (last_candidate.is_some() || candidate_bytes_seen != 0))
            || (candidates_seen > 0 && last_candidate.is_none())
        {
            return Err(SyncError::Invalid(
                "invalid range checkpoint progress".into(),
            ));
        }
        if let Some(cursor) = &last_candidate
            && (cursor.key.collection != parent.collection
                || parent.lower.is_some_and(|v| cursor.key.id < v)
                || parent.upper.is_some_and(|v| cursor.key.id >= v)
                || cursor.key.id.is_nil()
                || cursor.dot.writer_incarnation.is_nil()
                || cursor.dot.counter == 0
                || cursor.dot.counter > MAX_NUMBER)
        {
            return Err(SyncError::Invalid("invalid range checkpoint cursor".into()));
        }
        fn read_hash(value: &Value) -> Result<PortableHasher> {
            let text = value
                .as_str()
                .ok_or(SyncError::Invalid("invalid hash checkpoint".into()))?;
            if text.len() % 2 != 0
                || !text
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            {
                return Err(SyncError::Invalid(
                    "invalid hash checkpoint encoding".into(),
                ));
            }
            let bytes = (0..text.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(&text[i..i + 2], 16).map_err(err))
                .collect::<Result<Vec<_>>>()?;
            PortableHasher::from_checkpoint(&bytes)
        }
        let hash = read_hash(&m["hash"])?;
        let base = Self::new(
            scope,
            parent.clone(),
            candidates_expected,
            candidate_bytes_expected,
        )?;
        let main_delta = hash
            .len()
            .checked_sub(base.hash.len())
            .ok_or(SyncError::Invalid("range checkpoint hash length".into()))?;
        if main_delta
            != candidates_seen
                .checked_mul(32)
                .ok_or(SyncError::LogicalLimit)?
        {
            return Err(SyncError::Invalid("range checkpoint hash progress".into()));
        }
        Ok(Self {
            hash,
            scope_digest: scope_id(scope)?,
            parent: parent.clone(),
            candidates_expected,
            candidate_bytes_expected,
            candidates_seen,
            candidate_bytes_seen,
            last_candidate,
        })
    }
    pub fn finish(self) -> Result<Digest> {
        if self.candidates_seen != self.candidates_expected
            || self.candidate_bytes_seen != self.candidate_bytes_expected
        {
            return Err(SyncError::IncompleteCoverage);
        }
        Ok(self.hash.finish())
    }
}

/// Decode peer bytes, classifying serialization failures as invalid peer input.
pub fn decode_candidate_remote<M: SyncModel>(bytes: &[u8]) -> Result<Candidate<M>> {
    decode_candidate(bytes).map_err(remote_decode_error)
}
pub fn decode_event_remote<M: SyncModel>(bytes: &[u8]) -> Result<Event<M>> {
    decode_event(bytes).map_err(remote_decode_error)
}
pub fn decode_context_remote(bytes: &[u8]) -> Result<ExactContext> {
    decode_context(bytes).map_err(remote_decode_error)
}
pub fn decode_context_root_remote<M: SyncModel>(
    scope: &Scope<M>,
    bytes: &[u8],
) -> Result<ContextRoot<M>> {
    decode_context_root(scope, bytes).map_err(remote_decode_error)
}
fn remote_decode_error(error: SyncError) -> SyncError {
    match error {
        SyncError::Storage(message) => SyncError::Invalid(message),
        other => other,
    }
}
