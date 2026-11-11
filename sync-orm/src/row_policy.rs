//! Shared transactional projection of source rows.
//!
//! References are ordinary portable UUID columns. They are never followed or
//! rewritten during projection; callers resolve them in their own read queries.
use base64::{Engine as _, engine::general_purpose::STANDARD};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, DbErr, EntityTrait, QueryFilter, QueryOrder,
    QuerySelect, QueryTrait, Value, sea_query::Expr,
};
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::HashSet;
use std::marker::PhantomData;
use uuid::Uuid;

/// Binary source-column data serialized as canonical standard base64 text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BinaryValue(pub Vec<u8>);

impl Serialize for BinaryValue {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&STANDARD.encode(&self.0))
    }
}

impl<'de> Deserialize<'de> for BinaryValue {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let encoded = String::deserialize(deserializer)?;
        STANDARD
            .decode(encoded.as_bytes())
            .map(Self)
            .map_err(D::Error::custom)
    }
}

impl FieldCodec for BinaryValue {
    fn from_column(value: Value) -> Result<Self, DbErr> {
        <Vec<u8> as FieldCodec>::from_column(value).map(Self)
    }
    fn to_column(&self) -> Value {
        self.0.clone().into()
    }
    fn sample() -> Value {
        Vec::<u8>::new().into()
    }
}

/// Conversion between portable values and source-column values.
pub trait FieldCodec: Sized {
    fn from_column(value: Value) -> Result<Self, DbErr>;
    fn to_column(&self) -> Value;
    fn sample() -> Value;
    fn is_uuid() -> bool {
        false
    }
    fn validate(&self) -> Result<(), DbErr> {
        Ok(())
    }
}

/// A selected source row. Reads can only access explicitly selected columns;
/// local binary bodies are never fetched merely to encode portable metadata.
pub struct RowView<E: EntityTrait> {
    row: sea_orm::QueryResult,
    marker: PhantomData<fn() -> E>,
}
impl<E: EntityTrait> RowView<E> {
    pub(crate) fn new(row: sea_orm::QueryResult) -> Self {
        Self {
            row,
            marker: PhantomData,
        }
    }
    pub fn read<T: FieldCodec>(&self, column: E::Column) -> Result<T, DbErr> {
        use sea_orm::IdenStatic;
        T::from_column(column_value(&self.row, column.as_str(), T::sample())?)
    }
}
/// Use the declared codec's SQL value type, including nullable values.
fn column_value(row: &sea_orm::QueryResult, name: &str, sample: Value) -> Result<Value, DbErr> {
    macro_rules! read {
        ($ty:ty) => {
            row.try_get::<Option<$ty>>("", name)?.into()
        };
    }
    Ok(match sample {
        Value::Bool(_) => read!(bool),
        Value::TinyInt(_) => read!(i8),
        Value::SmallInt(_) => read!(i16),
        Value::Int(_) => read!(i32),
        Value::BigInt(_) => read!(i64),
        Value::TinyUnsigned(_) => read!(u8),
        Value::SmallUnsigned(_) => read!(u16),
        Value::Unsigned(_) => read!(u32),
        Value::Float(_) => read!(f32),
        Value::Double(_) => read!(f64),
        Value::String(_) => read!(String),
        Value::Bytes(_) => read!(Vec<u8>),
        Value::Decimal(_) => read!(sea_orm::prelude::Decimal),
        _ => return Err(DbErr::Custom("unsupported source column type".into())),
    })
}
macro_rules! scalar_codecs {
    ($($ty:ty),* $(,)?) => { $(impl FieldCodec for $ty {
        fn from_column(value: Value) -> Result<Self, DbErr> {
            <$ty as sea_orm::sea_query::ValueType>::try_from(value)
                .map_err(|e| DbErr::Custom(format!("invalid field: {e}")))
        }
        fn to_column(&self) -> Value { self.clone().into() }
        fn sample() -> Value { <$ty>::default().into() }
    })* };
}
scalar_codecs!(
    String,
    bool,
    i8,
    i16,
    i32,
    i64,
    u8,
    u16,
    u32,
    Option<String>,
    Option<bool>,
    Option<i8>,
    Option<i16>,
    Option<i32>,
    Option<i64>,
    Option<u8>,
    Option<u16>,
    Option<u32>,
    Vec<u8>
);
impl FieldCodec for Uuid {
    fn is_uuid() -> bool {
        true
    }
    fn sample() -> Value {
        Uuid::nil().to_string().into()
    }
    fn from_column(value: Value) -> Result<Self, DbErr> {
        let text = <String as FieldCodec>::from_column(value)?;
        let id = Uuid::parse_str(&text).map_err(|e| DbErr::Custom(e.to_string()))?;
        if id.is_nil() {
            return Err(DbErr::Custom("nil UUID".into()));
        }
        Ok(id)
    }
    fn to_column(&self) -> Value {
        self.to_string().into()
    }
    fn validate(&self) -> Result<(), DbErr> {
        if self.is_nil() {
            Err(DbErr::Custom("nil UUID".into()))
        } else {
            Ok(())
        }
    }
}
impl FieldCodec for Option<Uuid> {
    fn is_uuid() -> bool {
        true
    }
    fn sample() -> Value {
        Option::<String>::None.into()
    }
    fn from_column(value: Value) -> Result<Self, DbErr> {
        <Option<String> as FieldCodec>::from_column(value)?
            .map(|text| {
                let id = Uuid::parse_str(&text).map_err(|e| DbErr::Custom(e.to_string()))?;
                if id.is_nil() {
                    return Err(DbErr::Custom("nil UUID".into()));
                }
                Ok(id)
            })
            .transpose()
    }
    fn to_column(&self) -> Value {
        self.map(|id| id.to_string()).into()
    }
    fn validate(&self) -> Result<(), DbErr> {
        if self.is_some_and(|id| id.is_nil()) {
            Err(DbErr::Custom("nil UUID".into()))
        } else {
            Ok(())
        }
    }
}

