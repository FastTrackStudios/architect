//! `architect::testing::alloc::Counting`, installed by this test binary:
//! the counters follow what the program allocates. One test function, so
//! no sibling test's allocations land in the counters mid-assertion.

#![allow(clippy::unwrap_used, clippy::arithmetic_side_effects)]

use architect::testing::alloc::{self, Counting};

#[global_allocator]
static ALLOC: Counting = Counting;

const BLOCK: usize = 8 << 20;
/// Headroom for the harness's own threads: at least 7 of 8 MiB must show.
const SLACK: usize = 1 << 20;

#[test]
fn live_and_peak_follow_allocation_realloc_and_free() {
    alloc::reset_peak();
    let before = alloc::live();

    let block = std::hint::black_box(vec![1u8; BLOCK]);
    assert!(
        alloc::live() >= before + BLOCK - SLACK,
        "live {}",
        alloc::mib(alloc::live())
    );
    drop(block);
    assert!(
        alloc::peak() >= before + BLOCK - SLACK,
        "peak {}",
        alloc::mib(alloc::peak())
    );
    assert!(
        alloc::live() < before + BLOCK - SLACK,
        "the block's bytes were not given back"
    );

    // A new high-water mark starts from what is live now.
    alloc::reset_peak();
    assert!(alloc::peak() < before + BLOCK - SLACK);

    // realloc counts both ways.
    let mut v: Vec<u8> = Vec::with_capacity(1 << 10);
    v.reserve_exact(BLOCK);
    assert!(alloc::live() >= before + BLOCK - SLACK);
    v.shrink_to(1 << 10);
    assert!(alloc::live() < before + BLOCK - SLACK);
    drop(std::hint::black_box(v));
}
