//! Bounded replacement of registered derived rows, such as search documents.
//! Callers hold the namespace writer lock from preparation through commit.
use crate::owned_list::{ListBudget, ListWork};
use sea_orm::sea_query::Expr;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, Condition, DbErr, EntityTrait, IdenStatic, QueryFilter,
    QuerySelect, Value,
};
use std::{collections::BTreeSet, marker::PhantomData};

/// Declares the complete source columns and full owner predicate for an effect.
pub trait EffectMapping<E: EntityTrait> {
    type Item;
    fn max_items() -> u32;
    fn owners() -> Vec<E::Column>;
    fn constants() -> Vec<(E::Column, Value)>;
    fn fields() -> Vec<E::Column>;
    fn defaults() -> Vec<(E::Column, Value)>;
    fn validate(item: &Self::Item) -> Result<(), DbErr>;
    fn values(item: &Self::Item) -> Vec<Value>;
}
pub struct EffectPlan<E: EntityTrait> {
    predicate: Condition,
    old_count: u64,
    rows: Vec<E::ActiveModel>,
    pub work: ListWork,
}
impl<E: EntityTrait> EffectPlan<E> {
    pub async fn apply_on(self, tx: &sea_orm::DatabaseTransaction) -> Result<ListWork, DbErr> {
        if self.old_count != 0 {
            let result = E::delete_many().filter(self.predicate).exec(tx).await?;
            if result.rows_affected != self.old_count {
                return Err(DbErr::Custom("derived rows changed after planning".into()));
            }
        }
        for row in self.rows {
            E::insert(row).exec_without_returning(tx).await?;
        }
        Ok(self.work)
    }
}
pub struct DerivedEffect<E: EntityTrait, M: EffectMapping<E>>(PhantomData<(E, M)>);
impl<E: EntityTrait, M: EffectMapping<E>> DerivedEffect<E, M> {
    pub fn validate_mapping() -> Result<(), DbErr> {
        if M::max_items() == 0 || M::max_items() > 65_536 || M::owners().is_empty() {
            return Err(DbErr::Custom(
                "invalid derived effect bounds or owner".into(),
            ));
        }
        let mut columns = BTreeSet::new();
        for col in M::owners()
            .into_iter()
            .chain(M::constants().into_iter().map(|(c, _)| c))
            .chain(M::fields())
            .chain(M::defaults().into_iter().map(|(c, _)| c))
        {
            if !columns.insert(col.as_str()) {
                return Err(DbErr::Custom(format!(
                    "overlapping derived column {}",
                    col.as_str()
                )));
            }
        }
        Ok(())
    }
    /// The pure function sees source values only; it cannot issue SQL or author operations.
    pub async fn prepare_on<B, A, F>(
        tx: &sea_orm::DatabaseTransaction,
        owner: &[Value],
        before: &B,
        after: &A,
        derive: F,
        budget: ListBudget,
    ) -> Result<EffectPlan<E>, DbErr>
    where
        F: FnOnce(&B, &A) -> Result<Vec<M::Item>, DbErr>,
    {
        Self::validate_mapping()?;
        let items = derive(before, after)?;
        if items.len() > M::max_items() as usize {
            return Err(DbErr::Custom(
                "derived effect exceeds declared bound".into(),
            ));
        }
        if budget.max_reads < u64::from(M::max_items()) + 1 {
            return Err(DbErr::Custom(
                "derived effect read budget cannot cover bound".into(),
            ));
        }
        let columns = M::owners();
        if columns.len() != owner.len() {
            return Err(DbErr::Custom("derived owner value count mismatch".into()));
        }
        let mut predicate = Condition::all();
        let mut owner_model = <E::ActiveModel as ActiveModelTrait>::default();
        for (column, value) in columns
            .into_iter()
            .zip(owner.iter().cloned())
            .chain(M::constants())
        {
            owner_model.try_set(column, value.clone())?;
            predicate = predicate.add(column.eq(value));
        }
        let existing = E::find()
            .select_only()
            .expr(Expr::val(1))
            .filter(predicate.clone())
            .limit(u64::from(M::max_items()) + 1)
            .into_tuple::<i32>()
            .all(tx)
            .await?;
        if existing.len() > M::max_items() as usize {
            return Err(DbErr::Custom(
                "existing derived rows exceed declared bound".into(),
            ));
        }
        let mutations = (existing.len() + items.len()) as u64;
        if mutations > budget.max_mutations {
            return Err(DbErr::Custom(
                "derived effect mutation budget exceeded".into(),
            ));
        }
        let mut rows = Vec::with_capacity(items.len());
        for item in items {
            M::validate(&item)?;
            let fields = M::fields();
            let values = M::values(&item);
            if fields.len() != values.len() {
                return Err(DbErr::Custom("derived effect field count mismatch".into()));
            }
            let mut model = <E::ActiveModel as ActiveModelTrait>::default();
            for (column, value) in M::owners()
                .into_iter()
                .zip(owner.iter().cloned())
                .chain(M::constants())
                .chain(fields.into_iter().zip(values))
                .chain(M::defaults())
            {
                model.try_set(column, value)?;
            }
            rows.push(model);
        }
        let old_count = existing.len() as u64;
        Ok(EffectPlan {
            predicate,
            old_count,
            rows,
            work: ListWork {
                rows_read: old_count,
                mutations,
            },
        })
    }
}

/// Declare bounded derived row fields and owner predicate once.
#[macro_export]
macro_rules! derived_effect_mapping {
    (
        $vis:vis struct $item:ident, $mapping:ident for $entity:ty {
            max_items: $max:expr,
            owners: [$($owner:expr),+ $(,)?],
            constants: { $($constant:expr => $constant_value:expr),* $(,)? },
            fields: { $($field:ident : $ty:ty => $column:expr),* $(,)? },
            defaults: { $($default:expr => $default_value:expr),* $(,)? },
            $(validate: $validate:expr,)?
        }
    ) => {
        #[derive(Clone, Debug, PartialEq, Eq)]
        $vis struct $item { $(pub $field: $ty),* }
        $vis struct $mapping;
        impl $crate::derived_effect::EffectMapping<$entity> for $mapping {
            type Item = $item;
            fn max_items() -> u32 { $max }
            fn owners() -> Vec<<$entity as sea_orm::EntityTrait>::Column> { vec![$($owner),+] }
            fn constants() -> Vec<(<$entity as sea_orm::EntityTrait>::Column, sea_orm::Value)> { vec![$(($constant, ($constant_value).into())),*] }
            fn fields() -> Vec<<$entity as sea_orm::EntityTrait>::Column> { vec![$($column),*] }
            fn defaults() -> Vec<(<$entity as sea_orm::EntityTrait>::Column, sea_orm::Value)> { vec![$(($default, ($default_value).into())),*] }
            fn validate(item: &Self::Item) -> Result<(), sea_orm::DbErr> {
                $(<$ty as $crate::row_policy::FieldCodec>::validate(&item.$field)?;)*
                $(let result: Result<(), sea_orm::DbErr> = ($validate)(item); result?;)?
                Ok(())
            }
            fn values(item: &Self::Item) -> Vec<sea_orm::Value> { vec![$(<$ty as $crate::row_policy::FieldCodec>::to_column(&item.$field)),*] }
        }
    };
}
