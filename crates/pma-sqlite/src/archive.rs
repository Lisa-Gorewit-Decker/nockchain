use ahash::AHashMap;
use nockvm::noun::{view_noun, Noun, NounSpace, NounView};
use rkyv::{Archive, Serialize};

use crate::{PmaSqliteError, Result};

#[derive(Archive, Serialize, Debug)]
pub struct NounArchive {
    pub root: u32,
    pub tags: Vec<u8>,
    pub direct_atoms: Vec<u64>,
    pub indirect_offsets: Vec<u32>,
    pub indirect_lens: Vec<u32>,
    pub cell_heads: Vec<u32>,
    pub cell_tails: Vec<u32>,
    pub atom_bytes: Vec<u8>,
}

pub const TAG_DIRECT_ATOM: u8 = 0;
pub const TAG_INDIRECT_ATOM: u8 = 1;
pub const TAG_CELL: u8 = 2;

pub type ArchivedNoun = <NounArchive as Archive>::Archived;

enum BuildFrame {
    Visit(Noun),
    FinalizeCell { idx: u32, head: Noun, tail: Noun },
}

pub struct NounArchiveBuilder {
    tags: Vec<u8>,
    direct_atoms: Vec<u64>,
    indirect_offsets: Vec<u32>,
    indirect_lens: Vec<u32>,
    cell_heads: Vec<u32>,
    cell_tails: Vec<u32>,
    atom_bytes: Vec<u8>,
    index_map: AHashMap<u64, u32>,
    stack: Vec<BuildFrame>,
}

impl NounArchiveBuilder {
    pub fn new() -> Self {
        Self {
            tags: Vec::new(),
            direct_atoms: Vec::new(),
            indirect_offsets: Vec::new(),
            indirect_lens: Vec::new(),
            cell_heads: Vec::new(),
            cell_tails: Vec::new(),
            atom_bytes: Vec::new(),
            index_map: AHashMap::new(),
            stack: Vec::new(),
        }
    }

    pub fn reserve_nodes(&mut self, nodes: usize) {
        if self.tags.capacity() < nodes {
            self.tags.reserve(nodes - self.tags.capacity());
        }
        if self.direct_atoms.capacity() < nodes {
            self.direct_atoms
                .reserve(nodes - self.direct_atoms.capacity());
        }
        if self.indirect_offsets.capacity() < nodes {
            self.indirect_offsets
                .reserve(nodes - self.indirect_offsets.capacity());
        }
        if self.indirect_lens.capacity() < nodes {
            self.indirect_lens
                .reserve(nodes - self.indirect_lens.capacity());
        }
        if self.cell_heads.capacity() < nodes {
            self.cell_heads.reserve(nodes - self.cell_heads.capacity());
        }
        if self.cell_tails.capacity() < nodes {
            self.cell_tails.reserve(nodes - self.cell_tails.capacity());
        }
    }

    fn push_node(
        &mut self,
        tag: u8,
        direct: u64,
        offset: u32,
        len: u32,
        head: u32,
        tail: u32,
    ) -> u32 {
        let idx = self.tags.len() as u32;
        self.tags.push(tag);
        self.direct_atoms.push(direct);
        self.indirect_offsets.push(offset);
        self.indirect_lens.push(len);
        self.cell_heads.push(head);
        self.cell_tails.push(tail);
        idx
    }

    pub fn build(&mut self, space: &NounSpace, root: Noun) -> Result<NounArchive> {
        self.tags.clear();
        self.direct_atoms.clear();
        self.indirect_offsets.clear();
        self.indirect_lens.clear();
        self.cell_heads.clear();
        self.cell_tails.clear();
        self.atom_bytes.clear();
        self.index_map.clear();
        self.index_map.reserve(self.tags.capacity());
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
                            let idx = self.push_node(TAG_DIRECT_ATOM, value, 0, 0, 0, 0);
                            self.index_map.insert(raw, idx);
                        }
                        NounView::IndirectAtom(bytes) => {
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
                            let idx = self
                                .push_node(TAG_INDIRECT_ATOM, 0, offset as u32, len as u32, 0, 0);
                            self.index_map.insert(raw, idx);
                        }
                        NounView::Cell { head, tail } => {
                            let idx = self.push_node(TAG_CELL, 0, 0, 0, 0, 0);
                            self.index_map.insert(raw, idx);
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
                    let idx_usize = idx as usize;
                    if idx_usize >= self.tags.len() {
                        return Err(PmaSqliteError::Archive("cell index out of bounds".into()));
                    }
                    if self.tags[idx_usize] != TAG_CELL {
                        return Err(PmaSqliteError::Archive(
                            "expected cell while finalizing".into(),
                        ));
                    }
                    self.cell_heads[idx_usize] = head_idx;
                    self.cell_tails[idx_usize] = tail_idx;
                }
            }
        }

        let root_raw = unsafe { root.as_raw() };
        let root_idx = *self
            .index_map
            .get(&root_raw)
            .ok_or_else(|| PmaSqliteError::Archive("missing root index".into()))?;
        let tags = std::mem::take(&mut self.tags);
        let direct_atoms = std::mem::take(&mut self.direct_atoms);
        let indirect_offsets = std::mem::take(&mut self.indirect_offsets);
        let indirect_lens = std::mem::take(&mut self.indirect_lens);
        let cell_heads = std::mem::take(&mut self.cell_heads);
        let cell_tails = std::mem::take(&mut self.cell_tails);
        let atom_bytes = std::mem::take(&mut self.atom_bytes);
        Ok(NounArchive {
            root: root_idx,
            tags,
            direct_atoms,
            indirect_offsets,
            indirect_lens,
            cell_heads,
            cell_tails,
            atom_bytes,
        })
    }

    pub fn recycle(&mut self, mut archive: NounArchive) {
        self.tags = std::mem::take(&mut archive.tags);
        self.direct_atoms = std::mem::take(&mut archive.direct_atoms);
        self.indirect_offsets = std::mem::take(&mut archive.indirect_offsets);
        self.indirect_lens = std::mem::take(&mut archive.indirect_lens);
        self.cell_heads = std::mem::take(&mut archive.cell_heads);
        self.cell_tails = std::mem::take(&mut archive.cell_tails);
        self.atom_bytes = std::mem::take(&mut archive.atom_bytes);
    }
}

pub fn build_archive(space: &NounSpace, root: Noun) -> Result<NounArchive> {
    let mut builder = NounArchiveBuilder::new();
    builder.build(space, root)
}
