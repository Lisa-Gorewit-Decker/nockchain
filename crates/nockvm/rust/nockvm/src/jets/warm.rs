use std::ptr::{copy_nonoverlapping, null_mut};

use crate::hamt::Hamt;
use crate::jets::cold::{Batteries, Cold};
use crate::jets::hot::Hot;
use crate::jets::Jet;
use crate::mem::{NockStack, Preserve, Retag};
use crate::noun::{Noun, Slots};

/// key = formula
#[derive(Copy, Clone)]
pub struct Warm(Hamt<WarmEntry>);

impl Preserve for Warm {
    unsafe fn assert_in_stack(&self, stack: &NockStack) {
        self.0.assert_in_stack(stack);
    }
    unsafe fn preserve(&mut self, stack: &mut NockStack) {
        self.0.preserve(stack);
    }
}

#[derive(Copy, Clone)]
struct WarmEntry(*mut WarmEntryMem);

const WARM_ENTRY_NIL: WarmEntry = WarmEntry(null_mut());

struct WarmEntryMem {
    batteries: Batteries,
    jet: Jet,
    path: Noun, // useful for profiling/debugging
    test: bool, // Whether to *also* run the hoon for this jet
    next: WarmEntry,
}

impl Preserve for WarmEntry {
    unsafe fn assert_in_stack(&self, stack: &NockStack) {
        if self.0.is_null() {
            return;
        };
        let mut cursor = *self;
        loop {
            stack.assert_struct_is_in(cursor.0, 1);
            (*cursor.0).batteries.assert_in_stack(stack);
            (*cursor.0).path.assert_in_stack(stack);
            if (*cursor.0).next.0.is_null() {
                break;
            };
            cursor = (*cursor.0).next;
        }
    }
    unsafe fn preserve(&mut self, stack: &mut NockStack) {
        if self.0.is_null() {
            return;
        }
        let mut ptr: *mut *mut WarmEntryMem = &mut self.0;
        loop {
            if stack.is_in_frame(*ptr) {
                (**ptr).batteries.preserve(stack);
                (**ptr).path.preserve(stack);
                let dest_mem: *mut WarmEntryMem = stack.struct_alloc_in_previous_frame(1);
                copy_nonoverlapping(*ptr, dest_mem, 1);
                *ptr = dest_mem;
                ptr = &mut ((*dest_mem).next.0);
                if (*dest_mem).next.0.is_null() {
                    break;
                };
            } else {
                break;
            }
        }
    }
}

impl Retag for WarmEntry {
    fn retag(&mut self, stack: &NockStack) {
        let mut cursor = *self;
        while !cursor.0.is_null() {
            unsafe {
                let entry = &mut *cursor.0;
                entry.batteries.retag(stack);
                entry.path.retag(stack);
                cursor = entry.next;
            }
        }
    }
}

impl Iterator for WarmEntry {
    type Item = (Noun, Batteries, Jet, bool);
    fn next(&mut self) -> Option<Self::Item> {
        if self.0.is_null() {
            return None;
        }
        unsafe {
            let res = (
                (*(self.0)).path,
                (*(self.0)).batteries,
                (*(self.0)).jet,
                (*(self.0)).test,
            );
            *self = (*(self.0)).next;
            Some(res)
        }
    }
}

#[derive(Default)]
pub enum JetLookupResult {
    Run {
        jet: Jet,
        path: Noun,
    },
    Test {
        jet: Jet,
        path: Noun,
    },
    #[default]
    NoJet,
}

impl Iterator for JetLookupResult {
    type Item = (Jet, Noun, bool);
    fn next(&mut self) -> Option<Self::Item> {
        match std::mem::take(self) {
            JetLookupResult::Run { jet, path } => Some((jet, path, false)),
            JetLookupResult::Test { jet, path } => Some((jet, path, true)),
            JetLookupResult::NoJet => None,
        }
    }
}

impl Warm {
    #[allow(clippy::new_without_default)]
    pub fn new(stack: &mut NockStack) -> Self {
        Warm(Hamt::new(stack))
    }

    fn insert(
        &mut self,
        stack: &mut NockStack,
        formula: &mut Noun,
        path: Noun,
        batteries: Batteries,
        jet: Jet,
        test: bool,
    ) {
        let current_warm_entry = self.0.lookup(stack, formula).unwrap_or(WARM_ENTRY_NIL);
        unsafe {
            let warm_entry_mem_ptr: *mut WarmEntryMem = stack.struct_alloc(1);
            *warm_entry_mem_ptr = WarmEntryMem {
                batteries,
                jet,
                path,
                test,
                next: current_warm_entry,
            };
            self.0 = self.0.insert(stack, formula, WarmEntry(warm_entry_mem_ptr));
        }
    }

