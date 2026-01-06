use either::{Left, Right};
use nockvm::jets::util::BAIL_FAIL;
use nockvm::jets::JetErr;
use nockvm::noun::{Atom, Cell, Error, IndirectAtom, Noun, NounSpace, Result, D};
use noun_serde::{NounDecode, NounEncode};

use crate::belt::*;
use crate::felt::*;
use crate::handle::{finalize_poly, new_handle_mut_slice};
use crate::mary::*;
use crate::noun_ext::{AtomMathExt, NounMathExt};
use crate::poly::*;

impl AtomMathExt for Atom {
    fn as_u32(&self) -> Result<u32> {
        if let Ok(a) = self.as_direct() {
            if a.bit_size() > 32 {
                Err(Error::NotRepresentable)
            } else {
                Ok(a.data() as u32)
            }
        } else {
            Err(Error::NotRepresentable)
        }
    }

    fn as_belt(&self, space: &NounSpace) -> Result<Belt> {
        if let Ok(x) = self.as_u64(space) {
            Ok(Belt(x))
        } else {
            Err(Error::NotRepresentable)
        }
    }

    fn as_felt<'a>(&self, space: &NounSpace) -> Result<&'a Felt> {
        if let Ok(atom) = self.as_indirect() {
            if atom.size(space) == 4 {
                let buf_ptr = atom.data_pointer(space);
                unsafe {
                    assert!(*(buf_ptr.add(3)) == 0x1);
                }
                let felt_ref: &Felt = unsafe { &*(buf_ptr as *const Felt) };
                Ok(felt_ref)
            } else {
                Err(Error::NotRepresentable)
            }
        } else {
            Err(Error::NotRepresentable)
        }
    }

    fn as_mut_felt<'a>(&self, space: &NounSpace) -> Result<&'a mut Felt> {
        if let Ok(mut atom) = self.as_indirect() {
            if atom.size(space) == 4 {
                let buf_ptr = atom.data_pointer_mut(space);
                unsafe {
                    assert!(*(buf_ptr.add(3)) == 0x1);
                }
                let felt_ref: &mut Felt = unsafe { &mut *(buf_ptr as *mut Felt) };
                Ok(felt_ref)
            } else {
                Err(Error::NotRepresentable)
            }
        } else {
            Err(Error::NotRepresentable)
        }
    }
}

impl NounMathExt for Noun {
    fn as_belt(&self, space: &NounSpace) -> Result<Belt> {
        if let Ok(atom) = self.as_atom() {
            atom.as_belt(space)
        } else {
            Err(Error::NotRepresentable)
        }
    }

    fn as_felt<'a>(&self, space: &NounSpace) -> Result<&'a Felt> {
        if let Ok(atom) = self.as_atom() {
            atom.as_felt(space)
        } else {
            Err(Error::NotRepresentable)
        }
    }

    fn as_mut_felt<'a>(&self, space: &NounSpace) -> Result<&'a mut Felt> {
        if let Ok(atom) = self.as_atom() {
            atom.as_mut_felt(space)
        } else {
            Err(Error::NotRepresentable)
        }
    }

    fn uncell<const N: usize>(&self, space: &NounSpace) -> Result<[Self; N]> {
        let mut inp = *self;
        let mut cnt = 0;
        let mut ret = [(); N].map(|_| {
            cnt += 1;
            if cnt == N {
                Ok(inp)
            } else {
                let c = inp.as_cell()?;
                inp = c.tail(space);
                Ok(c.head(space))
            }
        });
        if let Some(e) = ret.iter_mut().find(|v| v.is_err()) {
            let n = core::mem::replace(e, Ok(D(0)));
            return Err(n.unwrap_err());
        }
        Ok(ret.map(|v| v.unwrap()))
    }
}

