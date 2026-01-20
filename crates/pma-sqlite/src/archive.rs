use std::collections::HashMap;

use nockvm::noun::{view_noun, Noun, NounSpace, NounView};
use rkyv::{Archive, Serialize};

use crate::{PmaSqliteError, Result};

#[derive(Archive, Serialize, Debug)]
pub struct NounArchive {
    pub root: u32,
    pub nodes: Vec<NounNode>,
    pub atom_bytes: Vec<u8>,
}

#[derive(Archive, Serialize, Debug)]
pub enum NounNode {
    DirectAtom(u64),
    IndirectAtom { offset: u32, len: u32 },
    Cell { head: u32, tail: u32 },
}

pub type ArchivedNoun = <NounArchive as Archive>::Archived;

enum BuildFrame {
    Visit(Noun),
    FinalizeCell { idx: u32, head: Noun, tail: Noun },
}

pub struct NounArchiveBuilder {
    nodes: Vec<NounNode>,
    atom_bytes: Vec<u8>,
    index_map: HashMap<u64, u32>,
    stack: Vec<BuildFrame>,
}

impl NounArchiveBuilder {
    pub fn new() -> Self {
        Self {
            nodes: Vec::new(),
            atom_bytes: Vec::new(),
            index_map: HashMap::new(),
            stack: Vec::new(),
        }
    }

    pub fn reserve_nodes(&mut self, nodes: usize) {
        if self.nodes.capacity() < nodes {
            self.nodes.reserve(nodes - self.nodes.capacity());
        }
    }

    pub fn build(&mut self, space: &NounSpace, root: Noun) -> Result<NounArchive> {
        self.nodes.clear();
        self.atom_bytes.clear();
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
                    match view_noun(noun, space).map_err(|_| {
                        PmaSqliteError::Archive("noun was neither atom nor cell".into())
                    })? {
                        NounView::DirectAtom(value) => {
                            let idx = self.nodes.len() as u32;
                            self.index_map.insert(raw, idx);
                            self.nodes.push(NounNode::DirectAtom(value));
                        }
                        NounView::IndirectAtom(bytes) => {
                            let idx = self.nodes.len() as u32;
                            self.index_map.insert(raw, idx);
                            let offset = self.atom_bytes.len();
                            let len = bytes.len();
                            if offset > u32::MAX as usize || len > u32::MAX as usize {
                                return Err(PmaSqliteError::Archive(
                                    "atom bytes length exceeds u32::MAX".into(),
                                ));
                            }
                            let end = offset.saturating_add(len);
                            if end > u32::MAX as usize {
                                return Err(PmaSqliteError::Archive(
                                    "atom bytes offset exceeds u32::MAX".into(),
                                ));
                            }
                            self.atom_bytes.extend_from_slice(bytes);
                            self.nodes.push(NounNode::IndirectAtom {
                                offset: offset as u32,
                                len: len as u32,
                            });
                        }
                        NounView::Cell { head, tail } => {
                            let idx = self.nodes.len() as u32;
                            self.index_map.insert(raw, idx);
                            self.nodes.push(NounNode::Cell { head: 0, tail: 0 });
                            self.stack
                                .push(BuildFrame::FinalizeCell { idx, head, tail });
                            self.stack.push(BuildFrame::Visit(tail));
                            self.stack.push(BuildFrame::Visit(head));
                        }
                    }
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
        let atom_bytes = std::mem::take(&mut self.atom_bytes);
        Ok(NounArchive {
            root: root_idx,
            nodes,
            atom_bytes,
        })
    }

    pub fn recycle(&mut self, mut archive: NounArchive) {
        self.nodes = std::mem::take(&mut archive.nodes);
        self.atom_bytes = std::mem::take(&mut archive.atom_bytes);
    }
}

pub fn build_archive(space: &NounSpace, root: Noun) -> Result<NounArchive> {
    let mut builder = NounArchiveBuilder::new();
    builder.build(space, root)
}