/// Types permitted for stable references.
pub trait ReferenceField {
    fn reference(&self) -> Option<Uuid>;
}
impl ReferenceField for Uuid {
    fn reference(&self) -> Option<Uuid> {
        Some(*self)
    }
}
impl ReferenceField for Option<Uuid> {
    fn reference(&self) -> Option<Uuid> {
        *self
    }
}
pub fn require_reference<T: ReferenceField>(_: &T) {}

/// Bounds apply to integer values or UTF-8 byte length for strings.
pub trait BoundedField {
    fn check_bounds(&self, min: i128, max: i128) -> Result<(), DbErr>;
}
macro_rules! numeric_bounds {
    ($($ty:ty),* $(,)?) => { $(impl BoundedField for $ty {
        fn check_bounds(&self, min: i128, max: i128) -> Result<(), DbErr> {
            let value = i128::from(*self);
            if min > max || value < min || value > max { Err(DbErr::Custom("field outside declared bounds".into())) } else { Ok(()) }
        }
    })* };
}
numeric_bounds!(i8, i16, i32, i64, u8, u16, u32, u64);
impl BoundedField for String {
    fn check_bounds(&self, min: i128, max: i128) -> Result<(), DbErr> {
        let n = self.len() as i128;
        if min > max || n < min || n > max {
            Err(DbErr::Custom(
                "string length outside declared bounds".into(),
            ))
        } else {
            Ok(())
        }
    }
}
impl BoundedField for Vec<u8> {
    fn check_bounds(&self, min: i128, max: i128) -> Result<(), DbErr> {
        let size = self.len() as i128;
        if min > max || size < min || size > max {
            Err(DbErr::Custom("binary field exceeds declared bounds".into()))
        } else {
            Ok(())
        }
    }
}
impl BoundedField for BinaryValue {
    fn check_bounds(&self, min: i128, max: i128) -> Result<(), DbErr> {
        self.0.check_bounds(min, max)
    }
}
impl<T: BoundedField> BoundedField for Option<T> {
    fn check_bounds(&self, min: i128, max: i128) -> Result<(), DbErr> {
        if let Some(value) = self {
            value.check_bounds(min, max)?;
        }
        Ok(())
    }
}

/// A declaration supplies field mapping and validation, while this executor owns CRUD.
#[async_trait::async_trait]
pub trait RowMapping<E: EntityTrait>: Send + Sync {
    type Payload: Clone + Send + Sync;
    fn uuid_column() -> E::Column;
    fn columns() -> Vec<E::Column>;
    fn encode(model: &RowView<E>) -> Result<Self::Payload, DbErr>;
    fn decode(value: &Self::Payload) -> Result<Vec<Value>, DbErr>;
    fn field_samples() -> Vec<Value>;
    fn uuid_fields() -> Vec<bool> {
        Vec::new()
    }
    fn validate(id: Uuid, value: Option<&Self::Payload>) -> Result<(), DbErr>;
    fn authoring_owner(_value: &Self::Payload) -> Option<Uuid> {
        None
    }
    fn assign_identity(_value: &Self::Payload) -> Result<Uuid, DbErr> {
        Ok(Uuid::new_v4())
    }
    fn validate_revision(
        _id: Uuid,
        _value: Option<&Self::Payload>,
        _revision: &sync::protocol::Revision,
    ) -> Result<(), DbErr> {
        Ok(())
    }
    fn validate_declaration() -> Result<(), DbErr> {
        Ok(())
    }
    fn reference_tags() -> Vec<&'static str> {
        Vec::new()
    }
    fn references(_value: &Self::Payload) -> Vec<(&'static str, Uuid)> {
        Vec::new()
    }
    fn local_defaults() -> Vec<(E::Column, Value)> {
        Vec::new()
    }
    fn local_read_columns() -> Vec<E::Column> {
        Vec::new()
    }
    fn source_filter() -> sea_orm::Condition {
        sea_orm::Condition::all()
    }
    fn read_ceiling() -> u64 {
        1
    }
    fn projection_ceiling() -> crate::owned_list::ListBudget {
        crate::owned_list::ListBudget {
            max_reads: 2,
            max_mutations: 1,
        }
    }
    async fn hydrate_on<C: ConnectionTrait>(
        _db: &C,
        _row: &RowView<E>,
        _value: &mut Self::Payload,
    ) -> Result<u64, DbErr> {
        Ok(0)
    }
    async fn project_children_on(
        _tx: &sea_orm::DatabaseTransaction,
        _row: &RowView<E>,
        _value: Option<&Self::Payload>,
    ) -> Result<(), DbErr> {
        Ok(())
    }
    fn local_updates(
        _before: Option<&RowView<E>>,
        _value: &Self::Payload,
    ) -> Result<Vec<(E::Column, Value)>, DbErr> {
        Ok(Vec::new())
    }
}

