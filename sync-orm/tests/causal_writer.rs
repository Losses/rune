use sync_orm::causal_writer::WriterIncarnation;
use uuid::Uuid;
#[test]
fn clones_share_allocation_and_reopened_handles_do_not_reuse_dots() {
    let actor = Uuid::new_v4();
    let first = WriterIncarnation::new(actor).unwrap();
    let clone = first.clone();
    let another = WriterIncarnation::new(actor).unwrap();
    assert_ne!(first.id(), another.id());
    assert_eq!(first.author(), actor);
    let abandoned = first.allocate().unwrap();
    let next = clone.allocate().unwrap();
    assert_eq!(abandoned.writer_incarnation, next.writer_incarnation);
    assert_eq!((abandoned.counter, next.counter), (1, 2));
    assert_ne!(
        another.allocate().unwrap().writer_incarnation,
        next.writer_incarnation
    );
    assert!(WriterIncarnation::new(Uuid::nil()).is_err());
}
