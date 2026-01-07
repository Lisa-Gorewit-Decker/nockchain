use std::fs;
use std::path::Path;

use tempfile::TempDir;

use super::NockApp;
use crate::kernel::form::Kernel;

pub async fn setup_nockapp_with_interval(
    jam: &str,
    save_interval: Option<std::time::Duration>,
) -> (TempDir, NockApp) {
    let temp_dir = TempDir::new().expect("Failed to create temp directory");
    let temp_dir_path = temp_dir.path().to_path_buf();
    // Try multiple possible locations for the jam file
    let possible_paths = [
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("test-jams")
            .join(jam),
        Path::new("open/crates/nockapp/test-jams").join(jam),
        // Add other potential paths
    ];

    let jam_bytes = possible_paths
        .iter()
        .find_map(|path| fs::read(path).ok())
        .unwrap_or_else(|| panic!("Failed to read {} file from any known location", jam));

    let kernel_f =
        async |checkpoint| {
            Kernel::load(&jam_bytes, checkpoint, vec![], Default::default(), None).await
        };
    (
        temp_dir,
        NockApp::new(kernel_f, &temp_dir_path, save_interval)
            .await
            .expect("Could not create NockApp"),
    )
}

pub async fn setup_nockapp(jam: &str) -> (TempDir, NockApp) {
    setup_nockapp_with_interval(jam, Some(std::time::Duration::from_secs(1))).await
}

#[cfg(test)]
pub mod tests {
    use std::sync::atomic::Ordering;
    use std::time::{Duration, Instant};

    use bytes::Bytes;
    use nockvm::jets::util::slot;
    use nockvm::mem::NockStack;
    use nockvm::noun::{CellHandle, Noun, NounAllocator, NounSpace, D, T};
    use nockvm::serialization::{cue, jam};
    use nockvm::unifying_equality::unifying_equality;
    use nockvm_macros::tas;
    use tracing::info;
    use tracing_test::traced_test;

    use super::{setup_nockapp, setup_nockapp_with_interval};
    use crate::nockapp::wire::{SystemWire, Wire};
    use crate::noun::slab::{slab_equality, NockJammer, NounSlab};
    use nockvm::ext::noun_equality;
    use crate::save::{SaveableCheckpoint, Saver};
    use crate::test_support::TestArena;
    use crate::utils::NOCK_STACK_SIZE;
    use crate::{NockApp, NounExt};

    async fn save_nockapp(nockapp: &mut NockApp) {
        nockapp.tasks.close();
        let permit = nockapp.save_mutex.clone().lock_owned().await;
        let _ = nockapp.save(permit).await;
        let _ = nockapp.tasks.wait().await;
        nockapp.tasks.reopen();
    }

    // Panics if checkpoint failed to load, only permissible because this is expressly for testing
    async fn spawn_save_t(nockapp: &mut NockApp, sleep_t: std::time::Duration) {
        let sleepy_time = tokio::time::sleep(sleep_t);
        let permit = nockapp.save_mutex.clone().lock_owned().await;
        let _join_handle = nockapp
            .save_f(sleepy_time, permit)
            .await
            .expect("Failed to spawn nockapp save task");
        // join_handle.await.expect("Failed to save nockapp").expect("Failed to save nockapp 2");
    }

    fn summarize_samples(label: &str, samples: &[Duration]) -> (f64, f64, f64) {
        if samples.is_empty() {
            println!("perf: {}: no samples", label);
            return (0.0, 0.0, 0.0);
        }
        let mut min = samples[0];
        let mut max = samples[0];
        let mut total_us: u128 = 0;
        for sample in samples {
            if *sample < min {
                min = *sample;
            }
            if *sample > max {
                max = *sample;
            }
            total_us += sample.as_micros();
        }
        let count = samples.len() as f64;
        let avg_ms = (total_us as f64) / count / 1000.0;
        let min_ms = (min.as_micros() as f64) / 1000.0;
        let max_ms = (max.as_micros() as f64) / 1000.0;
        println!(
            "perf: {}: n={}, avg_ms={:.3}, min_ms={:.3}, max_ms={:.3}",
            label,
            samples.len(),
            avg_ms,
            min_ms,
            max_ms
        );
        (avg_ms, min_ms, max_ms)
    }