impl MarySlice<'_> {
    pub fn try_from(noun: Noun, space: &NounSpace) -> std::result::Result<Self, ()> {
        if noun.is_atom() {
            Err(())
        } else {
            MarySlice::try_from_cell(noun.as_cell()?, space)
        }
    }

    #[inline(always)]
    pub fn try_from_cell(c: Cell, space: &NounSpace) -> std::result::Result<Self, ()> {
        let step = c.head(space).as_atom()?.as_u32()?;
        let len = c.tail(space).as_cell()?.head(space).as_atom()?.as_u32()?;
        let cell: Cell = c.tail(space).as_cell()?;
        let dat_noun: Atom = c.tail(space).as_cell()?.tail(space).as_atom()?;
        let dat_slice: &[u64] = match dat_noun.as_either() {
            Left(_direct) => unsafe {
                let tail_ptr2 = &(*(cell.to_raw_pointer(space))).tail as *const Noun;
                std::slice::from_raw_parts(tail_ptr2 as *const u64, (len * step) as usize)
            },
            Right(indirect) => unsafe {
                std::slice::from_raw_parts(
                    indirect.data_pointer(space) as *mut u64,
                    (len * step) as usize,
                )
            },
        };
        Ok(MarySlice {
            step,
            len,
            dat: dat_slice,
        })
    }
}

impl Mary {
    pub fn try_from(noun: Noun, space: &NounSpace) -> std::result::Result<Self, ()> {
        if noun.is_atom() {
            Err(())
        } else {
            let slice = MarySlice::try_from_cell(noun.as_cell()?, space)?;
            Ok(Mary {
                step: slice.step,
                len: slice.len,
                dat: slice.dat.to_vec(),
            })
        }
    }
}

impl Table<'_> {
    pub fn try_from(noun: Noun, space: &NounSpace) -> std::result::Result<Self, ()> {
        if noun.is_atom() {
            Err(())
        } else {
            Table::try_from_cell(noun.as_cell()?, space)
        }
    }

    #[inline(always)]
    pub fn try_from_cell(c: Cell, space: &NounSpace) -> std::result::Result<Self, ()> {
        let full_width = c.head(space).as_atom()?.as_u32()?;
        let mary_cell = c.tail(space).as_cell()?;
        let mary = MarySlice::try_from_cell(mary_cell, space)?;

        Ok(Table {
            num_cols: full_width,
            mary,
        })
    }
}

// TODO: use Ares::noun::Result or Error somehow for the methods that
// convert our structs from nouns
impl BPolySlice<'_> {
    #[inline(always)]
    pub fn try_from(noun: Noun, space: &NounSpace) -> std::result::Result<Self, JetErr> {
        if noun.is_atom() {
            Err(BAIL_FAIL)
        } else {
            BPolySlice::try_from_cell(noun.as_cell()?, space)
        }
    }

    #[inline(always)]
    pub fn try_from_cell(c: Cell, space: &NounSpace) -> std::result::Result<Self, JetErr> {
        let head = c.head(space).as_atom();
        let tail = c.tail(space).as_atom();
        if let (Ok(head), Ok(tail)) = (head, tail) {
            let len32 = head.as_u32()?;
            let dat_slice: BPolySlice = unsafe {
                PolySlice(std::slice::from_raw_parts(
                    tail.data_pointer(space) as *const Belt,
                    len32 as usize,
                ))
            };
            Ok(dat_slice)
        } else {
            Err(BAIL_FAIL)
        }
    }
}

impl FPolySlice<'_> {
    #[inline(always)]
    pub fn try_from(noun: Noun, space: &NounSpace) -> std::result::Result<Self, JetErr> {
        if noun.is_atom() {
            Err(BAIL_FAIL)
        } else {
            FPolySlice::try_from_cell(noun.as_cell()?, space)
        }
    }

    #[inline(always)]
    pub fn try_from_cell(c: Cell, space: &NounSpace) -> std::result::Result<Self, JetErr> {
        let head = c.head(space).as_atom();
        let tail = c.tail(space).as_atom();
        if let (Ok(head), Ok(tail)) = (head, tail) {
            let len32 = head.as_u32()?;
            let dat_slice: FPolySlice = unsafe {
                PolySlice(std::slice::from_raw_parts(
                    tail.data_pointer(space) as *const Felt,
                    len32 as usize,
                ))
            };
            Ok(dat_slice)
        } else {
            Err(BAIL_FAIL)
        }
    }
}

