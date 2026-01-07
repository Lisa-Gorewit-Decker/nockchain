use nockvm::jets::sort::util::gor;
use nockvm::mem::{Arena, NockStack};
use nockvm::noun::{Cell, Noun};
use nockvm::unifying_equality::unifying_equality;

use crate::noun_ext::NounMathExt;

#[derive(Copy, Clone)]
pub struct HoonList {
    pub(super) next: Option<Cell>,
}

impl Iterator for HoonList {
    type Item = Noun;

    #[inline(always)]
    fn next(&mut self) -> Option<Self::Item> {
        self.next.take().map(|cell| {
            // SAFETY: HoonList operates on stack-allocated nouns
            let tail = unsafe { cell.tail_stack() };
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
            // SAFETY: HoonList operates on stack-allocated nouns
            unsafe { cell.head_stack() }
        })
    }
}

impl TryFrom<Noun> for HoonList {
    type Error = nockvm::noun::Error;
    fn try_from(n: Noun) -> core::result::Result<Self, Self::Error> {
        if n.is_cell() {
            Ok(HoonList::from(n.as_cell().unwrap_or_else(|err| {
                panic!(
                    "Panicked with {err:?} at {}:{} (git sha: {:?})",
                    file!(),
                    line!(),
                    option_env!("GIT_SHA")
                )
            })))
        } else {
            Ok(HoonList { next: None })
        }
    }
}

impl From<Cell> for HoonList {
    fn from(c: Cell) -> Self {
        Self { next: Some(c) }
    }
}

pub fn next_cell(cell: Cell) -> Option<Cell> {
    // SAFETY: next_cell operates on stack-allocated nouns
    let tail = unsafe { cell.tail_stack() };
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
pub struct HoonMap {
    pub(super) node: Noun,
    pub(super) left: Option<Cell>,
    pub(super) right: Option<Cell>,
}

impl HoonMap {
    pub fn get(&self, stack: &mut NockStack, arena: &Arena, mut k: Noun) -> Option<Noun> {
        let [mut ck, cv] = self.node.uncell().ok()?;

        if unsafe { unifying_equality(stack, &mut ck, &mut k) } {
            // ?:  =(b p.n.a)
            //   (some q.n.a)
            Some(cv)
        } else if gor(stack, arena, k, ck).as_direct().map(|v| v.data()) == Ok(0) {
            // ?:  (gor b p.n.a)
            //   $(a l.a)
            let map: Self = self.left?.try_into().ok()?;
            map.get(stack, arena, k)
        } else {
            // $(a r.a)
            let map: Self = self.right?.try_into().ok()?;
            map.get(stack, arena, k)
        }
    }
}
#[allow(dead_code)]
#[derive(Clone)]
pub struct HoonMapIter {
    pub(super) stack: Vec<Option<Cell>>,
}

impl Iterator for HoonMapIter {
    type Item = Noun;

    #[inline(always)]
    fn next(&mut self) -> Option<Self::Item> {
        if let Some(maybe_cell) = self.stack.pop() {
            if let Some(cell) = maybe_cell {
                if let Ok(cell_trie) = HoonMap::try_from(cell) {
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

impl TryFrom<Noun> for HoonMap {
    type Error = nockvm::noun::Error;

    fn try_from(n: Noun) -> std::result::Result<Self, Self::Error> {
        if n.is_cell() {
            HoonMap::try_from(n.as_cell().unwrap_or_else(|err| {
                panic!(
                    "Panicked with {err:?} at {}:{} (git sha: {:?})",
                    file!(),
                    line!(),
                    option_env!("GIT_SHA")
                )
            }))
        } else {
            not_cell()
        }
    }
}

impl TryFrom<Cell> for HoonMap {
    type Error = nockvm::noun::Error;

    fn try_from(c: Cell) -> std::result::Result<Self, Self::Error> {
        // SAFETY: HoonMap operates on stack-allocated nouns
        unsafe {
            let tail: Noun = c.tail_stack();
            if let Ok(cell_tail) = tail.as_cell() {
                let left = cell_tail.head_stack();
                let right = cell_tail.tail_stack();

                Ok(Self {
                    node: c.head_stack(),
                    left: left.as_cell().ok(),
                    right: right.as_cell().ok(),
                })
            } else {
                not_cell()
            }
        }
    }
}

impl From<Noun> for HoonMapIter {
    fn from(n: Noun) -> Self {
        if let Ok(c) = n.as_cell() {
            Self {
                stack: vec![Some(c)],
            }
        } else {
            Self { stack: vec![None] }
        }
    }
}