    // Test nockapp save
    // TODO: bump the actual serf event number (can we do a poke to the test kernel?)
    #[test]
    #[traced_test]
    #[cfg_attr(miri, ignore)]
    fn test_nockapp_save_race_condition() {
        let _test_arena = TestArena::default();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap_or_else(|err| {
                panic!(
                    "Panicked with {err:?} at {}:{} (git sha: {:?})",
                    file!(),
                    line!(),
                    option_env!("GIT_SHA")
                )
            });
        let (temp, mut nockapp) = runtime.block_on(setup_nockapp("test-ker.jam"));
        assert_eq!(nockapp.kernel.serf.event_number.load(Ordering::SeqCst), 0);
        // first run
        runtime.block_on(spawn_save_t(&mut nockapp, Duration::from_millis(1000)));
        // second run
        nockapp.kernel.serf.event_number.store(1, Ordering::SeqCst); // we need to set the actual serf event number
        runtime.block_on(spawn_save_t(&mut nockapp, Duration::from_millis(5000)));
        // Simulate what the event handlers would be doing and wait for the task tracker to be done
        nockapp.tasks.close();
        runtime.block_on(nockapp.tasks.wait());
        nockapp.tasks.reopen();
        // Shutdown the runtime immediately
        runtime.shutdown_timeout(std::time::Duration::from_secs(0));

        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("Failed to build runtime");

        let (_, checkpoint_opt) = runtime
            .block_on(Saver::<NockJammer>::try_load(
                &temp.path().to_path_buf(),
                None,
            ))
            .expect("Failed trying to load checkpoint");
        let checkpoint: SaveableCheckpoint = checkpoint_opt.expect("No checkpoint found");
        info!("checkpoint: {:?}", checkpoint);
        assert_eq!(checkpoint.event_num, 1);
    }

    // Test nockapp save
    // TODO: need a way to grab arvo state from the serf. Probably a serf action
    #[tokio::test(flavor = "current_thread")]
    #[traced_test]
    #[cfg_attr(miri, ignore)]
    async fn test_nockapp_save() {
        let _test_arena = TestArena::default();
        // console_subscriber::init();
        let (temp, mut nockapp) = setup_nockapp("test-ker.jam").await;
        let first_checkpoint = nockapp
            .kernel
            .checkpoint()
            .await
            .expect("Couldn't get kernel checkpoint");

        assert_eq!(nockapp.kernel.serf.event_number.load(Ordering::SeqCst), 0);
        // Save
        info!("Saving nockapp");
        save_nockapp(&mut nockapp).await;
        // Permit should be dropped

        // A valid checkpoint should exist in one of the jam files
        let (_, checkpoint_opt) = Saver::<NockJammer>::try_load(&temp.path().to_path_buf(), None)
            .await
            .expect("Could not load checkpoint");
        let checkpoint: SaveableCheckpoint = checkpoint_opt.expect("No checkpoint loaded");

        // Checkpoint event number should be 0
        assert_eq!(checkpoint.event_num, 0);

        info!("Asserting checkpoint and arvo equality");
        // Checkpoint kernel should be equal to the saved kernel
        assert!(slab_equality(&checkpoint.state, &first_checkpoint.state));
        assert!(slab_equality(&checkpoint.cold, &first_checkpoint.cold));
    }