impl FPolyVec {
    #[inline(always)]
    pub fn try_from(noun: Noun, space: &NounSpace) -> std::result::Result<Self, JetErr> {
        if noun.is_atom() {
            Err(BAIL_FAIL)
        } else {
            FPolyVec::try_from_cell(noun.as_cell()?, space)
        }
    }

    #[inline(always)]
    pub fn try_from_cell(c: Cell, space: &NounSpace) -> std::result::Result<Self, JetErr> {
        let head = c.head(space).as_atom();
        let tail = c.tail(space).as_atom();
        if let (Ok(head), Ok(tail)) = (head, tail) {
            let len32 = head.as_u32()?;
            let dat_vec: FPolyVec = unsafe {
                PolyVec(
                    std::slice::from_raw_parts(
                        tail.data_pointer(space) as *const Felt,
                        len32 as usize,
                    )
                    .to_vec(),
                )
            };
            Ok(dat_vec)
        } else {
            Err(BAIL_FAIL)
        }
    }
}

impl BPolyVec {
    #[inline(always)]
    pub fn try_from(noun: Noun, space: &NounSpace) -> std::result::Result<Self, JetErr> {
        if noun.is_atom() {
            Err(BAIL_FAIL)
        } else {
            BPolyVec::try_from_cell(noun.as_cell()?, space)
        }
    }

    #[inline(always)]
    pub fn try_from_cell(c: Cell, space: &NounSpace) -> std::result::Result<Self, JetErr> {
        let head = c.head(space).as_atom();
        let tail = c.tail(space).as_atom();
        if let (Ok(head), Ok(tail)) = (head, tail) {
            let len32 = head.as_u32()?;
            let dat_vec: BPolyVec = unsafe {
                PolyVec(
                    std::slice::from_raw_parts(
                        tail.data_pointer(space) as *const Belt,
                        len32 as usize,
                    )
                    .to_vec(),
                )
            };
            Ok(dat_vec)
        } else {
            Err(BAIL_FAIL)
        }
    }
}

impl NounDecode for FPolyVec {
    fn from_noun(
        noun: &nockvm::noun::Noun,
        space: &NounSpace,
    ) -> std::result::Result<Self, noun_serde::NounDecodeError> {
        FPolyVec::try_from(*noun, space)
            .map_err(|_| noun_serde::NounDecodeError::FPolyDecodeError)
    }
}

impl NounEncode for FPolyVec {
    fn to_noun<A: nockvm::noun::NounAllocator>(&self, allocator: &mut A) -> nockvm::noun::Noun {
        let (res, res_poly): (IndirectAtom, &mut [Felt]) =
            new_handle_mut_slice(allocator, Some(self.0.len() as usize));
        res_poly.copy_from_slice(&self.0);
        finalize_poly(allocator, Some(self.0.len() as usize), res)
    }
}

impl NounDecode for BPolyVec {
    fn from_noun(
        noun: &nockvm::noun::Noun,
        space: &NounSpace,
    ) -> std::result::Result<Self, noun_serde::NounDecodeError> {
        BPolyVec::try_from(*noun, space)
            .map_err(|_| noun_serde::NounDecodeError::FPolyDecodeError)
    }
}

impl NounEncode for BPolyVec {
    fn to_noun<A: nockvm::noun::NounAllocator>(&self, allocator: &mut A) -> nockvm::noun::Noun {
        let (res, res_poly): (IndirectAtom, &mut [Belt]) =
            new_handle_mut_slice(allocator, Some(self.0.len() as usize));
        res_poly.copy_from_slice(&self.0);
        finalize_poly(allocator, Some(self.0.len() as usize), res)
    }
}
