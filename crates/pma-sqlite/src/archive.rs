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

pub struct NounArchiveBuilder {
    nodes: Vec<NounNode>,
    index_map: HashMap<u64, u32>,
    stack: Vec<BuildFrame>,
}

impl NounArchiveBuilder {
    pub fn new() -> Self {
        Self {
            nodes: Vec::new(),
            index_map: HashMap::new(),
            stack: Vec::new(),
        }
    }

    pub fn build(&mut self, space: &NounSpace, root: Noun) -> Result<NounArchive> {
        self.nodes.clear();
        self.index_map.clear();
        self.stack.clear();
        self.stack.push(BuildFrame::Visit(root));

        while let Some(frame) = self.stack.pop() {
            match frame {
                BuildFrame::Visit(noun) => {
                    let raw = unsafe { noun.as_raw() };
                    if self.index_map.contains_key(&raw) {
                        continue;
                    }
                    let handle = space.handle(noun);
                    if let Some(atom) = handle.atom() {
                        let idx = self.nodes.len() as u32;
                        self.index_map.insert(raw, idx);
                        if atom.is_direct() {
                            let direct = atom.atom().as_direct().map_err(|_| {
                                PmaSqliteError::Archive("expected direct atom".into())
                            })?;
                            self.nodes.push(NounNode::DirectAtom(direct.data()));
                        } else {
                            self.nodes
                                .push(NounNode::IndirectAtom(atom.as_ne_bytes().to_vec()));
                        }
                        continue;
                    }
                    if let Some(cell) = handle.cell() {
                        let idx = self.nodes.len() as u32;
                        self.index_map.insert(raw, idx);
                        self.nodes.push(NounNode::Cell { head: 0, tail: 0 });
                        let head = cell.head().noun();
                        let tail = cell.tail().noun();
                        self.stack
                            .push(BuildFrame::FinalizeCell { idx, head, tail });
                        self.stack.push(BuildFrame::Visit(tail));
                        self.stack.push(BuildFrame::Visit(head));
                        continue;
                    }
                    return Err(PmaSqliteError::Archive(
                        "noun was neither atom nor cell".into(),
                    ));
                }
                BuildFrame::FinalizeCell { idx, head, tail } => {
                    let head_raw = unsafe { head.as_raw() };
                    let tail_raw = unsafe { tail.as_raw() };
                    let head_idx = *self
                        .index_map
                        .get(&head_raw)
                        .ok_or_else(|| PmaSqliteError::Archive("missing head index".into()))?;
                    let tail_idx = *self
                        .index_map
                        .get(&tail_raw)
                        .ok_or_else(|| PmaSqliteError::Archive("missing tail index".into()))?;
                    match self
                        .nodes
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
        let root_idx = *self
            .index_map
            .get(&root_raw)
            .ok_or_else(|| PmaSqliteError::Archive("missing root index".into()))?;
        let nodes = std::mem::take(&mut self.nodes);
        Ok(NounArchive {
            root: root_idx,
            nodes,
        })
    }

    pub fn recycle(&mut self, mut archive: NounArchive) {
        self.nodes = std::mem::take(&mut archive.nodes);
    }
}

pub fn build_archive(space: &NounSpace, root: Noun) -> Result<NounArchive> {
    let mut builder = NounArchiveBuilder::new();
    builder.build(space, root)
}