    // Test nockapp poke
    #[tokio::test(flavor = "current_thread")]
    #[traced_test]
    #[cfg_attr(miri, ignore)]
    async fn test_nockapp_poke_save() {
        let _test_arena = TestArena::default();
        let (temp, mut nockapp) = setup_nockapp("test-ker.jam").await;
        assert_eq!(nockapp.kernel.serf.event_number.load(Ordering::SeqCst), 0);
        let state_before_poke = nockapp
            .kernel
            .checkpoint()
            .await
            .expect("Can't get kernel state before poke");

        let poke_noun = D(tas!(b"inc"));
        let poke = {
            let mut slab = NounSlab::new();
            let space = NounSpace::empty();
            slab.copy_into(poke_noun, &space);
            slab
        };

        let wire = SystemWire.to_wire();
        let _ = nockapp.kernel.poke(wire, poke).await.unwrap_or_else(|err| {
            panic!(
                "Panicked with {err:?} at {}:{} (git sha: {:?})",
                file!(),
                line!(),
                option_env!("GIT_SHA")
            )
        });

        // Save
        save_nockapp(&mut nockapp).await;

        // A valid checkpoint should exist in one of the jam files
        let (_, checkpoint_opt) = Saver::<NockJammer>::try_load(&temp.path().to_path_buf(), None)
            .await
            .expect("Failed to load checkpoint");
        let checkpoint: SaveableCheckpoint = checkpoint_opt.expect("No checkpoint");

        // Checkpoint event number should be 1
        assert!(checkpoint.event_num == 1);
        let state_after_poke = nockapp
            .kernel
            .checkpoint()
            .await
            .expect("Failed to get checkpoint after poke");

        assert!(slab_equality(&checkpoint.state, &state_after_poke.state));
        assert!(slab_equality(&checkpoint.cold, &state_after_poke.cold));
        assert!(!slab_equality(&checkpoint.state, &state_before_poke.state));
    }

    #[tokio::test(flavor = "current_thread")]
    #[traced_test]
    #[cfg_attr(miri, ignore)]
    async fn test_nockapp_save_multiple() {
        let _test_arena = TestArena::default();
        let (temp, mut nockapp) = setup_nockapp("test-ker.jam").await;
        assert_eq!(nockapp.kernel.serf.event_number.load(Ordering::SeqCst), 0);
        let mut stack = NockStack::new(NOCK_STACK_SIZE, 0);
        let space = stack.noun_space();

        for i in 1..4 {
            // Poke to increment the state
            let poke_noun = D(tas!(b"inc"));
            let poke = {
                let mut slab = NounSlab::new();
                let space = NounSpace::empty();
                slab.copy_into(poke_noun, &space);
                slab
            };
            let wire = SystemWire.to_wire();
            let _ = nockapp.kernel.poke(wire, poke).await.unwrap_or_else(|err| {
                panic!(
                    "Panicked with {err:?} at {}:{} (git sha: {:?})",
                    file!(),
                    line!(),
                    option_env!("GIT_SHA")
                )
            });

            // Save
            save_nockapp(&mut nockapp).await;

            // A valid checkpoint should exist in one of the jam files
            let (_, checkpoint_opt) =
                Saver::<NockJammer>::try_load(&temp.path().to_path_buf(), None)
                    .await
                    .expect("Failed to load checkpoint");
            let checkpoint: SaveableCheckpoint = checkpoint_opt.expect("No checkpoint found");

            // Checkpoint event number should be i
            assert!(checkpoint.event_num == i);

            // Checkpointed state should have been incremented
            let peek_noun = T(&mut stack, &[D(tas!(b"state")), D(0)]);
            let peek = {
                let mut slab = NounSlab::new();
                slab.copy_into(peek_noun, &space);
                slab
            };

            // res should be [~ ~ [%0 val]]
            let mut res = nockapp.kernel.peek(peek).await.unwrap_or_else(|err| {
                panic!(
                    "Panicked with {err:?} at {}:{} (git sha: {:?})",
                    file!(),
                    line!(),
                    option_env!("GIT_SHA")
                )
            });
            let res_space = res.noun_space();
            res.modify_noun(|r| {
                let cell = slot(r, 7, &res_space)
                    .unwrap_or_else(|err| {
                        panic!(
                            "Panicked with {err:?} at {}:{} (git sha: {:?})",
                            file!(),
                            line!(),
                            option_env!("GIT_SHA")
                        )
                    })
                    .as_cell()
                    .unwrap_or_else(|err| {
                        panic!(
                            "Panicked with {err:?} at {}:{} (git sha: {:?})",
                            file!(),
                            line!(),
                            option_env!("GIT_SHA")
                        )
                    });
                CellHandle::new(cell, &res_space).tail().noun()
            });

            let comp = {
                let mut slab = NounSlab::new();
                let space = NounSpace::empty();
                slab.copy_into(D(i), &space);
                slab
            };

            assert!(
                slab_equality(&res, &comp),
                "res: {:?} != comp: {:?}",
                res,
                comp
            );
        }
    }

