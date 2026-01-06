use std::io::{stderr, Write};

use either::Either::*;
use nockvm::interpreter::Slogger;
use nockvm::jets::list::util::lent;
use nockvm::mem::NockStack;
use nockvm::noun::{Atom, DirectAtom, IndirectAtom, Noun, NounSpace, Slots};
use nockvm_macros::tas;
use tracing::{debug, error, info, trace, warn};

use crate::{CrownError, Result};

pub struct CrownSlogger;

impl Slogger for CrownSlogger {
    fn slog(&mut self, stack: &mut NockStack, pri: u64, tank: Noun) {
        let mut buffer = Vec::new();
        match slog_tank(stack, tank, &mut buffer) {
            Ok(_) => {
                let message = String::from_utf8_lossy(&buffer)
                    .trim_matches('\0')
                    .replace('\n', " ")
                    .to_string();
                if !message.is_empty() {
                    if cfg!(feature = "slog-tracing") {
                        match pri {
                            0 => info!(target: "slogger", "{}", message),
                            1 => warn!(target: "slogger", "{}", message),
                            2 => debug!(target: "slogger", "{}", message),
                            3 => trace!(target: "slogger", "{}", message),
                            _ => info!(target: "slogger", "{}", message),
                        }
                    } else {
                        let _ = writeln!(stderr(), "{}", message);
                    }
                }
            }
            Err(e) => {
                let err_msg = format!("Failed to slog tank: {}", e);
                if cfg!(feature = "slog-tracing") {
                    error!(target: "slogger", "{}", err_msg);
                } else {
                    let _ = writeln!(stderr(), "{}", err_msg);
                }
            }
        }
    }

    fn flog(&mut self, _stack: &mut NockStack, cord: Noun) {
        let cord_atom = cord.as_atom().unwrap_or_else(|err| {
            panic!(
                "Panicked with {err:?} at {}:{} (git sha: {:?})",
                file!(),
                line!(),
                option_env!("GIT_SHA")
            )
        });
        let space = _stack.noun_space();
        let mut buffer = Vec::new();
        match slog_cord(cord_atom, &mut buffer, &space) {
            Ok(_) => {
                let message = String::from_utf8_lossy(&buffer)
                    .trim_matches('\0')
                    .to_string();
                if !message.is_empty() {
                    if cfg!(feature = "slog-tracing") {
                        info!(target: "slogger", "{}", message);
                    } else {
                        let _ = writeln!(stderr(), "{}", message);
                    }
                }
            }
            Err(e) => {
                let err_msg = format!("Failed to flog cord: {}", e);
                if cfg!(feature = "slog-tracing") {
                    error!(target: "slogger", "{}", err_msg);
                } else {
                    let _ = writeln!(stderr(), "{}", err_msg);
                }
            }
        }
    }
}

fn slog_cord<W: Write>(cord: Atom, out: &mut W, space: &NounSpace) -> Result<()> {
    out.write_all(cord.as_ne_bytes(space))?;
    Ok(())
}

fn slog_tape<W: Write>(stack: &mut NockStack, tape: Noun, out: &mut W) -> Result<()> {
    let cord = crip(stack, tape)?;
    let space = stack.noun_space();
    slog_cord(cord, out, &space)
}

// XX TODO: pre-crip all tapes
fn slog_palm<W: Write>(stack: &mut NockStack, palm: Noun, out: &mut W) -> Result<()> {
    let space = stack.noun_space();
    let ds = palm.slot(6, &space)?;
    let fore1 = ds.slot(6, &space)?;
    let fore2 = ds.slot(14, &space)?;
    slog_tape(stack, fore1, out)?;
    slog_tape(stack, fore2, out)?;
    let mid = ds.slot(2, &space)?;
    let end = ds.slot(15, &space)?;
    let mut tanks = palm.slot(7, &space)?;
    loop {
        if let Ok(tanks_it) = tanks.as_cell() {
            slog_tank(stack, tanks_it.head(&space), out)?;
            tanks = tanks_it.tail(&space);
            if tanks.is_cell() {
                slog_tape(stack, mid, out)?;
            }
        } else {
            break slog_tape(stack, end, out);
        }
    }
}

// XX todo: pre-crip all tapes
fn slog_rose<W: Write>(stack: &mut NockStack, rose: Noun, out: &mut W) -> Result<()> {
    let space = stack.noun_space();
    let ds = rose.slot(6, &space)?;
    let fore = ds.slot(6, &space)?;
    slog_tape(stack, fore, out)?;
    let mid = ds.slot(2, &space)?;
    let end = ds.slot(7, &space)?;

    let mut tanks = rose.slot(7, &space)?;

    loop {
        if let Ok(tanks_it) = tanks.as_cell() {
            slog_tank(stack, tanks_it.head(&space), out)?;
            tanks = tanks_it.tail(&space);
            if tanks.is_cell() {
                slog_tape(stack, mid, out)?;
            }
        } else {
            break slog_tape(stack, end, out);
        }
    }
}

fn slog_tank<W: Write>(stack: &mut NockStack, tank: Noun, out: &mut W) -> Result<()> {
    let space = stack.noun_space();
    match tank.as_either_atom_cell() {
        Left(cord) => slog_cord(cord, out, &space),
        Right(cell) => {
            let tag = cell.head(&space).as_direct()?;
            match tag.data() {
                tas!(b"leaf") => slog_tape(stack, cell.tail(&space), out),
                tas!(b"palm") => slog_palm(stack, tank, out),
                tas!(b"rose") => slog_rose(stack, tank, out),
                _ => Err(CrownError::Unknown("Bad tank".to_string())),
            }
        }
    }
}

fn crip(stack: &mut NockStack, mut tape: Noun) -> Result<Atom> {
    let space = stack.noun_space();
    let l = lent(tape, &space)?;
    if l == 0 {
        return Ok(unsafe { DirectAtom::new_unchecked(0).as_atom() });
    }
    let (mut indirect, buf) = unsafe { IndirectAtom::new_raw_mut_bytes(stack, l) };

    let mut idx = 0;
    loop {
        if let Ok(tape_it) = tape.as_cell() {
            let tape_byte = tape_it.head(&space).as_direct()?;
            tape = tape_it.tail(&space);
            if tape_byte.data() >= 256 {
                break Err(CrownError::Unknown("Bad tape".to_string()));
            } else {
                buf[idx] = tape_byte.data().to_le_bytes()[0];
                idx += 1;
            }
        } else {
            break Ok(unsafe { indirect.normalize_as_atom(&space) });
        }
    }
}
