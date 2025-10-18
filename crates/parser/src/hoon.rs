
use std::collections::*;
// use std::collections::HashSet;

#[derive(PartialEq, Debug, Clone)]
pub enum Noun {
    Atom(Atom),  // we are storing atoms as strings, without doing any convertion/validation...
    Cell(Box<Noun>, Box<Noun>),
}

pub type Atom = String;

pub type What = String;
pub type Term = String;
pub type Tome = (What, HashMap<Term, Hoon>);
pub type Stud = String;
pub type Tune = String;
pub type Help = String;
pub type Knot = String;
pub type Cord = String;
pub type Path = Vec<Knot>;
pub type Tyre = Vec<(Term, Hoon)>;
pub type Aura = String;
pub type Axis = u64;

pub type SemiNoun = (Stencil, Noun);   //  verify SemiNoun/Stencil code later...

pub type Gate = (Box<Spec>, Box<Spec>);

#[derive(PartialEq, Debug, Clone)]
pub enum Stencil {
    Half { left: Box<Stencil>, rite: Box<Stencil> },
    Full { blocks: Vec<Block> },  // change to set?
    Lazy { fragment: Axis, resolve: Gate },
}

pub type Block = Vec<Path>;

#[derive(PartialEq, Debug, Clone)]
pub enum Beer {
    Char(Cord),
    Hoon(Hoon),
}

// #[derive(Debug, Clone)]
// pub enum Woof {
//     Atom(Atom),
//     Hoon(Hoon),
// }
pub type Woof = String;

#[derive(PartialEq, Debug, Clone)]
pub enum Mane {
    Tag(String),
    TagSpace(String, String),
}

#[derive(PartialEq, Debug, Clone)]
pub struct Manx {
    pub g: Marx,
    pub c: Marl,
}

pub type Marl = Vec<Tuna>;

pub type Mart = Vec<(Mane, Vec<Beer>)>;

#[derive(PartialEq, Debug, Clone)]
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

#[derive(PartialEq, Debug, Clone)]
pub enum Tuna {
    Manx(Manx),
    TunaTail
}

#[derive(Debug, Clone)]
pub enum TunaTail {
    Tape(Hoon),
    Manx(Hoon),
    Marl(Hoon),
    Call(Hoon),
}

#[derive(PartialEq, Debug, Clone)]
pub enum Chum {
    Lef(Term),
    StdKel(Term, u64),
    VenProKel(Term, Term, u64),
    VenProVerKel(Term, Term, u64, u64),
}

#[derive(PartialEq, Debug, Clone)]
pub struct Pint {
    pub p: (u64, u64),
    pub q: (u64, u64),
}

#[derive(PartialEq, Debug, Clone)]
pub struct Spot {
    pub p: Path,
    pub q: Pint,
}

#[derive(PartialEq, Debug, Clone)]
pub enum Limb {
    Term(String),
    Axis(u64),
    Parent(u64, Option<String>),
}

pub type WingType = Vec<Limb>;

#[derive(PartialEq, Debug, Clone)]
pub enum Spec {
    Base(BaseType),
    Dbug(Spot, Box<Spec>),
    Gist(Help, Box<Spec>),
    Leaf(Term, Atom),
    Like(WingType, Vec<WingType>),
    Loop(Term),
    Made((Term, Vec<Term>), Box<Spec>),
    Make(Hoon, Vec<Spec>),
    Name(Term, Box<Spec>),
    Over(WingType, Box<Spec>),
    BucGar(Box<Spec>, Box<Spec>),
    BucBuc(Box<Spec>, HashMap<Term, Spec>),
    BucBar(Box<Spec>, Hoon),
    BucCab(Hoon),
    BucCol(Box<Spec>, Vec<Spec>),
    BucCen(Box<Spec>, Vec<Spec>),
    BucDot(Box<Spec>, HashMap<Term, Spec>),
    BucGal(Box<Spec>, Box<Spec>),
    BucHep(Box<Spec>, Box<Spec>),
    BucKet(Box<Spec>, Box<Spec>),
    BucLus(Stud, Box<Spec>),
    BucFas(Box<Spec>, HashMap<Term, Spec>),
    BucMic(Hoon),
    BucPam(Box<Spec>, Hoon),
    BucSig(Hoon, Box<Spec>),
    BucTic(Box<Spec>, HashMap<Term, Spec>),
    // BucTis(Box<Hoon>, Box<Spec>),
    BucTis(Skin, Box<Spec>),
    BucPat(Box<Spec>, Box<Spec>),
    BucWut(Box<Spec>, Vec<Spec>),
    BucZap(Box<Spec>, HashMap<Term, Spec>),
}

#[derive(PartialEq, Debug, Clone)]
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

#[derive(PartialEq, Debug, Clone)]
pub enum NockHint {
    Atom(u64),
    Pair(u64, Box<Nock>),
}

#[derive(PartialEq, Debug, Clone)]
pub enum Note {
    Help(Help),
    Know(Stud),
    Made(Term, Option<Vec<WingType>>),
}