    #[tokio::test(flavor = "current_thread")]
    #[traced_test]
    #[cfg_attr(miri, ignore)]
    #[ignore]
    async fn test_poke_peek_perf_workload() {
        std::env::set_var("NOCK_PMA_TIMING", "1");
        std::env::set_var("NOCKAPP_DISABLE_METRICS", "1");

        let _test_arena = TestArena::default();
        let (_temp, nockapp) = setup_nockapp_with_interval("test-ker.jam", None).await;
        let pma_timing = nockapp
            .kernel
            .serf
            .pma_timing
            .clone()
            .expect("NOCK_PMA_TIMING must be set before setup");
        let _ = pma_timing.take_samples();

        let iters: usize = std::env::var("NOCKAPP_PERF_ITERS")
            .ok()
            .and_then(|val| val.parse().ok())
            .unwrap_or(20);
        let warmup: usize = std::env::var("NOCKAPP_PERF_WARMUP")
            .ok()
            .and_then(|val| val.parse().ok())
            .unwrap_or(1);

        let mut stack = NockStack::new(NOCK_STACK_SIZE, 0);
        let space = stack.noun_space();
        let mut poke_wall = Vec::with_capacity(iters);
        let mut peek_wall = Vec::with_capacity(iters);

        for i in 1..=iters {
            let poke_noun = D(tas!(b"inc"));
            let poke = {
                let mut slab = NounSlab::new();
                let space = NounSpace::empty();
                slab.copy_into(poke_noun, &space);
                slab
            };
            let wire = SystemWire.to_wire();
            let poke_start = Instant::now();
            let _ = nockapp.kernel.poke(wire, poke).await.unwrap_or_else(|err| {
                panic!(
                    "Panicked with {err:?} at {}:{} (git sha: {:?})",
                    file!(),
                    line!(),
                    option_env!("GIT_SHA")
                )
            });
            poke_wall.push(poke_start.elapsed());

            let peek_noun = T(&mut stack, &[D(tas!(b"state")), D(0)]);
            let peek = {
                let mut slab = NounSlab::new();
                slab.copy_into(peek_noun, &space);
                slab
            };
            let peek_start = Instant::now();
            let mut res = nockapp.kernel.peek(peek).await.unwrap_or_else(|err| {
                panic!(
                    "Panicked with {err:?} at {}:{} (git sha: {:?})",
                    file!(),
                    line!(),
                    option_env!("GIT_SHA")
                )
            });
            peek_wall.push(peek_start.elapsed());

            let res_space = res.noun_space();
            res.modify_noun(|r| {
                let cell = slot(r, 7, &res_space)
                    .unwrap_or_else(|err| {
                        panic!(
                            "Panicked with {err:?} at {}:{} (git sha: {:?})",
                            file!(),
                            line!(),
                            option_env!("GIT_SHA")
                        )
                    })
                    .as_cell()
                    .unwrap_or_else(|err| {
                        panic!(
                            "Panicked with {err:?} at {}:{} (git sha: {:?})",
                            file!(),
                            line!(),
                            option_env!("GIT_SHA")
                        )
                    });
                CellHandle::new(cell, &res_space).tail().noun()
            });

            let comp = {
                let mut slab = NounSlab::new();
                let space = NounSpace::empty();
                slab.copy_into(D(i as u64), &space);
                slab
            };

            assert!(
                slab_equality(&res, &comp),
                "res: {:?} != comp: {:?}",
                res,
                comp
            );
        }

        let mut pma_samples = pma_timing.take_samples();
        assert_eq!(
            pma_samples.len(),
            iters,
            "expected one PMA timing sample per poke"
        );

        let skip = warmup.min(iters);
        let poke_wall = if poke_wall.len() > skip {
            &poke_wall[skip..]
        } else {
            &[]
        };
        let peek_wall = if peek_wall.len() > skip {
            &peek_wall[skip..]
        } else {
            &[]
        };
        if pma_samples.len() > skip {
            pma_samples = pma_samples.split_off(skip);
        } else {
            pma_samples.clear();
        }

        let event_samples: Vec<Duration> = pma_samples.iter().map(|s| s.event).collect();
        let pma_copy_samples: Vec<Duration> = pma_samples.iter().map(|s| s.pma_copy).collect();
        let total_samples: Vec<Duration> = pma_samples
            .iter()
            .map(|s| s.event + s.pma_copy)
            .collect();

        println!(
            "perf: pokes={}, peeks={}, warmup_skipped={}",
            iters,
            iters,
            skip
        );
        let (_event_avg, _, _) = summarize_samples("poke_event", &event_samples);
        let (pma_avg, _, _) = summarize_samples("poke_pma_copy", &pma_copy_samples);
        let (total_avg, _, _) = summarize_samples("poke_event_plus_pma", &total_samples);
        summarize_samples("poke_wall", poke_wall);
        summarize_samples("peek_wall", peek_wall);
        if total_avg > 0.0 {
            println!(
                "perf: poke_pma_share_avg_pct={:.1}",
                (pma_avg / total_avg) * 100.0
            );
        }
    }

