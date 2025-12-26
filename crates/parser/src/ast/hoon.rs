
use std::collections::*;
use num_bigint::BigUint;
use serde::Serialize;
use num_traits::Zero;

#[derive(serde::Serialize, PartialEq, Debug, Clone)]
pub enum Noun {
    Atom(Atom),
    Cell(Box<Noun>, Box<Noun>),
}

#[derive(serde::Serialize, PartialEq, Debug, Clone)]
pub enum Atom {
    Small(u128),
    #[serde(serialize_with = "serialize_biguint_decimal")]
    Big(BigUint),
}

fn serialize_biguint_decimal<S>(
    value: &BigUint,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    serializer.serialize_str(&value.to_string())
}

impl From<u16> for Atom {
    fn from(x: u16) -> Self {
        Atom::Small(x as u128)
    }
}

impl From<u32> for Atom {
    fn from(x: u32) -> Self {
        Atom::Small(x as u128)
    }
}

impl From<u64> for Atom {
    fn from(x: u64) -> Self {
        Atom::Small(x as u128)
    }
}

impl Atom {

    pub fn to_u128(&self) -> Option<u128> {
        match self {
            Atom::Small(n) => Some(*n as u128),
            Atom::Big(b) => b.try_into().ok(),
        }
    }

    pub fn to_biguint(&self) -> BigUint {
        match self {
            Atom::Small(n) => (*n).into(),
            Atom::Big(b) => b.clone(),
        }
    }

    pub fn from_biguint(b: BigUint) -> Self {
        if let Ok(n) = u128::try_from(&b) {
            Atom::Small(n)
        } else {
            Atom::Big(b)
        }
    }

    pub fn is_zero(&self) -> bool {
        match self {
            Atom::Small(n) => *n == 0,
            Atom::Big(n) => BigUint::is_zero(n),
        }
    }

    pub fn zero() -> Self {
        Atom::Small(0)
    }
}

// (-1)^s * a * 10^e
#[derive(Clone, Debug)]
pub enum DecimalFloat {
    Finite { sign: bool, exp: u128, mant: BigUint },
    Infinity { sign: bool },
    NaN,
}

//  (-1)^s * a * 2^e
#[derive(Clone, Debug)]
pub enum BinaryFloat {
    Finite { sign: bool, exp: u128, mant: BigUint },
    Infinity { sign: bool },
    NaN,
}

impl BinaryFloat {
    pub fn sign(&self) -> bool {
        match self {
            BinaryFloat::Finite { sign, .. } => *sign,
            BinaryFloat::Infinity { sign } => *sign,
            BinaryFloat::NaN => false, // irrelevant
        }
    }
}

pub type Tome = HashMap<String, Hoon>;
pub type Tune = (HashMap<String, Option<Hoon>>, Vec<Hoon>);
#[derive(serde::Serialize, PartialEq, Debug, Clone)]
pub enum TermOrTune {
    Term(String),
    Tune(Tune),
}
pub type Help = String;
pub type Knot = String;
pub type Cord = String;
pub type Path = Vec<Knot>;
pub type Tyre = Vec<(String, Hoon)>;
pub type Axis = u64;

pub type SemiNoun = (Stencil, Noun);   //  verify SemiNoun/Stencil code later...

pub type Gate = (Box<Spec>, Box<Spec>);

#[derive(serde::Serialize, PartialEq, Debug, Clone)]
pub enum Stencil {
    Half { left: Box<Stencil>, rite: Box<Stencil> },
    Full { blocks: Vec<Block> },  // change to set?
    Lazy { fragment: Axis, resolve: Gate },
}

pub type Block = Vec<Path>;

#[derive(serde::Serialize, PartialEq, Debug, Clone)]
pub enum Beer {
    Char(Cord),
    Hoon(Hoon),
}

#[derive(serde::Serialize, PartialEq, Debug, Clone)]
pub enum Woof {
    Atom(Atom),
    Hoon(Hoon),
}

#[derive(serde::Serialize, PartialEq, Debug, Clone)]
pub enum Mane {
    Tag(String),
    TagSpace(String, String),
}

#[derive(serde::Serialize, PartialEq, Debug, Clone)]
pub struct Manx {
    pub g: Marx,
    pub c: Marl,
}

pub type Marl = Vec<Tuna>;

pub type Mart = Vec<(Mane, Vec<Beer>)>;

#[derive(serde::Serialize, PartialEq, Debug, Clone)]
pub struct Marx {
    pub n: Mane,
    pub a: Mart,
}

