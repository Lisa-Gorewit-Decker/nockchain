use either::{Left, Right};
use nockvm::noun::{Noun, NounSpace};

pub enum ScryResult {
    BadPath,    // ~
    Nothing,    // [~ ~]
    Some(Noun), // [~ ~ foo]
    Invalid,    // anything that isn't one of the above
}

impl ScryResult {
    pub fn from_noun(noun: &Noun, space: &NounSpace) -> ScryResult {
        match noun.in_space(space).as_either_atom_cell() {
            Left(atom) => {
                let Ok(direct) = atom.atom().as_direct() else {
                    return ScryResult::Invalid;
                };
                if direct.data() == 0 {
                    return ScryResult::BadPath;
                }
            }
            Right(cell) => {
                let Ok(head) = cell.head().noun().as_direct() else {
                    return ScryResult::Invalid;
                };
                if head.data() == 0 {
                    match cell.tail().as_either_atom_cell() {
                        Left(atom) => {
                            let Ok(direct) = atom.atom().as_direct() else {
                                return ScryResult::Invalid;
                            };
                            if direct.data() == 0 {
                                return ScryResult::Nothing;
                            }
                        }
                        Right(tail) => {
                            let Ok(tail_head) = tail.head().noun().as_direct() else {
                                return ScryResult::Invalid;
                            };
                            if tail_head.data() == 0 {
                                return ScryResult::Some(tail.tail().noun());
                            }
                        }
                    }
                }
            }
        }
        ScryResult::Invalid
    }
}