    // Tests for fallback to previous checkpoint if checkpoint is corrupt
    // TODO: ask about this test and reframe it for 'Saver'
    /*
    #[tokio::test]
    #[traced_test]
    #[cfg_attr(miri, ignore)]
    async fn test_nockapp_corrupt_check() {
        let (temp, mut nockapp) = setup_nockapp("test-ker.jam").await;
        assert_eq!(nockapp.kernel.serf.event_number.load(Ordering::SeqCst), 0);

        // Save a valid checkpoint
        save_nockapp(&mut nockapp).await;

        // Generate an invalid checkpoint by incrementing the event number
        let mut invalid = nockapp
            .kernel
            .checkpoint()
            .await
            .expect("Could not get kernel checkpoint");
        invalid.event_num += 1;
        assert!(!invalid.validate());

        // The invalid checkpoint has a higher event number than the valid checkpoint
        let mut checkpoint_stack = NockStack::new(NOCK_STACK_SIZE, 0);
        let valid = jam_paths
            .load_checkpoint(&mut checkpoint_stack)
            .unwrap_or_else(|err| {
                panic!(
                    "Panicked with {err:?} at {}:{} (git sha: {:?})",
                    file!(),
                    line!(),
                    option_env!("GIT_SHA")
                )
            });
        assert!(valid.event_num < invalid.event_num);

        // Save the corrupted checkpoint, because of the toggle buffer, we will write to jam file 1
        assert!(!jam_paths.1.exists());
        let jam_path = &jam_paths.1;
        let jam_bytes = invalid.encode().unwrap_or_else(|err| {
            panic!(
                "Panicked with {err:?} at {}:{} (git sha: {:?})",
                file!(),
                line!(),
                option_env!("GIT_SHA")
            )
        });
        tokio::fs::write(jam_path, jam_bytes)
            .await
            .unwrap_or_else(|err| {
                panic!(
                    "Panicked with {err:?} at {}:{} (git sha: {:?})",
                    file!(),
                    line!(),
                    option_env!("GIT_SHA")
                )
            });

        // The loaded checkpoint will be the valid one
        let chk = jam_paths
            .load_checkpoint(&mut checkpoint_stack)
            .unwrap_or_else(|err| {
                panic!(
                    "Panicked with {err:?} at {}:{} (git sha: {:?})",
                    file!(),
                    line!(),
                    option_env!("GIT_SHA")
                )
            });
        assert!(chk.event_num == valid.event_num);
    }
    */