#[derive(Debug, Clone)]
pub enum Mare {
    Manx(Manx),
    Marl(Marl),
}

#[derive(Debug, Clone)]
pub enum Maru {
    Tuna(Tuna),
    Marl(Marl),
}

#[derive(serde::Serialize, PartialEq, Debug, Clone)]
pub enum Tuna {
    Manx(Manx),
    TunaTail(TunaTail),
}

#[derive(serde::Serialize, PartialEq, Debug, Clone)]
pub enum TunaTail {
    Tape(Hoon),
    Manx(Hoon),
    Marl(Hoon),
    Call(Hoon),
}

#[derive(serde::Serialize, PartialEq, Debug, Clone)]
pub enum Chum {
    Lef(String),
    StdKel(String, Atom),
    VenProKel(String, String, Atom),
    VenProVerKel(String, String, Atom, Atom),
}

#[derive(serde::Serialize, PartialEq, Debug, Clone)]
pub struct Pint {
    pub p: (u64, u64),
    pub q: (u64, u64),
}

#[derive(serde::Serialize, PartialEq, Debug, Clone)]
pub struct Spot {
    pub p: Path,
    pub q: Pint,
}

#[derive(serde::Serialize, PartialEq, Debug, Clone)]
pub enum Limb {
    Term(String),
    Axis(u64),
    Parent(u64, Option<String>),
}

pub type WingType = Vec<Limb>;

#[derive(serde::Serialize, PartialEq, Debug, Clone)]
pub enum Spec {
    Base(BaseType),
    Dbug(Spot, Box<Spec>),
    Leaf(String, Atom),
    Like(WingType, Vec<WingType>),
    Loop(String),
    Made((String, Vec<String>), Box<Spec>),
    Make(Hoon, Vec<Spec>),
    Name(String, Box<Spec>),
    Over(WingType, Box<Spec>),
    BucGar(Box<Spec>, Box<Spec>),
    BucBuc(Box<Spec>, HashMap<String, Spec>),
    BucBar(Box<Spec>, Hoon),
    BucCab(Hoon),
    BucCol(Box<Spec>, Vec<Spec>),
    BucCen(Box<Spec>, Vec<Spec>),
    BucDot(Box<Spec>, HashMap<String, Spec>),
    BucGal(Box<Spec>, Box<Spec>),
    BucHep(Box<Spec>, Box<Spec>),
    BucKet(Box<Spec>, Box<Spec>),
    BucLus(String, Box<Spec>),
    BucFas(Box<Spec>, HashMap<String, Spec>),
    BucMic(Hoon),
    BucPam(Box<Spec>, Hoon),
    BucSig(Hoon, Box<Spec>),
    BucTic(Box<Spec>, HashMap<String, Spec>),
    BucTis(Skin, Box<Spec>),
    BucPat(Box<Spec>, Box<Spec>),
    BucWut(Box<Spec>, Vec<Spec>),
    BucZap(Box<Spec>, HashMap<String, Spec>),
}

#[derive(serde::Serialize, PartialEq, Debug, Clone)]
pub enum Nock {
    Pair(Box<Nock>, Box<Nock>),
    Const(Noun),
    Compose(Box<Nock>, Box<Nock>),
    CellTest(Box<Nock>),
    Increment(Box<Nock>),
    Equality(Box<Nock>, Box<Nock>),
    IfThenElse(Box<Nock>, Box<Nock>, Box<Nock>),
    SerialCompose(Box<Nock>, Box<Nock>),
    PushSubject(Box<Nock>, Box<Nock>),
    SelectArm(u64, Box<Nock>),
    Edit((u64, Box<Nock>), Box<Nock>),
    Hint(NockHint, Box<Nock>),
    GrabData(Box<Nock>, Box<Nock>),
    AxisSelect(u64),
}

#[derive(serde::Serialize, PartialEq, Debug, Clone)]
pub enum NockHint {
    Atom(u64),
    Pair(u64, Box<Nock>),
}

#[derive(serde::Serialize, PartialEq, Debug, Clone)]
pub enum Note {
    Know(String),
    Made(String, Option<Vec<WingType>>),
}

#[derive(serde::Serialize, PartialEq, Debug, Clone)]
pub struct Coil {
    pub p: Garb,
    pub q: Type,
    pub r: (SemiNoun, HashMap<String, Tome>),
}

#[derive(serde::Serialize, Debug, Clone, PartialEq)]
pub struct Garb {
    pub name: Option<String>,
    pub poly: Poly,
    pub vair: Vair,
}