/// Hard bounds for one source traversal transaction.
#[derive(Clone, Copy, Debug)]
pub struct ScanLimits {
    pub max_records: u32,
    pub max_rows: u32,
    pub max_bytes: u64,
    pub max_mutations: u64,
}
#[derive(Clone, Debug)]
pub struct ScanRecord<P> {
    pub position: i64,
    pub id: Uuid,
    pub payload: P,
}
#[derive(Clone, Debug)]
pub struct ScanPage<P> {
    pub records: Vec<ScanRecord<P>>,
    pub next: Option<i64>,
    pub done: bool,
    pub rows_read: u64,
    pub bytes: u64,
    pub mutations: u64,
}

pub struct RowPolicy<E: EntityTrait, M: RowMapping<E>>(PhantomData<(E, M)>);
impl<E: EntityTrait, M: RowMapping<E>> RowPolicy<E, M> {
    fn select() -> sea_orm::Select<E> {
        use sea_orm::{IdenStatic, Iterable, PrimaryKeyToColumn};
        let mut selected = HashSet::new();
        let mut query = E::find().select_only().filter(M::source_filter());
        for column in <E::PrimaryKey as Iterable>::iter()
            .map(|key| key.into_column())
            .chain(std::iter::once(M::uuid_column()))
            .chain(M::columns())
            .chain(M::local_read_columns())
        {
            if selected.insert(column.as_str()) {
                query = query.column(column);
            }
        }
        query
    }
    async fn row_on<C: ConnectionTrait>(db: &C, id: Uuid) -> Result<Option<RowView<E>>, DbErr> {
        let query = Self::select().filter(M::uuid_column().eq(id.to_string()));
        Ok(db
            .query_one_raw(query.build(db.get_database_backend()))
            .await?
            .map(RowView::new))
    }
    pub async fn local_id_on<C: ConnectionTrait>(db: &C, id: Uuid) -> Result<Option<i64>, DbErr> {
        use sea_orm::{Iterable, PrimaryKeyToColumn, QueryTrait};
        let primary = <E::PrimaryKey as Iterable>::iter()
            .next()
            .ok_or_else(|| DbErr::Custom("missing local primary key".into()))?
            .into_column();
        let query = E::find()
            .select_only()
            .column(primary)
            .filter(M::source_filter())
            .filter(M::uuid_column().eq(id.to_string()))
            .limit(1);
        db.query_one_raw(query.build(db.get_database_backend()))
            .await?
            .map(|row| row.try_get_by_index(0))
            .transpose()
    }
    pub async fn source_uuid_on<C: ConnectionTrait>(
        db: &C,
        local_id: i64,
    ) -> Result<Option<Uuid>, DbErr> {
        use sea_orm::{Iterable, PrimaryKeyToColumn, QueryTrait};
        let primary = <E::PrimaryKey as Iterable>::iter()
            .next()
            .ok_or_else(|| DbErr::Custom("missing local primary key".into()))?
            .into_column();
        let query = E::find()
            .select_only()
            .column(M::uuid_column())
            .filter(M::source_filter())
            .filter(primary.eq(local_id))
            .limit(1);
        db.query_one_raw(query.build(db.get_database_backend()))
            .await?
            .map(|row| {
                let value: String = row.try_get_by_index(0)?;
                let id = Uuid::parse_str(&value).map_err(|e| DbErr::Custom(e.to_string()))?;
                if id.is_nil() {
                    return Err(DbErr::Custom("nil source UUID".into()));
                }
                Ok(id)
            })
            .transpose()
    }