    #[tokio::test(flavor = "current_thread")]
    #[cfg_attr(miri, ignore)]
    async fn test_jam_equality_stack() {
        let _test_arena = TestArena::default();
        let (_temp, nockapp) = setup_nockapp("test-ker.jam").await;
        let kernel = nockapp.kernel;
        let mut jam_stack = NockStack::new(NOCK_STACK_SIZE, 0);
        let arvo_slab = kernel
            .serf
            .get_kernel_state_slab()
            .await
            .expect("Could not get kernel state slab");
        let mut arvo = arvo_slab.copy_to_stack(&mut jam_stack);
        let j = jam(&mut jam_stack, arvo);
        let mut c = cue(&mut jam_stack, j).unwrap_or_else(|err| {
            panic!(
                "Panicked with {err:?} at {}:{} (git sha: {:?})",
                file!(),
                line!(),
                option_env!("GIT_SHA")
            )
        });
        // new nockstack
        unsafe { assert!(unifying_equality(&mut jam_stack, &mut arvo, &mut c)) }
    }

    // This actually gets used to test with miri
    // but when it was successful it took too long.
    #[test]
    #[cfg_attr(miri, ignore)]
    fn test_jam_equality_slab_no_driver() {
        let _test_arena = TestArena::default();
        let bytes = include_bytes!("../../test-jams/test-ker.jam");
        let mut slab1: NounSlab = NounSlab::new();
        slab1
            .cue_into(Bytes::from(Vec::from(bytes)))
            .unwrap_or_else(|err| {
                panic!(
                    "Panicked with {err:?} at {}:{} (git sha: {:?})",
                    file!(),
                    line!(),
                    option_env!("GIT_SHA")
                )
            });
        let jammed_bytes = slab1.jam();
        let mut slab2: NounSlab = NounSlab::new();
        let _c = slab2.cue_into(jammed_bytes).unwrap_or_else(|err| {
            panic!(
                "Panicked with {err:?} at {}:{} (git sha: {:?})",
                file!(),
                line!(),
                option_env!("GIT_SHA")
            )
        });
        assert!(slab_equality(&slab1, &slab2));
    }

    #[tokio::test(flavor = "current_thread")]
    #[cfg_attr(miri, ignore)]
    async fn test_jam_equality_slab() {
        let _test_arena = TestArena::default();
        let (_temp, nockapp) = setup_nockapp("test-ker.jam").await;
        let kernel = nockapp.kernel;
        let mut state_slab = kernel
            .serf
            .get_kernel_state_slab()
            .await
            .expect("Could not get kernel state slab");
        let bytes = state_slab.jam();
        let c = state_slab.cue_into(bytes).unwrap_or_else(|err| {
            panic!(
                "Panicked with {err:?} at {}:{} (git sha: {:?})",
                file!(),
                line!(),
                option_env!("GIT_SHA")
            )
        });
        let space = state_slab.noun_space();
        let root = unsafe { state_slab.root() };
        assert!(noun_equality(root.in_space(&space), c.in_space(&space)));
    }

    #[tokio::test(flavor = "current_thread")]
    #[cfg_attr(miri, ignore)]
    async fn test_jam_equality_slab_stack() {
        let _test_arena = TestArena::default();
        let (_temp, nockapp) = setup_nockapp("test-ker.jam").await;
        let kernel = nockapp.kernel;
        let mut stack = NockStack::new(NOCK_STACK_SIZE, 0);
        let state_slab = kernel
            .serf
            .get_kernel_state_slab()
            .await
            .expect("Failed to get kernel state slab");
        // Use slab to jam
        let bytes = state_slab.jam();
        // Use the stack to cue
        let mut c = Noun::cue_bytes(&mut stack, &bytes).unwrap_or_else(|err| {
            panic!(
                "Panicked with {err:?} at {}:{} (git sha: {:?})",
                file!(),
                line!(),
                option_env!("GIT_SHA")
            )
        });
        let mut state_stack = state_slab.copy_to_stack(&mut stack);
        unsafe {
            // check for equality
            assert!(unifying_equality(&mut stack, &mut state_stack, &mut c))
        }
    }
}
