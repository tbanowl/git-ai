#![cfg(windows)]

use crate::lines;
use crate::repos::test_repo::TestRepo;
use std::sync::{Arc, Barrier};
use std::thread;

const CONCURRENT_CALLERS: usize = 24;
const CALLS_PER_THREAD: usize = 40;

#[test]
#[ignore = "stress test for Windows concurrent spawn hardening"]
fn stress_concurrent_rev_parse_git_dir_spawns() {
    let repo = Arc::new(TestRepo::new());
    let mut file = repo.filename("seed.txt");
    file.set_contents(lines!["seed"]).stage();
    repo.stage_all_and_commit("seed repo for concurrent rev-parse stress")
        .expect("seed commit should succeed");

    let start_barrier = Arc::new(Barrier::new(CONCURRENT_CALLERS));
    let handles = (0..CONCURRENT_CALLERS)
        .map(|_| {
            let repo = Arc::clone(&repo);
            let start_barrier = Arc::clone(&start_barrier);
            thread::spawn(move || {
                start_barrier.wait();

                for _ in 0..CALLS_PER_THREAD {
                    let git_dir = repo
                        .git(&["rev-parse", "--git-dir"])
                        .expect("rev-parse --git-dir should succeed under concurrency");
                    assert_eq!(git_dir.trim(), ".git");
                }
            })
        })
        .collect::<Vec<_>>();

    for handle in handles {
        handle
            .join()
            .expect("concurrent rev-parse worker should complete successfully");
    }
}
