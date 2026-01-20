use std::collections::HashMap;

use nockvm::noun::{Noun, NounSpace};
use rkyv::{Archive, Serialize};

use crate::{PmaSqliteError, Result};

#[derive(Archive, Serialize, Debug)]
pub struct NounArchive {
    pub root: u32,
    pub nodes: Vec<NounNode>,
}

#[derive(Archive, Serialize, Debug)]
pub enum NounNode {
    DirectAtom(u64),
    IndirectAtom(Vec<u8>),
    Cell { head: u32, tail: u32 },
}

pub type ArchivedNoun = <NounArchive as Archive>::Archived;

enum BuildFrame {
    Visit(Noun),
    FinalizeCell { idx: u32, head: Noun, tail: Noun },
}

pub fn build_archive(space: &NounSpace, root: Noun) -> Result<NounArchive> {
    let mut nodes = Vec::new();
    let mut index_map: HashMap<u64, u32> = HashMap::new();
    let mut stack = Vec::new();
    stack.push(BuildFrame::Visit(root));

    while let Some(frame) = stack.pop() {
        match frame {
            BuildFrame::Visit(noun) => {
                let raw = unsafe { noun.as_raw() };
                if index_map.contains_key(&raw) {
                    continue;
                }
                let handle = space.handle(noun);
                if let Some(atom) = handle.atom() {
                    let idx = nodes.len() as u32;
                    index_map.insert(raw, idx);
                    if atom.is_direct() {
                        let direct = atom
                            .atom()
                            .as_direct()
                            .map_err(|_| PmaSqliteError::Archive("expected direct atom".into()))?;
                        nodes.push(NounNode::DirectAtom(direct.data()));
                    } else {
                        nodes.push(NounNode::IndirectAtom(atom.as_ne_bytes().to_vec()));
                    }
                    continue;
                }
                if let Some(cell) = handle.cell() {
                    let idx = nodes.len() as u32;
                    index_map.insert(raw, idx);
                    nodes.push(NounNode::Cell { head: 0, tail: 0 });
                    let head = cell.head().noun();
                    let tail = cell.tail().noun();
                    stack.push(BuildFrame::FinalizeCell { idx, head, tail });
                    stack.push(BuildFrame::Visit(tail));
                    stack.push(BuildFrame::Visit(head));
                    continue;
                }
                return Err(PmaSqliteError::Archive(
                    "noun was neither atom nor cell".into(),
                ));
            }
            BuildFrame::FinalizeCell { idx, head, tail } => {
                let head_raw = unsafe { head.as_raw() };
                let tail_raw = unsafe { tail.as_raw() };
                let head_idx = *index_map
                    .get(&head_raw)
                    .ok_or_else(|| PmaSqliteError::Archive("missing head index".into()))?;
                let tail_idx = *index_map
                    .get(&tail_raw)
                    .ok_or_else(|| PmaSqliteError::Archive("missing tail index".into()))?;
                match nodes
                    .get_mut(idx as usize)
                    .ok_or_else(|| PmaSqliteError::Archive("cell index out of bounds".into()))?
                {
                    NounNode::Cell { head, tail } => {
                        *head = head_idx;
                        *tail = tail_idx;
                    }
                    _ => {
                        return Err(PmaSqliteError::Archive(
                            "expected cell while finalizing".into(),
                        ));
                    }
                }
            }
        }
    }

    let root_raw = unsafe { root.as_raw() };
    let root_idx = *index_map
        .get(&root_raw)
        .ok_or_else(|| PmaSqliteError::Archive("missing root index".into()))?;
    Ok(NounArchive {
        root: root_idx,
        nodes,
    })
}
