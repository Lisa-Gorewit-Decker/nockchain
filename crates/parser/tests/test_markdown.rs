use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use bytes::Bytes;
use chumsky::Parser;
use nockapp::noun::slab::{NockJammer, NounSlab};
use parser::native_parser;
use parser::utils::{diff_noun, hoon_to_noun, LineMap};

pub static MARKDOWNJAM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/markdown.jam"
));

#[test]
fn test_markdown() {
    let source_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../hoon/common/markdown/markdown.hoon");
    let source = fs::read_to_string(&source_path)
        .unwrap_or_else(|err| panic!("read {source_path:?} failed: {err}"));

    let linemap = Arc::new(LineMap::new(&source));
    let wer: Vec<String> = source_path
        .iter()
        .map(|s| s.to_string_lossy().into_owned())
        .collect();
    let hoon = match native_parser(wer, true, linemap)
        .parse(source.as_str())
        .into_result()
    {
        Ok(h) => h,
        Err(err) => {
            eprintln!("parse_block error: {err:?}");
            panic!("failed to parse markdown.hoon");
        }
    };

    let mut slab = NounSlab::<NockJammer>::new();
    let jammed = Bytes::from(MARKDOWNJAM);
    let expected_hoon = slab.cue_into(jammed).expect("cue markdown.jam");
    let actual_hoon = hoon_to_noun(&mut slab, &hoon);
    let mut printed = false;
    assert!(diff_noun(&expected_hoon, &actual_hoon, &mut printed).is_ok());
}
