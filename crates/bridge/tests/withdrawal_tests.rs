#![allow(clippy::unwrap_used)]
#![cfg(feature = "bazel_build")]
//! Bridge withdrawal kernel tests exercised via Roswell kernel.

use bridge_roswell_harness::run_roswell_test;

#[tokio::test]
async fn test_withdrawal_suite() {
    run_roswell_test("test-withdrawal")
        .await
        .expect("roswell withdrawal tests failed");
}