#[derive(serde::Serialize, PartialEq, Debug, Clone)]
pub enum Poly {
    Wet,
    Dry,
}

#[derive(serde::Serialize, PartialEq, Debug, Clone)]
pub enum Vair {
    Gold,
    Iron,
    Lead,
    Zinc,
}

#[derive(serde::Serialize, Debug, Clone, PartialEq, Eq)]
pub enum BaseType {
    Noun,
    Cell,
    Flag,
    Null,
    Void,
    Atom(String),  // Aura
}

#[derive(serde::Serialize, PartialEq, Debug, Clone)]
pub enum Tiki {
    Wing((Option<String>, WingType)),
    Hoon((Option<String>, Box<Hoon>)),
}

#[derive(serde::Serialize, PartialEq, Debug, Clone)]
pub enum Skin {
    Term(String),
    Base(BaseType),
    Cell(Box<Skin>, Box<Skin>),
    Dbug(Spot, Box<Skin>),
    Leaf(String, Atom),
    Name(String, Box<Skin>),
    Over(WingType, Box<Skin>),
    Spec(Box<Spec>, Box<Skin>),
    Wash(u64),
}

#[derive(serde::Serialize, PartialEq, Debug, Clone)]
pub enum Type {
    Noun,
    Void,
    Atom(String, Option<u64>),
    Cell(Box<Type>, Box<Type>),
    Core(Box<Type>, Box<Coil>),
    Face(FaceType, Box<Type>),
    Fork(Vec<Type>), // change to set?
    Hint((Box<Type>, Note), Box<Type>),
    Hold(Box<Type>, Hoon),
}

#[derive(serde::Serialize, PartialEq, Debug, Clone)]
pub enum FaceType {
    Term(String),
    Tune(Tune),
}

#[derive(serde::Serialize, PartialEq, Debug, Clone)]
pub enum ZpwtArg {
    Atom(String),
    Pair(String, String),
}

pub type Alas = Vec<(String, Hoon)>;

#[derive(serde::Serialize, PartialEq, Debug, Clone)]
pub enum TermOrPair {
    Term(String),
    Pair(String, Box<Hoon>),
}

