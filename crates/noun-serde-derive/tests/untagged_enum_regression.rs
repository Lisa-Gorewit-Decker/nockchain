use nockvm::mem::{Arena, NockStack};
use noun_serde::{NounDecode, NounEncode};

#[derive(Debug, Clone, PartialEq, Eq, NounEncode, NounDecode)]
struct Hash(pub [u64; 2]);

#[derive(Debug, Clone, PartialEq, Eq, NounEncode, NounDecode)]
enum RecipientSpec {
    #[noun(tag = "pkh")]
    Pkh { hash: Hash, amount: u64 },
    #[noun(tag = "multi")]
    Multi { first: u64, second: u64 },
}

struct TestArenaGuard {
    _stack: NockStack,
}

impl TestArenaGuard {
    fn install() -> Self {
        let stack = NockStack::new(1 << 16, 0);
        stack.install_arena();
        Self { _stack: stack }
    }
}

impl Drop for TestArenaGuard {
    fn drop(&mut self) {
        Arena::clear_thread_local();
    }
}

#[test]
fn untagged_named_variant_decodes_all_fields() {
    let _arena = TestArenaGuard::install();
    let mut stack = NockStack::new(8 << 10 << 10, 0);
    let expected = RecipientSpec::Pkh {
        hash: Hash([0x1234, 0x5678]),
        amount: 42,
    };
    let noun = expected.to_noun(&mut stack);

    let decoded = RecipientSpec::from_noun(&noun).expect("recipient spec decodes");
    assert_eq!(decoded, expected);
}

#[test]
fn untagged_named_variant_with_multiple_fields_decodes_all_fields() {
    let _arena = TestArenaGuard::install();
    let mut stack = NockStack::new(8 << 10 << 10, 0);
    let expected = RecipientSpec::Multi {
        first: 0xaaaa,
        second: 0xbbbb,
    };
    let noun = expected.to_noun(&mut stack);

    let decoded = RecipientSpec::from_noun(&noun).expect("multi recipient spec decodes");
    assert_eq!(decoded, expected);
}