    /// Check column ownership and local defaults before opening a store.
    pub fn validate_mapping() -> Result<(), DbErr> {
        use sea_orm::{IdenStatic, Iterable, PrimaryKeyToColumn};
        M::validate_declaration()?;
        let mut names = HashSet::new();
        names.insert(M::uuid_column().as_str());
        for key in <E::PrimaryKey as Iterable>::iter() {
            if !names.insert(key.into_column().as_str()) {
                return Err(DbErr::Custom(
                    "source UUID must be separate from local primary key".into(),
                ));
            }
        }
        for column in M::columns() {
            if !names.insert(column.as_str()) {
                return Err(DbErr::Custom(format!(
                    "duplicate owned column: {}",
                    column.as_str()
                )));
            }
        }
        let mut active = <E::ActiveModel as ActiveModelTrait>::default();
        active.try_set(M::uuid_column(), Uuid::nil().to_string().into())?;
        let columns = M::columns();
        let samples = M::field_samples();
        if columns.len() != samples.len() {
            return Err(DbErr::Custom("field sample count mismatch".into()));
        }
        for (column, sample) in columns.into_iter().zip(samples) {
            active.try_set(column, sample)?;
        }
        for (column, value) in M::local_defaults() {
            if !names.insert(column.as_str()) {
                return Err(DbErr::Custom(format!(
                    "overlapping local default: {}",
                    column.as_str()
                )));
            }
            active.try_set(column, value)?;
        }
        Ok(())
    }
    /// Verify the SQLite source table has one integer primary key and a
    /// non-partial single-column unique index for its text UUID.
    pub async fn validate_database_on<C: ConnectionTrait>(db: &C) -> Result<(), DbErr> {
        use sea_orm::{
            ExprTrait, IdenStatic, Iterable, PrimaryKeyToColumn,
            sea_query::{Alias, Func, Order, Query},
        };
        Self::validate_mapping()?;
        let backend = db.get_database_backend();
        if backend != sea_orm::DbBackend::Sqlite {
            return Err(DbErr::Custom(
                "source schema validation supports SQLite only".into(),
            ));
        }
        let table = E::default().table_name().to_owned();
        let column_query = Query::select()
            .columns(["name", "type", "pk", "notnull", "dflt_value"].map(Alias::new))
            .from_function(
                Func::cust(Alias::new("pragma_table_info")).arg(table.clone()),
                Alias::new("source_columns"),
            )
            .to_owned();
        let columns = db.query_all(&column_query).await?;
        if columns.is_empty() {
            return Err(DbErr::Custom(format!("missing source table {table}")));
        }
        let mut primary = Vec::new();
        let mut uuid_text = false;
        let supplied: std::collections::HashSet<_> = std::iter::once(M::uuid_column())
            .chain(M::columns())
            .chain(M::local_defaults().into_iter().map(|(column, _)| column))
            .map(|column| column.as_str())
            .collect();
        for column in columns {
            let name: String = column.try_get("", "name")?;
            let ty: String = column.try_get("", "type")?;
            let primary_position: i64 = column.try_get("", "pk")?;
            let required: i64 = column.try_get("", "notnull")?;
            let default: Option<String> = column.try_get("", "dflt_value")?;
            if primary_position == 0
                && required != 0
                && default.is_none()
                && !supplied.contains(name.as_str())
            {
                return Err(DbErr::Custom(format!(
                    "{table}.{name}: required local column needs an insertion default"
                )));
            }
            let pk: i64 = column.try_get("", "pk")?;
            if pk > 0 {
                primary.push((name.clone(), ty.clone()));
            }
            if name == M::uuid_column().as_str() {
                let upper = ty.to_ascii_uppercase();
                uuid_text =
                    upper.contains("CHAR") || upper.contains("TEXT") || upper.contains("CLOB");
            }
        }
        if !uuid_text {
            return Err(DbErr::Custom(format!(
                "{table}: source UUID column must have text affinity"
            )));
        }
        let mut declared_keys = <E::PrimaryKey as Iterable>::iter();
        let declared = declared_keys.next().map(|key| key.into_column().as_str());
        if primary.len() != 1
            || declared_keys.next().is_some()
            || declared != Some(primary[0].0.as_str())
            || !primary[0].1.to_ascii_uppercase().contains("INT")
            || primary[0].0 == M::uuid_column().as_str()
        {
            return Err(DbErr::Custom(format!(
                "{table}: expected one separate integer primary key"
            )));
        }
        let index_query = Query::select()
            .column(Alias::new("name"))
            .from_function(
                Func::cust(Alias::new("pragma_index_list")).arg(table.clone()),
                Alias::new("source_indexes"),
            )
            .and_where(Expr::col(Alias::new("unique")).eq(1))
            .and_where(Expr::col(Alias::new("partial")).eq(0))
            .to_owned();
        let indexes = db.query_all(&index_query).await?;
        for index in indexes {
            let name: String = index.try_get("", "name")?;
            let indexed_query = Query::select()
                .column(Alias::new("name"))
                .from_function(
                    Func::cust(Alias::new("pragma_index_info")).arg(name),
                    Alias::new("index_columns"),
                )
                .order_by(Alias::new("seqno"), Order::Asc)
                .to_owned();
            let indexed = db.query_all(&indexed_query).await?;
            if indexed.len() == 1 {
                let column: Option<String> = indexed[0].try_get("", "name")?;
                if column.as_deref() == Some(M::uuid_column().as_str()) {
                    return Ok(());
                }
            }
        }
        Err(DbErr::Custom(format!(
            "{table}: source UUID needs a single-column non-partial unique index"
        )))
    }
    /// Read a source value using the same mapping used by projection.
    pub async fn encode_on<C: ConnectionTrait>(
        db: &C,
        id: Uuid,
    ) -> Result<Option<M::Payload>, DbErr> {
        if id.is_nil() {
            return Err(DbErr::Custom("nil source UUID".into()));
        }
        let row = Self::row_on(db, id).await?;
        let Some(row) = row else {
            return Ok(None);
        };
        let mut payload = M::encode(&row)?;
        M::hydrate_on(db, &row, &mut payload).await?;
        M::validate(id, Some(&payload))?;
        Ok(Some(payload))
    }
    /// Read a bounded keyset page and assign missing source UUIDs in the same transaction.
    /// The caller persists the returned cursor only when its authoring work commits.
    pub async fn scan_on(
        tx: &sea_orm::DatabaseTransaction,
        after: Option<i64>,
        limits: ScanLimits,
    ) -> Result<ScanPage<M::Payload>, DbErr>
    where
        M::Payload: serde::Serialize,
    {
        Self::scan_with_on(tx, after, limits, |payload| {
            u64::try_from(
                serde_json::to_vec(payload)
                    .map_err(|e| DbErr::Custom(e.to_string()))?
                    .len(),
            )
            .map_err(|e| DbErr::Custom(e.to_string()))
        })
        .await
    }
    /// Scan using the encoded size of the enclosing registry payload.
    pub async fn scan_with_on<F>(
        tx: &sea_orm::DatabaseTransaction,
        after: Option<i64>,
        limits: ScanLimits,
        measure: F,
    ) -> Result<ScanPage<M::Payload>, DbErr>
    where
        F: Fn(&M::Payload) -> Result<u64, DbErr>,
    {
        use sea_orm::{Iterable, PrimaryKeyToColumn};
        if limits.max_records == 0
            || limits.max_records > 1024
            || limits.max_rows == 0
            || limits.max_rows > 1024
            || limits.max_bytes == 0
        {
            return Err(DbErr::Custom("invalid scan limits".into()));
        }
        let mut keys = <E::PrimaryKey as Iterable>::iter();
        let primary = keys
            .next()
            .ok_or_else(|| DbErr::Custom("missing primary key".into()))?
            .into_column();
        if keys.next().is_some() {
            return Err(DbErr::Custom("composite primary key is unsupported".into()));
        }
        let capacity =
            (u64::from(limits.max_rows) / M::read_ceiling()).min(u64::from(limits.max_records));
        if capacity == 0 {
            return Err(DbErr::Custom(
                "scan read budget cannot cover one complete source value".into(),
            ));
        }
        let mut query = Self::select().order_by_asc(primary).limit(capacity);
        if let Some(cursor) = after {
            query = query.filter(primary.gt(cursor));
        }
        let rows = tx
            .query_all_raw(query.build(tx.get_database_backend()))
            .await?;
        let fetched = rows.len();
        let mut rows_read = fetched as u64;
        let mut records = Vec::new();
        let mut bytes = 0_u64;
        let mut mutations = 0_u64;
        for row in rows {
            let row = RowView::<E>::new(row);
            let position = row.read::<i64>(primary)?;
            let source = row.read::<Option<String>>(M::uuid_column())?;
            let mut payload = M::encode(&row)?;
            rows_read += M::hydrate_on(tx, &row, &mut payload).await?;
            let missing = source.as_deref().is_none_or(str::is_empty);
            let id = if missing {
                M::assign_identity(&payload)?
            } else {
                Uuid::parse_str(source.as_deref().unwrap())
                    .map_err(|e| DbErr::Custom(format!("invalid source UUID at {position}: {e}")))?
            };
            if id.is_nil() {
                return Err(DbErr::Custom(format!("nil source UUID at {position}")));
            }
            M::validate(id, Some(&payload))?;
            let mut updates = Vec::new();
            let canonical = id.to_string();
            if source.as_deref() != Some(canonical.as_str()) {
                updates.push((M::uuid_column(), canonical.into()));
            }
            for ((column, is_uuid), value) in M::columns()
                .into_iter()
                .zip(M::uuid_fields())
                .zip(M::decode(&payload)?)
            {
                if is_uuid
                    && column_value(
                        &row.row,
                        {
                            use sea_orm::IdenStatic;
                            column.as_str()
                        },
                        Option::<String>::None.into(),
                    )? != value
                {
                    updates.push((column, value));
                }
            }
            let size = measure(&payload)?;
            if size > limits.max_bytes.saturating_sub(bytes)
                || (!updates.is_empty() && mutations == limits.max_mutations)
            {
                if records.is_empty() {
                    return Err(DbErr::Custom("scan budget too small for source row".into()));
                }
                break;
            }
            if !updates.is_empty() {
                let source_filter = match &source {
                    Some(text) => M::uuid_column().eq(text.clone()),
                    None => M::uuid_column().is_null(),
                };
                let mut update = E::update_many()
                    .filter(primary.eq(position))
                    .filter(source_filter);
                for (column, value) in updates {
                    update = update.col_expr(column, Expr::value(value));
                }
                let result = update.exec(tx).await?;
                if result.rows_affected != 1 {
                    return Err(DbErr::Custom(
                        "source UUID assignment changed concurrently".into(),
                    ));
                }
                mutations += 1;
            }
            bytes += size;
            records.push(ScanRecord {
                position,
                id,
                payload,
            });
        }
        let next = records.last().map(|record| record.position).or(after);
        Ok(ScanPage {
            done: records.len() == fetched && fetched < capacity as usize,
            records,
            next,
            rows_read,
            bytes,
            mutations,
        })
    }
    /// Apply an already selected value using the caller's transaction.
    pub async fn project_on(
        db: &sea_orm::DatabaseTransaction,
        id: Uuid,
        value: Option<&M::Payload>,
    ) -> Result<(), DbErr> {
        use sea_orm::{IdenStatic, Iterable, PrimaryKeyToColumn};
        M::validate(id, value)?;
        if id.is_nil() {
            return Err(DbErr::Custom("nil source UUID".into()));
        }
        let existing = Self::row_on(db, id).await?;
        let Some(value) = value else {
            if let Some(row) = &existing {
                M::project_children_on(db, row, None).await?;
                E::delete_many()
                    .filter(M::source_filter())
                    .filter(M::uuid_column().eq(id.to_string()))
                    .exec(db)
                    .await?;
            }
            return Ok(());
        };
        let mut columns = M::columns();
        let values = M::decode(value)?;
        if columns.len() != values.len() {
            return Err(DbErr::Custom("field count mismatch".into()));
        }
        let mut active = <E::ActiveModel as ActiveModelTrait>::default();
        if existing.is_none() {
            for (column, value) in M::local_defaults() {
                active.try_set(column, value)?;
            }
        }
        for (column, value) in columns.iter().copied().zip(values) {
            active.try_set(column, value)?;
        }
        let mut protected: std::collections::HashSet<_> =
            columns.iter().map(|c| c.as_str()).collect();
        protected.insert(M::uuid_column().as_str());
        for key in <E::PrimaryKey as Iterable>::iter() {
            protected.insert(key.into_column().as_str());
        }
        for (column, value) in M::local_updates(existing.as_ref(), value)? {
            if !protected.insert(column.as_str()) {
                return Err(DbErr::Custom(format!(
                    "local update overlaps replicated field or identity: {}",
                    column.as_str()
                )));
            }
            active.try_set(column, value)?;
            columns.push(column);
        }
        if existing.is_some() {
            let mut update = E::update_many()
                .filter(M::source_filter())
                .filter(M::uuid_column().eq(id.to_string()));
            for column in columns {
                update = update.col_expr(column, Expr::value(active.get(column).unwrap()));
            }
            update.exec(db).await?;
        } else {
            active.try_set(M::uuid_column(), id.to_string().into())?;
            E::insert(active).exec(db).await?;
        }
        let row = Self::row_on(db, id)
            .await?
            .ok_or_else(|| DbErr::Custom("source row missing after projection".into()))?;
        M::project_children_on(db, &row, Some(value)).await?;
        Ok(())
    }
}