    pub fn init(stack: &mut NockStack, cold: &mut Cold, hot: &Hot, test_jets: &Hamt<()>) -> Self {
        let mut warm = Self::new(stack);
        for (mut path, axis, jet) in *hot {
            let test_path = test_jets.lookup(stack, &mut path).is_some();
            let batteries_list = cold.find(stack, &mut path);
            for batteries in batteries_list {
                let mut batteries_tmp = batteries;
                let (battery, _parent_axis) = batteries_tmp
                    .next()
                    .expect("IMPOSSIBLE: empty battery entry in cold state");
                if let Ok(mut formula) = unsafe { (*battery).slot_atom(axis) } {
                    warm.insert(stack, &mut formula, path, batteries, jet, test_path);
                } else {
                    //  XX: need NockStack allocated string interpolation
                    // eprintln!("Bad axis {} into formula {:?}", axis, battery);
                    continue;
                }
            }
        }
        warm
    }

    /// Walk through the linked list of WarmEntry objects and do a partial check
    /// against the subject using Batteries (walk to root of parent batteries).
    /// If there's a match, then we've found a valid jet.
    pub fn find_jet(
        &mut self,
        stack: &mut NockStack,
        s: &mut Noun,
        f: &mut Noun,
    ) -> JetLookupResult {
        let Some(warm_it) = self.0.lookup(stack, f) else {
            return JetLookupResult::NoJet;
        };
        for (path, batteries, jet, test) in warm_it {
            if batteries.matches(stack, *s) {
                if test {
                    return JetLookupResult::Test { jet, path };
                } else {
                    return JetLookupResult::Run { jet, path };
                }
            }
        }
        JetLookupResult::NoJet
    }
}

