use nockvm::jets::sort::util::gor;
use nockvm::mem::NockStack;
use nockvm::noun::{Cell, Noun, NounSpace};
use nockvm::unifying_equality::unifying_equality;

use crate::noun_ext::NounMathExt;

#[derive(Copy, Clone)]
pub struct HoonList<'a> {
    pub(super) next: Option<Cell>,
    pub(super) space: &'a NounSpace,
}

impl<'a> Iterator for HoonList<'a> {
    type Item = Noun;

    #[inline(always)]
    fn next(&mut self) -> Option<Self::Item> {
        self.next.take().map(|cell| {
            let tail = cell.tail(self.space);
            self.next = if tail.is_cell() {
                Some(tail.as_cell().unwrap_or_else(|err| {
                    panic!(
                        "Panicked with {err:?} at {}:{} (git sha: {:?})",
                        file!(),
                        line!(),
                        option_env!("GIT_SHA")
                    )
                }))
            } else {
                None
            };
            cell.head(self.space)
        })
    }
}

impl<'a> HoonList<'a> {
    pub fn try_from(n: Noun, space: &'a NounSpace) -> core::result::Result<Self, nockvm::noun::Error> {
        if n.is_cell() {
            Ok(HoonList::from_cell(
                n.as_cell().unwrap_or_else(|err| {
                    panic!(
                        "Panicked with {err:?} at {}:{} (git sha: {:?})",
                        file!(),
                        line!(),
                        option_env!("GIT_SHA")
                    )
                }),
                space,
            ))
        } else {
            Ok(HoonList { next: None, space })
        }
    }

    pub fn from_cell(c: Cell, space: &'a NounSpace) -> Self {
        Self {
            next: Some(c),
            space,
        }
    }
}

pub fn next_cell(cell: Cell, space: &NounSpace) -> Option<Cell> {
    let tail = cell.tail(space);
    if tail.is_cell() {
        Some(tail.as_cell().unwrap_or_else(|err| {
            panic!(
                "Panicked with {err:?} at {}:{} (git sha: {:?})",
                file!(),
                line!(),
                option_env!("GIT_SHA")
            )
        }))
    } else {
        None
    }
}

#[allow(dead_code)]
#[derive(Copy, Clone)]
pub struct HoonMap<'a> {
    pub(super) node: Noun,
    pub(super) left: Option<Cell>,
    pub(super) right: Option<Cell>,
    pub(super) space: &'a NounSpace,
}

impl<'a> HoonMap<'a> {
    pub fn get(&self, stack: &mut NockStack, mut k: Noun) -> Option<Noun> {
        let [mut ck, cv] = self.node.uncell(self.space).ok()?;

        if unsafe { unifying_equality(stack, &mut ck, &mut k) } {
            // ?:  =(b p.n.a)
            //   (some q.n.a)
            Some(cv)
        } else if gor(stack, k, ck, self.space)
            .as_direct()
            .map(|v| v.data())
            == Ok(0)
        {
            // ?:  (gor b p.n.a)
            //   $(a l.a)
            let map = Self::try_from_cell(self.left?, self.space).ok()?;
            map.get(stack, k)
        } else {
            // $(a r.a)
            let map = Self::try_from_cell(self.right?, self.space).ok()?;
            map.get(stack, k)
        }
    }

    pub fn try_from(n: Noun, space: &'a NounSpace) -> std::result::Result<Self, nockvm::noun::Error> {
        if n.is_cell() {
            HoonMap::try_from_cell(
                n.as_cell().unwrap_or_else(|err| {
                    panic!(
                        "Panicked with {err:?} at {}:{} (git sha: {:?})",
                        file!(),
                        line!(),
                        option_env!("GIT_SHA")
                    )
                }),
                space,
            )
        } else {
            not_cell()
        }
    }

    pub fn try_from_cell(c: Cell, space: &'a NounSpace) -> std::result::Result<Self, nockvm::noun::Error> {
        let tail: Noun = c.tail(space);
        if let Ok(cell_tail) = tail.as_cell() {
            let left = cell_tail.head(space);
            let right = cell_tail.tail(space);

            Ok(Self {
                node: c.head(space),
                left: left.as_cell().ok(),
                right: right.as_cell().ok(),
                space,
            })
        } else {
            not_cell()
        }
    }
}
#[allow(dead_code)]
#[derive(Clone)]
pub struct HoonMapIter<'a> {
    pub(super) stack: Vec<Option<Cell>>,
    pub(super) space: &'a NounSpace,
}

impl<'a> Iterator for HoonMapIter<'a> {
    type Item = Noun;

    #[inline(always)]
    fn next(&mut self) -> Option<Self::Item> {
        if let Some(maybe_cell) = self.stack.pop() {
            if let Some(cell) = maybe_cell {
                if let Ok(cell_trie) = HoonMap::try_from_cell(cell, self.space) {
                    self.stack.push(cell_trie.right);
                    self.stack.push(cell_trie.left);
                    return Some(cell_trie.node);
                } else {
                    return self.next();
                }
            } else {
                return self.next();
            }
        }
        None
    }
}
fn not_cell<T>() -> core::result::Result<T, nockvm::noun::Error> {
    Err(nockvm::noun::Error::NotCell)
}

impl<'a> HoonMapIter<'a> {
    pub fn new(n: Noun, space: &'a NounSpace) -> Self {
        if let Ok(c) = n.as_cell() {
            Self {
                stack: vec![Some(c)],
                space,
            }
        } else {
            Self {
                stack: vec![None],
                space,
            }
        }
    }
}