#[derive(PartialEq, Debug, Clone)]
pub struct Coil {
    pub p: Garb,
    pub q: Type,
    pub r: (SemiNoun, HashMap<Term, Tome>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Garb {
    pub name: Option<Term>,
    pub poly: Poly,
    pub vair: Vair,
}

#[derive(PartialEq, Debug, Clone)]
pub enum Poly {
    Wet,
    Dry,
}

#[derive(PartialEq, Debug, Clone)]
pub enum Vair {
    Gold,
    Iron,
    Lead,
    Zinc,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BaseType {
    Noun,
    Cell,
    Flag,
    Null,
    Void,
    Atom(Aura),
}

#[derive(PartialEq, Debug, Clone)]
pub enum Tiki {
    Wing((Option<Term>, WingType)),
    Hoon((Option<Term>, Box<Hoon>)),
}

#[derive(PartialEq, Debug, Clone)]
pub enum Skin {
    Term(Term),
    Base(BaseType),
    Cell(Box<Skin>, Box<Skin>),
    Dbug(Spot, Box<Skin>),
    Leaf(Aura, Atom),
    Help(Help, Box<Skin>),
    Name(Term, Box<Skin>),
    Over(WingType, Box<Skin>),
    Spec(Box<Spec>, Box<Skin>),
    Wash(u32),
}

#[derive(PartialEq, Debug, Clone)]
pub enum Type {
    Noun,
    Void,
    Atom(Term, Option<u64>),
    Cell(Box<Type>, Box<Type>),
    Core(Box<Type>, Box<Coil>),
    Face(FaceType, Box<Type>),
    Fork(Vec<Type>), // change to set?
    Hint((Box<Type>, Note), Box<Type>),
    Hold(Box<Type>, Hoon),
}

#[derive(PartialEq, Debug, Clone)]
pub enum FaceType {
    Term(Term),
    Tune(Tune),
}

#[derive(PartialEq, Debug, Clone)]
pub enum ZpwtArg {
    Atom(u64),
    Pair(u64, u64),
}

pub type Alas = Vec<(Term, Hoon)>;

#[derive(PartialEq, Debug, Clone)]
pub enum TermOrPair {
    Term(Term),
    Pair((Term, Box<Hoon>)),
}

#[derive(PartialEq, Debug, Clone)]
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
    // Knit(Vec<Woof>),
    Knit(Woof),
    Leaf(Term, Atom),
    Limb(Term),
    Lost(Box<Hoon>),
    Rock(Term, Noun),
    Sand(Term, Noun),
    Tell(Vec<Hoon>),
    Tune(Term),
    Wing(WingType),
    Yell(Vec<Hoon>),
    Xray(Manx),
    BarBuc(Vec<Term>, Box<Spec>),
    BarCab(Box<Spec>, Alas, HashMap<Term, Tome>),
    BarCol(Box<Hoon>, Box<Hoon>),
    BarCen(Option<Term>, HashMap<Term, Tome>),
    BarDot(Box<Hoon>),
    BarKet(Box<Hoon>, HashMap<Term, Tome>),
    BarHep(Box<Hoon>),
    BarSig(Box<Spec>, Box<Hoon>),
    BarTar(Box<Spec>, Box<Hoon>),
    BarTis(Box<Spec>, Box<Hoon>),
    BarPat(Option<Term>, HashMap<Term, Tome>),
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
    Centar(WingType, Box<Hoon>, Vec<(WingType, Hoon)>),
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
    KetTisSkin(Skin, Box<Hoon>),
    KetTis(Box<Hoon>, Box<Hoon>),
    KetWut(Box<Hoon>),
    KetTar(Box<Spec>),
    KetCol(Box<Spec>),
    SigBar(Box<Hoon>, Box<Hoon>),
    SigCab(Box<Hoon>, Box<Hoon>),
    SigCen(Chum, Box<Hoon>, Tyre, Box<Hoon>),
    SigFas(Chum, Box<Hoon>),
    SigGal(TermOrPair, Box<Hoon>),
    SigGar(TermOrPair, Box<Hoon>),
    SigBuc(Term, Box<Hoon>),
    SigLus(u64, Box<Hoon>),
    SigPam(u32, Box<Hoon>, Box<Hoon>),
    SigTis(Box<Hoon>, Box<Hoon>),
    SigWut(u32, Box<Hoon>, Box<Hoon>, Box<Hoon>),
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
    // TisMic(Skin, Box<Hoon>, Box<Hoon>),
    // TisFas(Box<Hoon>, Box<Hoon>, Box<Hoon>),
    TisMic(Box<Hoon>, Box<Hoon>, Box<Hoon>),
    TisDot(WingType, Box<Hoon>, Box<Hoon>),
    TisWut(WingType, Box<Hoon>, Box<Hoon>, Box<Hoon>),
    TisGal(Box<Hoon>, Box<Hoon>),
    TisHep(Box<Hoon>, Box<Hoon>),
    TisGar(Box<Hoon>, Box<Hoon>),
    // TisKet(Box<Hoon>, WingType, Box<Hoon>, Box<Hoon>),
    TisKet(Skin, WingType, Box<Hoon>, Box<Hoon>),
    TisLus(Box<Hoon>, Box<Hoon>),
    TisSig(Vec<Hoon>),
    TisTar((Term, Option<Box<Spec>>), Box<Hoon>, Box<Hoon>),
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