/// Declare a typed payload and its field-to-column mapping in one place.
/// The validator receives the source identity and optional payload, including
/// removals. It must be pure and must not consult database state.
#[macro_export]
macro_rules! row_mapping {
    (
        $(#[$meta:meta])* $vis:vis struct $payload:ident, $mapping:ident for $entity:ty {
            uuid: $uuid:expr,
            fields: { $($field:ident : $ty:ty => $column:expr),* $(,)? },
            defaults: { $($default_column:expr => $default_value:expr),* $(,)? },
            $(source_filter: $source_filter:expr,)?
            $(local_reads: [$($local_read:expr),* $(,)?],)?
            $(lists: { $($list:ident : $item:ty => { entity: $list_entity:ty, mapping: $list_mapping:ty, owner: $list_owner:expr $(,)? }),* $(,)? },)?
            $(effects: { $($effect:ident => { entity: $effect_entity:ty, mapping: $effect_mapping:ty, owner: $effect_owner:expr, derive: $derive:expr $(,)? }),* $(,)? },)?
            $(local_updates: $local_updates:expr,)?
            $(references: { $($reference:ident => $target:literal),* $(,)? },)?
            $(limits: { $($bounded:ident => ($min:expr, $max:expr)),* $(,)? },)?
            $(owner: { field: $owner:ident, allow_removal: $allow_removal:expr },)?
            $(identity: $identity:expr,)? $(validate: $validate:expr,)? $(revision: $revision:expr,)?
        }
    ) => {
        $(#[$meta])*
        #[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
        #[serde(deny_unknown_fields)]
        $vis struct $payload { $(pub $field: $ty,)* $($(pub $list: Vec<$item>,)*)? }
        $vis struct $mapping;
        #[async_trait::async_trait]
        impl $crate::row_policy::RowMapping<$entity> for $mapping {
            type Payload = $payload;
            fn uuid_column() -> <$entity as sea_orm::EntityTrait>::Column { $uuid }
            fn columns() -> Vec<<$entity as sea_orm::EntityTrait>::Column> { vec![$($column),*] }
            fn encode(model: &$crate::row_policy::RowView<$entity>) -> Result<Self::Payload, sea_orm::DbErr> {
                Ok($payload { $($field: model.read::<$ty>($column).map_err(|e| sea_orm::DbErr::Custom(format!("{}: {}", stringify!($field), e)))?,)* $($($list: Vec::new(),)*)? })
            }
            fn field_samples() -> Vec<sea_orm::Value> { vec![$(<$ty as $crate::row_policy::FieldCodec>::sample()),*] }
            fn uuid_fields() -> Vec<bool> { vec![$(<$ty as $crate::row_policy::FieldCodec>::is_uuid()),*] }
            fn decode(value: &Self::Payload) -> Result<Vec<sea_orm::Value>, sea_orm::DbErr> {
                Ok(vec![$(<$ty as $crate::row_policy::FieldCodec>::to_column(&value.$field)),*])
            }
            fn validate(id: uuid::Uuid, value: Option<&Self::Payload>) -> Result<(), sea_orm::DbErr> { if let Some(value) = value {
                $($( $crate::owned_list::OwnedListPolicy::<$list_entity, $list_mapping>::validate_items(&value.$list)?; )*)? $(<$ty as $crate::row_policy::FieldCodec>::validate(&value.$field).map_err(|e| sea_orm::DbErr::Custom(format!("{}: {}", stringify!($field), e)))?;)* $( $( $crate::row_policy::require_reference(&value.$reference); )* )? $( $( $crate::row_policy::BoundedField::check_bounds(&value.$bounded, $min, $max).map_err(|e| sea_orm::DbErr::Custom(format!("{}: {}", stringify!($bounded), e)))?; )* )? } $(if value.is_none() { return Err(sea_orm::DbErr::Custom("owner-protected removal is forbidden".into())); } if let Some(value) = value { if value.$owner.is_nil() { return Err(sea_orm::DbErr::Custom("nil owner".into())); } })? $(if let Some(value) = value { let expected: uuid::Uuid = ($identity)(value)?; if expected.is_nil() || id != expected { return Err(sea_orm::DbErr::Custom("derived identity mismatch".into())); } })? $(let result: Result<(), sea_orm::DbErr> = ($validate)(id, value); result?;)? let _ = (id, value); Ok(()) }
            $(fn assign_identity(value: &Self::Payload) -> Result<uuid::Uuid, sea_orm::DbErr> { ($identity)(value) })?
            $(fn authoring_owner(value: &Self::Payload) -> Option<uuid::Uuid> { Some(value.$owner) })?
            fn validate_declaration() -> Result<(), sea_orm::DbErr> {
                $($( $crate::owned_list::OwnedListPolicy::<$list_entity, $list_mapping>::validate_mapping()?; )*)?
                $($( $crate::derived_effect::DerivedEffect::<$effect_entity, $effect_mapping>::validate_mapping()?; )*)? $(if $allow_removal { return Err(sea_orm::DbErr::Custom("owner-protected removal cannot be authorized from an absent value".into())); })? $( $(if $min > $max { return Err(sea_orm::DbErr::Custom(format!("invalid bounds for {}", stringify!($bounded))));})* )? Ok(()) }
            fn reference_tags() -> Vec<&'static str> {
                #[allow(unused_mut)]
                let mut tags = vec![$($( $target ),*)?];
                $($( tags.extend(<$list_mapping as $crate::owned_list::OwnedListMapping<$list_entity>>::reference_tags()); )*)?
                tags
            }
            fn references(value: &Self::Payload) -> Vec<(&'static str, uuid::Uuid)> {
                #[allow(unused_mut)]
                let mut references = Vec::new();
                $($( if let Some(id) = $crate::row_policy::ReferenceField::reference(&value.$reference) { references.push(($target, id)); } )*)?
                $($( for item in &value.$list { references.extend(<$list_mapping as $crate::owned_list::OwnedListMapping<$list_entity>>::references(item)); } )*)?
                let _ = value;
                references
            }
            fn validate_revision(id: uuid::Uuid, value: Option<&Self::Payload>, revision: &sync::protocol::Revision) -> Result<(), sea_orm::DbErr> {
                $(if value.is_none() { return Err(sea_orm::DbErr::Custom("owner-protected removal is forbidden".into())); }
                if let Some(value) = value { if revision.actor != value.$owner { return Err(sea_orm::DbErr::Custom("unauthorized author".into())); } })?
                $(let result: Result<(), sea_orm::DbErr> = ($revision)(id, value, revision); result?;)?
                let _ = (id, value, revision);
                Ok(())
            }

            fn local_read_columns() -> Vec<<$entity as sea_orm::EntityTrait>::Column> { vec![$($($local_read),*)?] }
            $(fn source_filter() -> sea_orm::Condition { $source_filter })?
            fn read_ceiling() -> u64 {
                #[allow(unused_mut)]
                let mut reads = 1;
                $($( reads += $crate::owned_list::OwnedListPolicy::<$list_entity, $list_mapping>::read_ceiling(); )*)?
                reads
            }
            fn projection_ceiling() -> $crate::owned_list::ListBudget {
                #[allow(unused_mut)]
                let mut budget = $crate::owned_list::ListBudget { max_reads: 2, max_mutations: 1 };
                $($(
                    budget.max_reads += $crate::owned_list::OwnedListPolicy::<$list_entity, $list_mapping>::read_ceiling();
                    budget.max_mutations += u64::from(<$list_mapping as $crate::owned_list::OwnedListMapping<$list_entity>>::max_items()) * 3;
                )*)?
                $($(
                    budget.max_reads += u64::from(<$effect_mapping as $crate::derived_effect::EffectMapping<$effect_entity>>::max_items()) + 1;
                    budget.max_mutations += u64::from(<$effect_mapping as $crate::derived_effect::EffectMapping<$effect_entity>>::max_items()) * 2;
                )*)?
                budget
            }
            async fn hydrate_on<C: sea_orm::ConnectionTrait>(
                db: &C, row: &$crate::row_policy::RowView<$entity>, value: &mut Self::Payload,
            ) -> Result<u64, sea_orm::DbErr> {
                #[allow(unused_mut)]
                let mut reads = 0;
                $($(
                    let owner: Vec<sea_orm::Value> = ($list_owner)(row)?;
                    value.$list = $crate::owned_list::OwnedListPolicy::<$list_entity, $list_mapping>::read_on(
                        db, &owner, $crate::owned_list::OwnedListPolicy::<$list_entity, $list_mapping>::read_ceiling(),
                    ).await?;
                    reads += value.$list.len() as u64;
                )*)?
                let _ = (db, row, value);
                Ok(reads)
            }
            async fn project_children_on(
                tx: &sea_orm::DatabaseTransaction, row: &$crate::row_policy::RowView<$entity>, value: Option<&Self::Payload>,
            ) -> Result<(), sea_orm::DbErr> {
                $($(
                    let owner: Vec<sea_orm::Value> = ($list_owner)(row)?;
                    let budget = $crate::owned_list::ListBudget {
                        max_reads: $crate::owned_list::OwnedListPolicy::<$list_entity, $list_mapping>::read_ceiling(),
                        max_mutations: u64::from(<$list_mapping as $crate::owned_list::OwnedListMapping<$list_entity>>::max_items()) * 3,
                    };
                    $crate::owned_list::OwnedListPolicy::<$list_entity, $list_mapping>::reconcile_on(
                        tx, &owner, value.map_or(&[], |value| value.$list.as_slice()), budget,
                    ).await?;
                )*)?
                $($(
                    let owner: Vec<sea_orm::Value> = ($effect_owner)(row)?;
                    let maximum = u64::from(<$effect_mapping as $crate::derived_effect::EffectMapping<$effect_entity>>::max_items());
                    $crate::derived_effect::DerivedEffect::<$effect_entity, $effect_mapping>::prepare_on(
                        tx, &owner, row, &value, |row, value| ($derive)(row, *value),
                        $crate::owned_list::ListBudget { max_reads: maximum + 1, max_mutations: maximum * 2 },
                    ).await?.apply_on(tx).await?;
                )*)?
                let _ = (tx, row, value);
                Ok(())
            }
            $(fn local_updates(before: Option<&$crate::row_policy::RowView<$entity>>, value: &Self::Payload) -> Result<Vec<(<$entity as sea_orm::EntityTrait>::Column, sea_orm::Value)>, sea_orm::DbErr> {
                ($local_updates)(before, value)
            })?
            fn local_defaults() -> Vec<(<$entity as sea_orm::EntityTrait>::Column, sea_orm::Value)> {
                vec![$(($default_column, ($default_value).into())),*]
            }
        }
    };
}