impl Retag for Warm {
    fn retag(&mut self, stack: &NockStack) {
        self.0.retag(stack);
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::interpreter::Context;
    use crate::jets::cold::{Batteries, BatteriesMem, NO_BATTERIES};
    use crate::jets::JetErr;
    use crate::mem::NockStack;
    use crate::noun::{NounAllocator, D};
    use crate::pma::{Pma, PmaCopy};
    use std::path::PathBuf;

    const DEFAULT_STACK_SIZE: usize = 1 << 16;

    fn make_test_stack(size: usize) -> NockStack {
        NockStack::new(size, 0)
    }

    /// Dummy jet function for testing
    fn dummy_jet(_ctx: &mut Context, _subj: Noun) -> Result<Noun, JetErr> {
        Ok(D(42))
    }

    /// Another dummy jet function to differentiate entries
    fn dummy_jet_2(_ctx: &mut Context, _subj: Noun) -> Result<Noun, JetErr> {
        Ok(D(99))
    }

    /// Create a simple Batteries for testing (single entry with given battery value)
    fn make_simple_batteries(stack: &mut NockStack, battery_value: u64) -> Batteries {
        let batteries_mem: *mut BatteriesMem = unsafe { stack.alloc_struct(1) };
        unsafe {
            batteries_mem.write(BatteriesMem {
                battery: D(battery_value),
                parent_axis: D(0).as_atom().expect("0 is a valid atom"),
                parent_batteries: NO_BATTERIES,
            });
        }
        Batteries::new(batteries_mem)
    }

    /// Create a WarmEntry linked list for testing
    fn make_warm_entry(stack: &mut NockStack, entries: &[(u64, Jet, u64, bool)]) -> WarmEntry {
        let mut warm_entry = WARM_ENTRY_NIL;
        for &(battery_value, jet, path_value, test) in entries.iter().rev() {
            let batteries = make_simple_batteries(stack, battery_value);
            let warm_entry_mem: *mut WarmEntryMem = unsafe { stack.alloc_struct(1) };
            unsafe {
                warm_entry_mem.write(WarmEntryMem {
                    batteries,
                    jet,
                    path: D(path_value),
                    test,
                    next: warm_entry,
                });
            }
            warm_entry = WarmEntry(warm_entry_mem);
        }
        warm_entry
    }

    /// Helper to verify a noun is not stack-allocated (is in offset form)
    fn verify_noun_not_stack_allocated(noun: Noun, context: &str) {
        if noun.is_direct() {
            return;
        }
        assert!(
            !noun.is_stack_allocated(),
            "{} should be in offset form after evacuation",
            context
        );
        if let Ok(cell) = noun.as_cell() {
            verify_noun_not_stack_allocated(cell.head(), context);
            verify_noun_not_stack_allocated(cell.tail(), context);
        }
    }

    /// Verifies WarmEntry can be evacuated to PMA and remains functional.
    ///
    /// This test exercises:
    /// - Creating a WarmEntry linked list with multiple entries
    /// - Evacuating the WarmEntry to PMA via copy_to_pma
    /// - Verifying all entries are still accessible after evacuation
    /// - Verifying all nouns are in offset form (not stack-allocated)
    /// - Verifying the WarmEntry passes assert_in_pma
    ///
    /// Note: copy_to_pma sets forwarding pointers in the source nouns, which corrupts
    /// them for normal use. We use expected values for comparison.
    #[test]
    #[cfg(any())] // TODO: Enable when PmaCopy for WarmEntry is implemented
    #[cfg_attr(miri, ignore)]
    fn test_evacuate_warm_entry_round_trip() {
        let mut stack = make_test_stack(DEFAULT_STACK_SIZE);
        let mut pma = Pma::new(100000, PathBuf::from("/tmp/test_warm_entry_pma"))
            .expect("Failed to create test PMA");

        // Install PMA arena for offset-form access
        let _guard = pma.install();

        // Create WarmEntry linked list with two entries
        // (battery_value, jet, path_value, test)
        let entries: Vec<(u64, Jet, u64, bool)> = vec![
            (10, dummy_jet, 100, false),
            (20, dummy_jet_2, 200, true),
        ];
        let mut warm_entry = make_warm_entry(&mut stack, &entries);

        // Evacuate WarmEntry to PMA
        unsafe {
            warm_entry.copy_to_pma(&stack, &mut pma);
        }

        // Iterate over evacuated warm_entry and verify values
        let mut expected_iter = entries.iter();
        for (path, batteries, jet, test) in warm_entry {
            let (expected_battery, expected_jet, expected_path, expected_test) = expected_iter
                .next()
                .expect("WarmEntry has more entries than expected");

            // Verify path
            assert_eq!(
                unsafe { path.as_raw() },
                *expected_path,
                "Path should match"
            );

            // Verify jet function pointer
            assert!(
                std::ptr::fn_addr_eq(jet, *expected_jet),
                "Jet function pointer should match"
            );

            // Verify test flag
            assert_eq!(test, *expected_test, "Test flag should match");

            // Verify batteries (first entry only for simplicity)
            let mut batteries_iter = batteries.into_iter();
            let (battery_ptr, parent_axis) = batteries_iter
                .next()
                .expect("Batteries should have at least one entry");
            let battery = unsafe { *battery_ptr };
            assert_eq!(
                unsafe { battery.as_raw() },
                *expected_battery,
                "Battery value should match"
            );
            assert_eq!(parent_axis.as_u64().unwrap(), 0, "Parent axis should be 0");

            // Verify nouns are in offset form
            verify_noun_not_stack_allocated(path, "WarmEntry path");
            verify_noun_not_stack_allocated(battery, "WarmEntry battery");
        }

        assert!(
            expected_iter.next().is_none(),
            "WarmEntry has fewer entries than expected"
        );

        // Verify the WarmEntry passes assert_in_pma
        warm_entry.assert_in_pma(&pma);
    }

    /// Verifies Warm state can be evacuated to PMA and remains functional.
    ///
    /// This test exercises:
    /// - Creating a Warm HAMT with multiple formula->WarmEntry mappings
    /// - Evacuating the Warm to PMA via copy_to_pma
    /// - Verifying all entries are still accessible via lookup after evacuation
    /// - Verifying all nouns are in offset form (not stack-allocated)
    /// - Verifying the Warm passes assert_in_pma
    ///
    /// Note: copy_to_pma sets forwarding pointers in the source nouns, which corrupts
    /// them for normal use. We use expected values for comparison.
    #[test]
    #[cfg(any())] // TODO: Enable when PmaCopy for Warm is implemented
    #[cfg_attr(miri, ignore)]
    fn test_evacuate_warm_round_trip() {
        let mut stack = make_test_stack(DEFAULT_STACK_SIZE);
        let mut pma = Pma::new(100000, PathBuf::from("/tmp/test_warm_pma"))
            .expect("Failed to create test PMA");

        // Install PMA arena for offset-form access
        let _guard = pma.install();

        // Create a Warm and insert some entries
        let mut warm = Warm::new(&mut stack);

        // Insert entry 1: formula D(100) -> (battery=10, jet=dummy_jet, path=1000, test=false)
        let batteries1 = make_simple_batteries(&mut stack, 10);
        let mut formula1 = D(100);
        warm.insert(&mut stack, &mut formula1, D(1000), batteries1, dummy_jet, false);

        // Insert entry 2: formula D(200) -> (battery=20, jet=dummy_jet_2, path=2000, test=true)
        let batteries2 = make_simple_batteries(&mut stack, 20);
        let mut formula2 = D(200);
        warm.insert(&mut stack, &mut formula2, D(2000), batteries2, dummy_jet_2, true);

        // Insert entry 3: same formula as entry 1, different jet (creates linked list)
        let batteries3 = make_simple_batteries(&mut stack, 30);
        let mut formula3 = D(100);
        warm.insert(&mut stack, &mut formula3, D(3000), batteries3, dummy_jet_2, true);

        // Expected values for verification
        // formula D(100) should have two entries: (30, dummy_jet_2, 3000, true) -> (10, dummy_jet, 1000, false)
        // formula D(200) should have one entry: (20, dummy_jet_2, 2000, true)
        let expected_formula_100: Vec<(u64, Jet, u64, bool)> = vec![
            (30, dummy_jet_2, 3000, true),
            (10, dummy_jet, 1000, false),
        ];
        let expected_formula_200: Vec<(u64, Jet, u64, bool)> = vec![
            (20, dummy_jet_2, 2000, true),
        ];

        // Evacuate Warm to PMA
        unsafe {
            warm.copy_to_pma(&stack, &mut pma);
        }

        // Verify lookup for formula D(100)
        let mut lookup_key1 = D(100);
        let warm_entry1 = warm.0.lookup(&mut stack, &mut lookup_key1)
            .expect("Should find entry for formula D(100)");

        let mut expected_iter1 = expected_formula_100.iter();
        for (path, batteries, jet, test) in warm_entry1 {
            let (expected_battery, expected_jet, expected_path, expected_test) = expected_iter1
                .next()
                .expect("WarmEntry has more entries than expected");

            assert_eq!(unsafe { path.as_raw() }, *expected_path, "Path should match");
            assert!(std::ptr::fn_addr_eq(jet, *expected_jet), "Jet should match");
            assert_eq!(test, *expected_test, "Test flag should match");

            // Verify battery
            let mut batteries_iter = batteries.into_iter();
            let (battery_ptr, _) = batteries_iter.next().expect("Batteries should have entry");
            let battery = unsafe { *battery_ptr };
            assert_eq!(unsafe { battery.as_raw() }, *expected_battery, "Battery should match");

            // Verify nouns are in offset form
            verify_noun_not_stack_allocated(path, "Warm path");
            verify_noun_not_stack_allocated(battery, "Warm battery");
        }
        assert!(expected_iter1.next().is_none(), "Missing entries for formula D(100)");

        // Verify lookup for formula D(200)
        let mut lookup_key2 = D(200);
        let warm_entry2 = warm.0.lookup(&mut stack, &mut lookup_key2)
            .expect("Should find entry for formula D(200)");

        let mut expected_iter2 = expected_formula_200.iter();
        for (path, batteries, jet, test) in warm_entry2 {
            let (expected_battery, expected_jet, expected_path, expected_test) = expected_iter2
                .next()
                .expect("WarmEntry has more entries than expected");

            assert_eq!(unsafe { path.as_raw() }, *expected_path, "Path should match");
            assert!(std::ptr::fn_addr_eq(jet, *expected_jet), "Jet should match");
            assert_eq!(test, *expected_test, "Test flag should match");

            // Verify battery
            let mut batteries_iter = batteries.into_iter();
            let (battery_ptr, _) = batteries_iter.next().expect("Batteries should have entry");
            let battery = unsafe { *battery_ptr };
            assert_eq!(unsafe { battery.as_raw() }, *expected_battery, "Battery should match");

            // Verify nouns are in offset form
            verify_noun_not_stack_allocated(path, "Warm path");
            verify_noun_not_stack_allocated(battery, "Warm battery");
        }
        assert!(expected_iter2.next().is_none(), "Missing entries for formula D(200)");

        // Verify non-existent lookup returns None
        let mut lookup_key3 = D(999);
        assert!(
            warm.0.lookup(&mut stack, &mut lookup_key3).is_none(),
            "Lookup for non-existent formula should return None"
        );

        // Verify the Warm passes assert_in_pma
        warm.assert_in_pma(&pma);
    }
}