#[derive(serde::Serialize, PartialEq, Debug, Clone)]
pub enum Hoon {
    Pair(Box<Hoon>, Box<Hoon>),
    ZapZap,
    Axis(u64),
    Base(BaseType),
    Bust(BaseType),
    Dbug(Spot, Box<Hoon>),
    Eror(String),
    Hand(Box<Type>, Nock),
    Note(Note, Box<Hoon>),
    Fits(Box<Hoon>, WingType),
    Knit(Vec<Woof>),
    Leaf(String, Atom),
    Limb(String),
    Lost(Box<Hoon>),
    Rock(String, Noun),
    Sand(String, Noun),
    Tell(Vec<Hoon>),
    Tune(TermOrTune),
    Wing(WingType),
    Yell(Vec<Hoon>),
    Xray(Manx),
    BarBuc(Vec<String>, Box<Spec>),
    BarCab(Box<Spec>, Alas, HashMap<String, Tome>),
    BarCol(Box<Hoon>, Box<Hoon>),
    BarCen(Option<String>, HashMap<String, Tome>),
    BarDot(Box<Hoon>),
    BarKet(Box<Hoon>, HashMap<String, Tome>),
    BarHep(Box<Hoon>),
    BarSig(Box<Spec>, Box<Hoon>),
    BarTar(Box<Spec>, Box<Hoon>),
    BarTis(Box<Spec>, Box<Hoon>),
    BarPat(Option<String>, HashMap<String, Tome>),
    BarWut(Box<Hoon>),
    ColCab(Box<Hoon>, Box<Hoon>),
    ColKet(Box<Hoon>, Box<Hoon>, Box<Hoon>, Box<Hoon>),
    ColHep(Box<Hoon>, Box<Hoon>),
    ColLus(Box<Hoon>, Box<Hoon>, Box<Hoon>),
    ColSig(Vec<Hoon>),
    ColTar(Vec<Hoon>),
    CenCab(WingType, Vec<(WingType, Hoon)>),
    CenDot(Box<Hoon>, Box<Hoon>),
    CenHep(Box<Hoon>, Box<Hoon>),
    CenCol(Box<Hoon>, Vec<Hoon>),
    CenTar(WingType, Box<Hoon>, Vec<(WingType, Hoon)>),
    CenKet(Box<Hoon>, Box<Hoon>, Box<Hoon>, Box<Hoon>),
    CenLus(Box<Hoon>, Box<Hoon>, Box<Hoon>),
    CenSig(WingType, Box<Hoon>, Vec<Hoon>),
    CenTis(WingType, Vec<(WingType, Hoon)>),
    DotKet(Box<Spec>, Box<Hoon>),
    DotLus(Box<Hoon>),
    DotTar(Box<Hoon>, Box<Hoon>),
    DotTis(Box<Hoon>, Box<Hoon>),
    DotWut(Box<Hoon>),
    KetBar(Box<Hoon>),
    KetDot(Box<Hoon>, Box<Hoon>),
    KetLus(Box<Hoon>, Box<Hoon>),
    KetHep(Box<Spec>, Box<Hoon>),
    KetPam(Box<Hoon>),
    KetSig(Box<Hoon>),
    KetTis(Skin, Box<Hoon>),
    KetWut(Box<Hoon>),
    KetTar(Box<Spec>),
    KetCol(Box<Spec>),
    SigBar(Box<Hoon>, Box<Hoon>),
    SigCab(Box<Hoon>, Box<Hoon>),
    SigCen(Chum, Box<Hoon>, Tyre, Box<Hoon>),
    SigFas(Chum, Box<Hoon>),
    SigGal(TermOrPair, Box<Hoon>),
    SigGar(TermOrPair, Box<Hoon>),
    SigBuc(String, Box<Hoon>),
    SigLus(u64, Box<Hoon>),
    SigPam(u64, Box<Hoon>, Box<Hoon>),
    SigTis(Box<Hoon>, Box<Hoon>),
    SigWut(u64, Box<Hoon>, Box<Hoon>, Box<Hoon>),
    SigZap(Box<Hoon>, Box<Hoon>),
    MicTis(Marl),
    MicCol(Box<Hoon>, Vec<Hoon>),
    MicFas(Box<Hoon>),
    MicGal(Box<Spec>, Box<Hoon>, Box<Hoon>, Box<Hoon>),
    MicSig(Box<Hoon>, Vec<Hoon>),
    MicMic(Box<Spec>, Box<Hoon>),
    TisBar(Box<Spec>, Box<Hoon>),
    TisCol(Vec<(WingType, Hoon)>, Box<Hoon>),
    TisFas(Skin, Box<Hoon>, Box<Hoon>),
    TisMic(Skin, Box<Hoon>, Box<Hoon>),
    TisDot(WingType, Box<Hoon>, Box<Hoon>),
    TisWut(WingType, Box<Hoon>, Box<Hoon>, Box<Hoon>),
    TisGal(Box<Hoon>, Box<Hoon>),
    TisHep(Box<Hoon>, Box<Hoon>),
    TisGar(Box<Hoon>, Box<Hoon>),
    TisKet(Skin, WingType, Box<Hoon>, Box<Hoon>),
    TisLus(Box<Hoon>, Box<Hoon>),
    TisSig(Vec<Hoon>),
    TisTar((String, Option<Box<Spec>>), Box<Hoon>, Box<Hoon>),
    TisCom(Box<Hoon>, Box<Hoon>),
    WutBar(Vec<Hoon>),
    WutHep(WingType, Vec<(Spec, Hoon)>),
    WutCol(Box<Hoon>, Box<Hoon>, Box<Hoon>),
    WutDot(Box<Hoon>, Box<Hoon>, Box<Hoon>),
    WutKet(WingType, Box<Hoon>, Box<Hoon>),
    WutGal(Box<Hoon>, Box<Hoon>),
    WutGar(Box<Hoon>, Box<Hoon>),
    WutLus(WingType, Box<Hoon>, Vec<(Spec, Hoon)>),
    WutPam(Vec<Hoon>),
    WutPat(WingType, Box<Hoon>, Box<Hoon>),
    WutSig(WingType, Box<Hoon>, Box<Hoon>),
    WutHax(Skin, WingType),
    WutTis(Box<Spec>, WingType),
    WutZap(Box<Hoon>),
    ZapCom(Box<Hoon>, Box<Hoon>),
    ZapGar(Box<Hoon>),
    ZapGal(Box<Spec>, Box<Hoon>),
    ZapMic(Box<Hoon>, Box<Hoon>),
    ZapTis(Box<Hoon>),
    ZapPat(Vec<WingType>, Box<Hoon>, Box<Hoon>),
    ZapWut(ZpwtArg, Box<Hoon>),
}