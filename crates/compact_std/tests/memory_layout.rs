use compact_std::{
    CageAllocation, CageConfig, CompactBox, CompactVec, CompactVecDeque, FrozenBytes, FrozenMap,
    FrozenOsString, FrozenPathBuf, FrozenSet, FrozenString, FrozenVec,
};
use core::mem::{align_of, size_of};

#[test]
fn configuration_and_owner_layouts_match_the_v2_4_baseline() {
    assert_eq!(size_of::<CageConfig>(), size_of::<usize>());
    assert_eq!(align_of::<CageConfig>(), align_of::<usize>());

    assert_eq!(size_of::<CageAllocation<u64>>(), 4);
    assert_eq!(align_of::<CageAllocation<u64>>(), 4);
    assert_eq!(size_of::<Option<CageAllocation<u64>>>(), 4);
    assert_eq!(align_of::<Option<CageAllocation<u64>>>(), 4);

    assert_eq!(size_of::<CompactBox<u64>>(), 4);
    assert_eq!(align_of::<CompactBox<u64>>(), 4);
    assert_eq!(size_of::<CompactVec<u64>>(), 4);
    assert_eq!(align_of::<CompactVec<u64>>(), 4);
    assert_eq!(size_of::<CompactVecDeque<u64>>(), 12);
    assert_eq!(align_of::<CompactVecDeque<u64>>(), 4);
}

#[test]
fn frozen_descriptors_match_the_v2_4_baseline() {
    for (size, alignment) in [
        (size_of::<FrozenString>(), align_of::<FrozenString>()),
        (size_of::<FrozenBytes>(), align_of::<FrozenBytes>()),
        (size_of::<FrozenOsString>(), align_of::<FrozenOsString>()),
        (size_of::<FrozenPathBuf>(), align_of::<FrozenPathBuf>()),
        (size_of::<FrozenVec<u32>>(), align_of::<FrozenVec<u32>>()),
        (
            size_of::<FrozenMap<u32, u64>>(),
            align_of::<FrozenMap<u32, u64>>(),
        ),
        (size_of::<FrozenSet<u32>>(), align_of::<FrozenSet<u32>>()),
    ] {
        assert_eq!(size, 8);
        assert_eq!(alignment, 4);
    }
}
