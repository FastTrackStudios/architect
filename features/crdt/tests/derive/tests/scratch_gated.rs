//! Scratch: a test only a feature turns on, which no pass runs.
#![cfg(feature = "scratch")]

#[test]
fn gated() {
    assert_eq!(1 + 1, 2);
}
