use compact_std::prelude::*;

const MAX_WORKER: u32 = 65_535;

#[compact]
struct Record {
    #[max = 0]
    constant: u8,
    first: bool,
    second: bool,
    #[max = MAX_WORKER]
    worker: u32,
    #[hot]
    key: u16,
    #[cold]
    archived: bool,
}

#[compact]
enum Phase {
    Created,
    Scheduled,
    Running,
    Complete,
    Failed,
}

fn main() {
    assert_eq!(RecordCompact::CONSTANT_BIT_OFFSET, 0);
    assert_eq!(RecordCompact::FIRST_BIT_OFFSET, 0);
    assert_eq!(RecordCompact::SECOND_BIT_OFFSET, 1);
    assert_eq!(RecordCompact::WORKER_BIT_OFFSET, 2);
    assert_eq!(RecordCompact::MAIN_STORAGE_BYTES, 3);
    assert_eq!(RecordCompact::HOT_STORAGE_BYTES, 2);
    assert_eq!(RecordCompact::COLD_STORAGE_BYTES, 1);
    assert_eq!(PhaseCompact::DISCRIMINANT_BITS, 3);
    assert_eq!(PhaseCompact::STORAGE_BYTES, 1);

    StdArena::with_capacity(256, |arena| {
        let logical = Record {
            constant: 0,
            first: true,
            second: false,
            worker: MAX_WORKER,
            key: 12,
            archived: true,
        };
        let compact = logical.compact_in(arena).unwrap();
        assert_eq!(compact.constant(arena).unwrap(), 0);
        assert_eq!(compact.worker(arena).unwrap(), MAX_WORKER);
        assert!(compact.first(arena).unwrap());
        assert!(!compact.second(arena).unwrap());
        assert!(compact.archived(arena).unwrap());
        compact.set_first(false, arena).unwrap();
        compact.set_second(true, arena).unwrap();
        assert!(!compact.first(arena).unwrap());
        assert!(compact.second(arena).unwrap());
        assert!(compact.set_worker(MAX_WORKER + 1, arena).is_err());

        let phase = Phase::Failed.compact_in(arena).unwrap();
        assert!(matches!(phase.get(arena).unwrap(), Phase::Failed));
    })
    .unwrap();
}
