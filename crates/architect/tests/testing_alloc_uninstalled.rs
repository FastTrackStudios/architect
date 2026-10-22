//! The counting allocator is not a library dependency: linking architect
//! with `testing-alloc` installs nothing. This binary has no
//! `#[global_allocator]`, so the counters must stay at zero however much
//! it allocates.

use architect::testing::alloc;

#[test]
fn the_library_does_not_install_the_counting_allocator() {
    let block = std::hint::black_box(vec![1u8; 8 << 20]);
    assert_eq!(alloc::live(), 0);
    assert_eq!(alloc::peak(), 0);
    drop(block);
}
