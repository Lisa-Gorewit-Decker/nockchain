use std::collections::{BTreeSet, HashMap};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, Once, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};
use std::{fs, io};

use chumsky::Parser;
use hoonc::{
    build_jam, build_jam_with_primed_parse_cache, initialize_with_default_cli,
    is_valid_file_or_dir, prime_parse_cache_public,
};
use nockapp::noun::slab::NounSlab;
use parser::ast::hoon as ast;
use parser::native_parser;
use parser::utils::{hoon_to_noun, LineMap};
use rayon::prelude::*;
use walkdir::WalkDir;

const KERNEL_ENTRIES: &[&str] = &[
    "apps/dumbnet/outer.hoon", "apps/wallet/wallet.hoon", "apps/dumbnet/miner.hoon",
    "apps/peek/peek.hoon", "apps/bridge/bridge.hoon",
];

const HOON_DIR: &str = "../../hoon";
const MARKDOWN_HOON: &str = "../../hoon/common/markdown/markdown.hoon";

static DISABLE_METRICS: Once = Once::new();
static LOG_CAPTURE: OnceLock<LogCapture> = OnceLock::new();

fn disable_metrics() {
    DISABLE_METRICS.call_once(|| {
        std::env::set_var("NOCKAPP_DISABLE_METRICS", "1");
    });
}

struct LogCapture {
    buffer: Arc<Mutex<Vec<u8>>>,
}

impl LogCapture {
    fn clear(&self) {
        if let Ok(mut buffer) = self.buffer.lock() {
            buffer.clear();
        }
    }

    fn take_string(&self) -> String {
        let mut buffer = self.buffer.lock().unwrap_or_else(|err| err.into_inner());
        let output = String::from_utf8_lossy(&buffer).to_string();
        buffer.clear();
        output
    }
}

struct TeeWriter {
    buffer: Arc<Mutex<Vec<u8>>>,
    tee_stdout: bool,
}

impl Write for TeeWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if let Ok(mut buffer) = self.buffer.lock() {
            buffer.extend_from_slice(buf);
        }
        if self.tee_stdout {
            let mut stdout = io::stdout().lock();
            stdout.write_all(buf)?;
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        if self.tee_stdout {
            io::stdout().lock().flush()?;
        }
        Ok(())
    }
}

fn init_logging(tee_stdout: bool) -> &'static LogCapture {
    LOG_CAPTURE.get_or_init(|| {
        let buffer = Arc::new(Mutex::new(Vec::new()));
        let writer_buffer = Arc::clone(&buffer);
        let make_writer = move || TeeWriter {
            buffer: Arc::clone(&writer_buffer),
            tee_stdout,
        };
        if let Err(err) = tracing_subscriber::fmt()
            .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
            .with_target(false)
            .with_writer(make_writer)
            .try_init()
        {
            eprintln!("test: failed to install tracing subscriber: {err}");
        }
        LogCapture { buffer }
    })
}

struct EnvVarGuard {
    key: &'static str,
    previous: Option<String>,
}

impl EnvVarGuard {
    fn set(key: &'static str, value: &str) -> Self {
        let previous = std::env::var(key).ok();
        std::env::set_var(key, value);
        Self { key, previous }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        match &self.previous {
            Some(value) => std::env::set_var(self.key, value),
            None => std::env::remove_var(self.key),
        }
    }
}

fn temp_out_dir(prefix: &str) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let mut path = std::env::temp_dir();
    path.push(format!("{prefix}-{}-{nanos}", std::process::id()));
    fs::create_dir_all(&path)?;
    Ok(path)
}

fn repo_hoon_dir() -> Result<PathBuf, Box<dyn std::error::Error>> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(HOON_DIR);
    Ok(path.canonicalize()?)
}

fn markdown_hoon_path() -> Result<PathBuf, Box<dyn std::error::Error>> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(MARKDOWN_HOON);
    Ok(path.canonicalize()?)
}

fn list_hoon_files(root: &Path) -> Result<Vec<PathBuf>, Box<dyn std::error::Error>> {
    let walker = WalkDir::new(root).follow_links(true).into_iter();
    let mut files = Vec::new();
    for entry_result in walker.filter_entry(is_valid_file_or_dir) {
        let entry = entry_result?;
        if !entry.file_type().is_file() {
            continue;
        }
        if entry.path().extension().and_then(|ext| ext.to_str()) != Some("hoon") {
            continue;
        }
        files.push(entry.path().canonicalize()?);
    }
    Ok(files)
}

fn parse_all_hoon_files(root: &Path) -> Result<Vec<(PathBuf, String)>, Box<dyn std::error::Error>> {
    let files = list_hoon_files(root)?;
    let failures: Vec<(PathBuf, String)> = files
        .par_iter()
        .filter_map(|path| match parse_native_ast_err(path) {
            Ok(_) => None,
            Err(err) => Some((path.clone(), err)),
        })
        .collect();
    Ok(failures)
}

fn ensure_entry_ast(
    asts: &mut HashMap<PathBuf, Vec<u8>>,
    entry: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let entry_path = entry.canonicalize()?;
    if asts.contains_key(&entry_path) {
        return Ok(());
    }

    let jammed = parse_native_ast(entry)?;
    asts.insert(entry_path, jammed);
    Ok(())
}

fn parse_native_ast(path: &Path) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let source = fs::read_to_string(path)?;
    let linemap = Arc::new(LineMap::new(&source));
    let wer = path
        .iter()
        .map(|s| s.to_string_lossy().into_owned())
        .collect();
    let parsed = native_parser(wer, false, linemap)
        .parse(source.as_str())
        .into_result()
        .map_err(|errs| {
            io::Error::new(
                io::ErrorKind::Other,
                format!("native parser failed for {}: {:?}", path.display(), errs),
            )
        })?;

    let mut slab = NounSlab::new();
    let noun = hoon_to_noun(&mut slab, &parsed);
    slab.set_root(noun);
    Ok(slab.jam().to_vec())
}

fn parse_native_ast_err(path: &Path) -> Result<Vec<u8>, String> {
    parse_native_ast(path).map_err(|err| err.to_string())
}

fn collect_native_asts(
    deps_dir: &Path,
    entry: &Path,
) -> Result<HashMap<PathBuf, Vec<u8>>, Box<dyn std::error::Error>> {
    let mut asts = HashMap::new();

    let walker = WalkDir::new(deps_dir).follow_links(true).into_iter();
    for entry_result in walker.filter_entry(is_valid_file_or_dir) {
        let entry = entry_result?;
        if !entry.file_type().is_file() {
            continue;
        }
        if entry.path().extension().and_then(|ext| ext.to_str()) != Some("hoon") {
            continue;
        }

        let jammed = match parse_native_ast(entry.path()) {
            Ok(jammed) => jammed,
            Err(_) => continue,
        };
        asts.insert(entry.path().canonicalize()?, jammed);
    }

    ensure_entry_ast(&mut asts, entry)?;

    Ok(asts)
}

struct PrimeAttempt {
    warned: bool,
    pc_size: Option<usize>,
}

fn parse_pc_size(logs: &str) -> Option<usize> {
    let marker = "prime-dir: pc-size ";
    logs.lines().find_map(|line| {
        let idx = line.find(marker)?;
        let rest = &line[idx + marker.len()..];
        let number = rest.split_whitespace().next()?.trim_matches('"');
        number.parse().ok()
    })
}

fn native_subset_for_paths(
    entry: &PathBuf,
    native_asts: &HashMap<PathBuf, Vec<u8>>,
    paths: &[PathBuf],
) -> Result<HashMap<PathBuf, Vec<u8>>, Box<dyn std::error::Error>> {
    let entry_path = entry.canonicalize()?;
    let entry_jam = native_asts.get(&entry_path).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            format!("native AST missing for entry {}", entry.display()),
        )
    })?;

    let mut subset = HashMap::new();
    subset.insert(entry_path, entry_jam.clone());
    for path in paths {
        let jam = native_asts.get(path).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                format!("native AST missing for {}", path.display()),
            )
        })?;
        subset.insert(path.clone(), jam.clone());
    }
    Ok(subset)
}

fn native_subset_for_prefix(
    entry: &PathBuf,
    native_asts: &HashMap<PathBuf, Vec<u8>>,
    sorted_paths: &[PathBuf],
    prefix_len: usize,
) -> Result<HashMap<PathBuf, Vec<u8>>, Box<dyn std::error::Error>> {
    let subset_paths = &sorted_paths[..prefix_len];
    native_subset_for_paths(entry, native_asts, subset_paths)
}

async fn prime_with_subset(
    entry: &PathBuf,
    deps_dir: &PathBuf,
    subset: &HashMap<PathBuf, Vec<u8>>,
    log_capture: &LogCapture,
) -> Result<PrimeAttempt, Box<dyn std::error::Error>> {
    let nockapp_home = temp_out_dir("bisect-home")?;
    let out_dir = temp_out_dir("bisect-out")?;
    let _env_guard = EnvVarGuard::set("NOCKAPP_HOME", nockapp_home.to_string_lossy().as_ref());
    let _prewarm_guard = EnvVarGuard::set("HOONC_DISABLE_PREWARM", "1");

    log_capture.clear();
    let (mut nockapp, _out_path) =
        initialize_with_default_cli(entry.clone(), deps_dir.clone(), Some(out_dir), false, true)
            .await
            .map_err(|err| {
                io::Error::new(io::ErrorKind::Other, format!("bisect init failed: {err}"))
            })?;
    let prime_result = prime_parse_cache_public(&mut nockapp, entry, deps_dir, subset).await;
    let logs = log_capture.take_string();
    if let Err(err) = prime_result {
        if !logs.is_empty() {
            println!("prime logs:\n{logs}");
        }
        return Err(
            io::Error::new(io::ErrorKind::Other, format!("bisect prime failed: {err}")).into(),
        );
    }

    let warned = logs.contains("hoonc: warning: input is not a proper cause");
    let pc_size = parse_pc_size(&logs);
    Ok(PrimeAttempt { warned, pc_size })
}

async fn bisect_first_failing_path(
    entry: &PathBuf,
    deps_dir: &PathBuf,
    native_asts: &HashMap<PathBuf, Vec<u8>>,
    log_capture: &LogCapture,
) -> Result<Option<PathBuf>, Box<dyn std::error::Error>> {
    let entry_path = entry.canonicalize()?;
    let mut paths: Vec<PathBuf> = native_asts
        .keys()
        .filter(|path| **path != entry_path)
        .cloned()
        .collect();
    paths.sort_by(|a, b| a.to_string_lossy().cmp(&b.to_string_lossy()));

    let full_attempt = {
        let subset = native_subset_for_prefix(entry, native_asts, &paths, paths.len())?;
        prime_with_subset(entry, deps_dir, &subset, log_capture).await?
    };
    println!(
        "bisect: full set warned={}, pc-size={:?}",
        full_attempt.warned, full_attempt.pc_size
    );
    if !full_attempt.warned {
        return Ok(None);
    }

    let mut low = 0usize;
    let mut high = paths.len();
    while low < high {
        let mid = (low + high) / 2;
        let subset = native_subset_for_prefix(entry, native_asts, &paths, mid)?;
        let attempt = prime_with_subset(entry, deps_dir, &subset, log_capture).await?;
        println!(
            "bisect: prefix={} warned={} pc-size={:?}",
            mid, attempt.warned, attempt.pc_size
        );
        if attempt.warned {
            high = mid;
        } else {
            low = mid + 1;
        }
    }

    let failing_path = if low == 0 {
        entry_path
    } else {
        paths[low - 1].clone()
    };
    Ok(Some(failing_path))
}

fn resolve_hoon_path(deps_dir: &Path, path: &str) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let raw = PathBuf::from(path);
    let resolved = if raw.is_absolute() {
        raw
    } else {
        deps_dir.join(raw)
    };
    Ok(resolved.canonicalize()?)
}

fn parse_native_hoon(path: &Path) -> Result<ast::Hoon, Box<dyn std::error::Error>> {
    let source = fs::read_to_string(path)?;
    let linemap = Arc::new(LineMap::new(&source));
    let wer = path
        .iter()
        .map(|s| s.to_string_lossy().into_owned())
        .collect();
    let parsed = native_parser(wer, false, linemap)
        .parse(source.as_str())
        .into_result()
        .map_err(|errs| {
            io::Error::new(
                io::ErrorKind::Other,
                format!("native parser failed for {}: {:?}", path.display(), errs),
            )
        })?;
    Ok(parsed)
}

fn hoon_variant_label(hoon: &ast::Hoon) -> String {
    let debug = format!("{hoon:?}");
    let end = debug
        .find(|c: char| c == '(' || c == '{' || c == '[')
        .unwrap_or_else(|| debug.len());
    debug[..end].to_string()
}

fn collect_hoon_variants(hoon: &ast::Hoon, counts: &mut HashMap<String, usize>) {
    use ast::Hoon::*;

    let label = hoon_variant_label(hoon);
    *counts.entry(label).or_insert(0) += 1;

    match hoon {
        Pair(a, b) => {
            collect_hoon_variants(a, counts);
            collect_hoon_variants(b, counts);
        }
        ZapZap
        | Axis(_)
        | Base(_)
        | Bust(_)
        | Eror(_)
        | Leaf(_, _)
        | Limb(_)
        | Rock(_, _)
        | Sand(_, _)
        | Wing(_) => {}
        Dbug(_, h)
        | Note(_, h)
        | Fits(h, _)
        | Lost(h)
        | BarDot(h)
        | BarHep(h)
        | BarWut(h)
        | DotLus(h)
        | DotWut(h)
        | KetBar(h)
        | KetPam(h)
        | KetSig(h)
        | KetWut(h)
        | SigBuc(_, h)
        | SigLus(_, h)
        | SigFas(_, h)
        | MicFas(h)
        | WutZap(h)
        | ZapGar(h)
        | ZapTis(h)
        | ZapWut(_, h) => {
            collect_hoon_variants(h, counts);
        }
        Hand(typ, _) => collect_hoon_variants_in_type(typ, counts),
        Knit(woofs) => {
            for woof in woofs {
                collect_hoon_variants_in_woof(woof, counts);
            }
        }
        Tell(hoons) | Yell(hoons) | ColSig(hoons) | ColTar(hoons) | TisSig(hoons)
        | WutBar(hoons) | WutPam(hoons) => {
            for h in hoons {
                collect_hoon_variants(h, counts);
            }
        }
        Tune(term_or_tune) => collect_hoon_variants_in_term_or_tune(term_or_tune, counts),
        Xray(manx) => collect_hoon_variants_in_manx(manx, counts),
        BarBuc(_, spec) | KetTar(spec) | KetCol(spec) => {
            collect_hoon_variants_in_spec(spec, counts);
        }
        BarCab(spec, alas, tomes) => {
            collect_hoon_variants_in_spec(spec, counts);
            collect_hoon_variants_in_alas(alas, counts);
            collect_hoon_variants_in_tomes(tomes, counts);
        }
        BarCol(a, b)
        | CenDot(a, b)
        | CenHep(a, b)
        | ColCab(a, b)
        | ColHep(a, b)
        | DotTar(a, b)
        | DotTis(a, b)
        | KetDot(a, b)
        | KetLus(a, b)
        | SigBar(a, b)
        | SigCab(a, b)
        | SigPam(_, a, b)
        | SigTis(a, b)
        | SigZap(a, b)
        | TisDot(_, a, b)
        | TisGal(a, b)
        | TisHep(a, b)
        | TisGar(a, b)
        | TisLus(a, b)
        | TisCom(a, b)
        | WutKet(_, a, b)
        | WutGal(a, b)
        | WutGar(a, b)
        | WutPat(_, a, b)
        | WutSig(_, a, b)
        | ZapCom(a, b)
        | ZapMic(a, b)
        | ZapPat(_, a, b) => {
            collect_hoon_variants(a, counts);
            collect_hoon_variants(b, counts);
        }
        BarCen(_, tomes) | BarPat(_, tomes) => {
            collect_hoon_variants_in_tomes(tomes, counts);
        }
        BarKet(h, tomes) => {
            collect_hoon_variants(h, counts);
            collect_hoon_variants_in_tomes(tomes, counts);
        }
        BarSig(spec, h)
        | BarTar(spec, h)
        | BarTis(spec, h)
        | DotKet(spec, h)
        | KetHep(spec, h)
        | MicMic(spec, h)
        | TisBar(spec, h)
        | ZapGal(spec, h) => {
            collect_hoon_variants_in_spec(spec, counts);
            collect_hoon_variants(h, counts);
        }
        ColKet(a, b, c, d) | CenKet(a, b, c, d) => {
            collect_hoon_variants(a, counts);
            collect_hoon_variants(b, counts);
            collect_hoon_variants(c, counts);
            collect_hoon_variants(d, counts);
        }
        ColLus(a, b, c)
        | CenLus(a, b, c)
        | WutCol(a, b, c)
        | WutDot(a, b, c)
        | SigWut(_, a, b, c)
        | TisWut(_, a, b, c) => {
            collect_hoon_variants(a, counts);
            collect_hoon_variants(b, counts);
            collect_hoon_variants(c, counts);
        }
        CenCab(_, items) | CenTis(_, items) => {
            for (_, h) in items {
                collect_hoon_variants(h, counts);
            }
        }
        CenCol(a, hoons) | CenSig(_, a, hoons) | MicSig(a, hoons) | MicCol(a, hoons) => {
            collect_hoon_variants(a, counts);
            for h in hoons {
                collect_hoon_variants(h, counts);
            }
        }
        CenTar(_, h, items) => {
            collect_hoon_variants(h, counts);
            for (_, item) in items {
                collect_hoon_variants(item, counts);
            }
        }
        KetTis(skin, h) => {
            collect_hoon_variants_in_skin(skin, counts);
            collect_hoon_variants(h, counts);
        }
        SigCen(_, a, tyre, b) => {
            collect_hoon_variants(a, counts);
            collect_hoon_variants_in_tyre(tyre, counts);
            collect_hoon_variants(b, counts);
        }
        SigGal(term_or_pair, h) | SigGar(term_or_pair, h) => {
            collect_hoon_variants_in_term_or_pair(term_or_pair, counts);
            collect_hoon_variants(h, counts);
        }
        MicTis(marl) => collect_hoon_variants_in_marl(marl, counts),
        MicGal(spec, a, b, c) => {
            collect_hoon_variants_in_spec(spec, counts);
            collect_hoon_variants(a, counts);
            collect_hoon_variants(b, counts);
            collect_hoon_variants(c, counts);
        }
        TisCol(items, h) => {
            for (_, item) in items {
                collect_hoon_variants(item, counts);
            }
            collect_hoon_variants(h, counts);
        }
        TisFas(skin, a, b) | TisMic(skin, a, b) => {
            collect_hoon_variants_in_skin(skin, counts);
            collect_hoon_variants(a, counts);
            collect_hoon_variants(b, counts);
        }
        TisKet(skin, _, a, b) => {
            collect_hoon_variants_in_skin(skin, counts);
            collect_hoon_variants(a, counts);
            collect_hoon_variants(b, counts);
        }
        TisTar((_, spec), a, b) => {
            if let Some(spec) = spec {
                collect_hoon_variants_in_spec(spec, counts);
            }
            collect_hoon_variants(a, counts);
            collect_hoon_variants(b, counts);
        }
        WutHep(_, pairs) => {
            for (spec, h) in pairs {
                collect_hoon_variants_in_spec(spec, counts);
                collect_hoon_variants(h, counts);
            }
        }
        WutLus(_, h, pairs) => {
            collect_hoon_variants(h, counts);
            for (spec, item) in pairs {
                collect_hoon_variants_in_spec(spec, counts);
                collect_hoon_variants(item, counts);
            }
        }
        WutHax(skin, _) => collect_hoon_variants_in_skin(skin, counts),
        WutTis(spec, _) => collect_hoon_variants_in_spec(spec, counts),
    }
}

fn collect_hoon_variants_in_alas(alas: &ast::Alas, counts: &mut HashMap<String, usize>) {
    for (_, h) in alas {
        collect_hoon_variants(h, counts);
    }
}

fn collect_hoon_variants_in_tomes(
    tomes: &HashMap<String, ast::Tome>,
    counts: &mut HashMap<String, usize>,
) {
    for tome in tomes.values() {
        collect_hoon_variants_in_tome(tome, counts);
    }
}

fn collect_hoon_variants_in_tome(tome: &ast::Tome, counts: &mut HashMap<String, usize>) {
    for hoon in tome.1.values() {
        collect_hoon_variants(hoon, counts);
    }
}

fn collect_hoon_variants_in_tyre(tyre: &ast::Tyre, counts: &mut HashMap<String, usize>) {
    for (_, hoon) in tyre {
        collect_hoon_variants(hoon, counts);
    }
}

fn collect_hoon_variants_in_term_or_tune(
    term_or_tune: &ast::TermOrTune,
    counts: &mut HashMap<String, usize>,
) {
    match term_or_tune {
        ast::TermOrTune::Term(_) => {}
        ast::TermOrTune::Tune(tune) => collect_hoon_variants_in_tune(tune, counts),
    }
}

fn collect_hoon_variants_in_tune(tune: &ast::Tune, counts: &mut HashMap<String, usize>) {
    for hoon in tune.0.values().flatten() {
        collect_hoon_variants(hoon, counts);
    }
    for hoon in &tune.1 {
        collect_hoon_variants(hoon, counts);
    }
}

fn collect_hoon_variants_in_term_or_pair(
    term_or_pair: &ast::TermOrPair,
    counts: &mut HashMap<String, usize>,
) {
    if let ast::TermOrPair::Pair(_, hoon) = term_or_pair {
        collect_hoon_variants(hoon, counts);
    }
}

fn collect_hoon_variants_in_spec(spec: &ast::Spec, counts: &mut HashMap<String, usize>) {
    use ast::Spec::*;

    match spec {
        Base(_) | Leaf(_, _) | Like(_, _) | Loop(_) => {}
        Dbug(_, spec) | Made(_, spec) | Name(_, spec) | Over(_, spec) | BucLus(_, spec) => {
            collect_hoon_variants_in_spec(spec, counts)
        }
        Make(hoon, specs) => {
            collect_hoon_variants(hoon, counts);
            for spec in specs {
                collect_hoon_variants_in_spec(spec, counts);
            }
        }
        BucGar(a, b) | BucGal(a, b) | BucHep(a, b) | BucKet(a, b) | BucPat(a, b) => {
            collect_hoon_variants_in_spec(a, counts);
            collect_hoon_variants_in_spec(b, counts);
        }
        BucBuc(spec, map)
        | BucDot(spec, map)
        | BucFas(spec, map)
        | BucTic(spec, map)
        | BucZap(spec, map) => {
            collect_hoon_variants_in_spec(spec, counts);
            for spec in map.values() {
                collect_hoon_variants_in_spec(spec, counts);
            }
        }
        BucBar(spec, hoon) | BucPam(spec, hoon) => {
            collect_hoon_variants_in_spec(spec, counts);
            collect_hoon_variants(hoon, counts);
        }
        BucCab(hoon) | BucMic(hoon) => collect_hoon_variants(hoon, counts),
        BucCol(spec, specs) | BucCen(spec, specs) | BucWut(spec, specs) => {
            collect_hoon_variants_in_spec(spec, counts);
            for spec in specs {
                collect_hoon_variants_in_spec(spec, counts);
            }
        }
        BucSig(hoon, spec) => {
            collect_hoon_variants(hoon, counts);
            collect_hoon_variants_in_spec(spec, counts);
        }
        BucTis(skin, spec) => {
            collect_hoon_variants_in_skin(skin, counts);
            collect_hoon_variants_in_spec(spec, counts);
        }
    }
}

fn collect_hoon_variants_in_skin(skin: &ast::Skin, counts: &mut HashMap<String, usize>) {
    use ast::Skin::*;

    match skin {
        Term(_) | Base(_) | Leaf(_, _) | Wash(_) => {}
        Cell(a, b) => {
            collect_hoon_variants_in_skin(a, counts);
            collect_hoon_variants_in_skin(b, counts);
        }
        Dbug(_, skin) | Name(_, skin) | Over(_, skin) => {
            collect_hoon_variants_in_skin(skin, counts);
        }
        Spec(spec, skin) => {
            collect_hoon_variants_in_spec(spec, counts);
            collect_hoon_variants_in_skin(skin, counts);
        }
    }
}

fn collect_hoon_variants_in_type(typ: &ast::Type, counts: &mut HashMap<String, usize>) {
    use ast::Type::*;

    match typ {
        NounExpr | Void | ParsedAtom(_, _) => {}
        Cell(a, b) => {
            collect_hoon_variants_in_type(a, counts);
            collect_hoon_variants_in_type(b, counts);
        }
        Core(typ, coil) => {
            collect_hoon_variants_in_type(typ, counts);
            collect_hoon_variants_in_coil(coil, counts);
        }
        Face(_, typ) => collect_hoon_variants_in_type(typ, counts),
        Fork(types) => {
            for typ in types {
                collect_hoon_variants_in_type(typ, counts);
            }
        }
        Hint((typ, _), other) => {
            collect_hoon_variants_in_type(typ, counts);
            collect_hoon_variants_in_type(other, counts);
        }
        Hold(typ, hoon) => {
            collect_hoon_variants_in_type(typ, counts);
            collect_hoon_variants(hoon, counts);
        }
    }
}

fn collect_hoon_variants_in_coil(coil: &ast::Coil, counts: &mut HashMap<String, usize>) {
    collect_hoon_variants_in_type(&coil.q, counts);
    collect_hoon_variants_in_semi_noun_expr(&coil.r.0, counts);
    for tome in coil.r.1.values() {
        collect_hoon_variants_in_tome(tome, counts);
    }
}

fn collect_hoon_variants_in_semi_noun_expr(
    expr: &ast::SemiNounExpr,
    counts: &mut HashMap<String, usize>,
) {
    collect_hoon_variants_in_stencil(&expr.0, counts);
}

fn collect_hoon_variants_in_stencil(stencil: &ast::Stencil, counts: &mut HashMap<String, usize>) {
    match stencil {
        ast::Stencil::Half { left, rite } => {
            collect_hoon_variants_in_stencil(left, counts);
            collect_hoon_variants_in_stencil(rite, counts);
        }
        ast::Stencil::Full { blocks: _ } => {}
        ast::Stencil::Lazy { resolve, .. } => {
            collect_hoon_variants_in_spec(&resolve.0, counts);
            collect_hoon_variants_in_spec(&resolve.1, counts);
        }
    }
}

fn collect_hoon_variants_in_manx(manx: &ast::Manx, counts: &mut HashMap<String, usize>) {
    collect_hoon_variants_in_marx(&manx.g, counts);
    collect_hoon_variants_in_marl(&manx.c, counts);
}

fn collect_hoon_variants_in_marx(marx: &ast::Marx, counts: &mut HashMap<String, usize>) {
    collect_hoon_variants_in_mart(&marx.a, counts);
}

fn collect_hoon_variants_in_mart(mart: &ast::Mart, counts: &mut HashMap<String, usize>) {
    for (_, beers) in mart {
        for beer in beers {
            collect_hoon_variants_in_beer(beer, counts);
        }
    }
}

fn collect_hoon_variants_in_beer(beer: &ast::Beer, counts: &mut HashMap<String, usize>) {
    if let ast::Beer::Hoon(hoon) = beer {
        collect_hoon_variants(hoon, counts);
    }
}

fn collect_hoon_variants_in_woof(woof: &ast::Woof, counts: &mut HashMap<String, usize>) {
    if let ast::Woof::Hoon(hoon) = woof {
        collect_hoon_variants(hoon, counts);
    }
}

fn collect_hoon_variants_in_marl(marl: &ast::Marl, counts: &mut HashMap<String, usize>) {
    for tuna in marl {
        collect_hoon_variants_in_tuna(tuna, counts);
    }
}

fn collect_hoon_variants_in_tuna(tuna: &ast::Tuna, counts: &mut HashMap<String, usize>) {
    match tuna {
        ast::Tuna::Manx(manx) => collect_hoon_variants_in_manx(manx, counts),
        ast::Tuna::TunaTail(tail) => collect_hoon_variants_in_tuna_tail(tail, counts),
    }
}

fn collect_hoon_variants_in_tuna_tail(tail: &ast::TunaTail, counts: &mut HashMap<String, usize>) {
    match tail {
        ast::TunaTail::Tape(hoon)
        | ast::TunaTail::Manx(hoon)
        | ast::TunaTail::Marl(hoon)
        | ast::TunaTail::Call(hoon) => collect_hoon_variants(hoon, counts),
    }
}

fn inventory_hoon_variants(
    path: &Path,
) -> Result<HashMap<String, usize>, Box<dyn std::error::Error>> {
    let hoon = parse_native_hoon(path)?;
    let mut counts = HashMap::new();
    collect_hoon_variants(&hoon, &mut counts);
    Ok(counts)
}

fn sorted_variant_diffs(
    base: &HashMap<String, usize>,
    target: &HashMap<String, usize>,
) -> Vec<(String, i64, usize, usize)> {
    let mut keys = BTreeSet::new();
    keys.extend(base.keys().cloned());
    keys.extend(target.keys().cloned());

    let mut diffs = Vec::new();
    for key in keys {
        let base_count = base.get(&key).copied().unwrap_or(0);
        let target_count = target.get(&key).copied().unwrap_or(0);
        if base_count != target_count {
            let diff = target_count as i64 - base_count as i64;
            diffs.push((key, diff, base_count, target_count));
        }
    }

    diffs.sort_by(|a, b| b.1.abs().cmp(&a.1.abs()).then_with(|| a.0.cmp(&b.0)));
    diffs
}

fn kernel_entries(deps_dir: &Path) -> Result<Vec<PathBuf>, Box<dyn std::error::Error>> {
    KERNEL_ENTRIES
        .iter()
        .map(|entry| Ok(deps_dir.join(entry).canonicalize()?))
        .collect()
}

#[tokio::test]
async fn primed_parse_cache_matches_regular_build() -> Result<(), Box<dyn std::error::Error>> {
    disable_metrics();
    let nockapp_home = temp_out_dir("nockapp-home")?;
    let _env_guard = EnvVarGuard::set("NOCKAPP_HOME", nockapp_home.to_string_lossy().as_ref());

    let entry = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../hoonc/hoon/hoon-138.hoon");
    let deps_dir = repo_hoon_dir()?;
    let entry = entry.canonicalize()?;

    let native_asts = collect_native_asts(&deps_dir, &entry)?;
    let regular_out_dir = temp_out_dir("hoonc-regular")?;
    let primed_out_dir = temp_out_dir("hoonc-primed")?;

    let regular_jam = build_jam(
        &entry,
        deps_dir.clone(),
        Some(regular_out_dir.clone()),
        true,
        true,
    )
    .await
    .map_err(|err| {
        io::Error::new(
            io::ErrorKind::Other,
            format!(
                "regular build failed (out_dir: {}): {err}",
                regular_out_dir.display()
            ),
        )
    })?;
    let primed_jam = build_jam_with_primed_parse_cache(
        entry,
        deps_dir,
        Some(primed_out_dir.clone()),
        true,
        true,
        &native_asts,
    )
    .await
    .map_err(|err| {
        io::Error::new(
            io::ErrorKind::Other,
            format!(
                "primed build failed (out_dir: {}): {err}",
                primed_out_dir.display()
            ),
        )
    })?;

    assert_eq!(regular_jam, primed_jam);
    Ok(())
}

#[test]
fn native_parser_parses_markdown_hoon() -> Result<(), Box<dyn std::error::Error>> {
    let path = markdown_hoon_path()?;
    parse_native_ast(&path)?;
    Ok(())
}

#[test]
fn native_parser_parses_all_hoon_files() -> Result<(), Box<dyn std::error::Error>> {
    let root = repo_hoon_dir()?;
    let failures = parse_all_hoon_files(&root)?;

    if failures.is_empty() {
        return Ok(());
    }

    let mut report = String::new();
    report.push_str("native parser failures:\n");
    for (path, error) in failures {
        report.push_str(&format!("- {}\n    {error}\n", path.display()));
    }
    Err(io::Error::new(io::ErrorKind::Other, report).into())
}

#[tokio::test]
async fn primed_parse_cache_matches_regular_build_for_kernels(
) -> Result<(), Box<dyn std::error::Error>> {
    disable_metrics();
    let deps_dir = repo_hoon_dir()?;
    let entries = kernel_entries(&deps_dir)?;
    let first_entry = entries
        .first()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "no kernel entries defined"))?;
    let mut native_asts = collect_native_asts(&deps_dir, first_entry)?;
    let mut tested = Vec::new();

    for entry in entries {
        ensure_entry_ast(&mut native_asts, &entry)?;

        let regular_home = temp_out_dir("nockapp-home-regular")?;
        let primed_home = temp_out_dir("nockapp-home-primed")?;

        let regular_out_dir = temp_out_dir("hoonc-regular")?;
        let primed_out_dir = temp_out_dir("hoonc-primed")?;

        println!("testing kernel: {}", entry.display());

        let regular_jam = {
            let _env_guard =
                EnvVarGuard::set("NOCKAPP_HOME", regular_home.to_string_lossy().as_ref());
            build_jam(
                &entry,
                deps_dir.clone(),
                Some(regular_out_dir.clone()),
                false,
                false,
            )
            .await
            .map_err(|err| {
                io::Error::new(
                    io::ErrorKind::Other,
                    format!(
                        "kernel build failed (entry: {}, out_dir: {}): {err}",
                        entry.display(),
                        regular_out_dir.display()
                    ),
                )
            })?
        };

        let primed_jam = {
            let _env_guard =
                EnvVarGuard::set("NOCKAPP_HOME", primed_home.to_string_lossy().as_ref());
            build_jam_with_primed_parse_cache(
                entry.clone(),
                deps_dir.clone(),
                Some(primed_out_dir.clone()),
                false,
                false,
                &native_asts,
            )
            .await
            .map_err(|err| {
                io::Error::new(
                    io::ErrorKind::Other,
                    format!(
                        "kernel primed build failed (entry: {}, out_dir: {}): {err}",
                        entry.display(),
                        primed_out_dir.display()
                    ),
                )
            })?
        };

        assert_eq!(
            regular_jam,
            primed_jam,
            "kernel jam mismatch for {}",
            entry.display()
        );
        tested.push(entry);
    }

    assert!(
        !tested.is_empty(),
        "no kernel entries eligible for primed parse-cache test"
    );
    Ok(())
}

#[tokio::test]
#[ignore]
async fn bisect_primed_parse_cache_failure() -> Result<(), Box<dyn std::error::Error>> {
    disable_metrics();
    if std::env::var("RUST_LOG").is_err() {
        std::env::set_var("RUST_LOG", "info");
    }
    let log_capture = init_logging(true);

    let deps_dir = repo_hoon_dir()?;
    let entry = deps_dir.join(KERNEL_ENTRIES[0]).canonicalize()?;
    let native_asts = collect_native_asts(&deps_dir, &entry)?;

    let failing = bisect_first_failing_path(&entry, &deps_dir, &native_asts, log_capture).await?;
    match failing {
        Some(path) => println!("first failing path: {}", path.display()),
        None => println!("no failing path found"),
    }
    Ok(())
}

#[tokio::test]
#[ignore]
async fn prime_single_path_debug() -> Result<(), Box<dyn std::error::Error>> {
    disable_metrics();
    if std::env::var("RUST_LOG").is_err() {
        std::env::set_var("RUST_LOG", "info");
    }
    let log_capture = init_logging(true);

    let deps_dir = repo_hoon_dir()?;
    let entry_rel =
        std::env::var("HOONC_PRIME_ENTRY").unwrap_or_else(|_| KERNEL_ENTRIES[0].to_string());
    let entry = resolve_hoon_path(&deps_dir, &entry_rel)?;
    let target_rel = std::env::var("HOONC_PRIME_SINGLE_PATH")
        .unwrap_or_else(|_| "apps/bridge/bridge.hoon".to_string());
    let target = resolve_hoon_path(&deps_dir, &target_rel)?;

    let native_asts = collect_native_asts(&deps_dir, &entry)?;
    let subset = native_subset_for_paths(&entry, &native_asts, std::slice::from_ref(&target))?;
    let attempt = prime_with_subset(&entry, &deps_dir, &subset, log_capture).await?;

    println!(
        "prime single path entry={} target={} warned={} pc-size={:?}",
        entry.display(),
        target.display(),
        attempt.warned,
        attempt.pc_size
    );
    Ok(())
}

#[tokio::test]
#[ignore]
async fn enumerate_failing_primed_paths() -> Result<(), Box<dyn std::error::Error>> {
    disable_metrics();
    if std::env::var("RUST_LOG").is_err() {
        std::env::set_var("RUST_LOG", "info");
    }
    let log_capture = init_logging(true);

    let deps_dir = repo_hoon_dir()?;
    let entry_rel =
        std::env::var("HOONC_PRIME_ENTRY").unwrap_or_else(|_| KERNEL_ENTRIES[0].to_string());
    let entry = resolve_hoon_path(&deps_dir, &entry_rel)?;
    let entry_path = entry.canonicalize()?;

    let native_asts = collect_native_asts(&deps_dir, &entry)?;
    let mut paths: Vec<PathBuf> = native_asts
        .keys()
        .filter(|path| **path != entry_path)
        .cloned()
        .collect();
    paths.sort_by(|a, b| a.to_string_lossy().cmp(&b.to_string_lossy()));

    let mut failing = Vec::new();
    for path in &paths {
        let subset = native_subset_for_paths(&entry, &native_asts, std::slice::from_ref(path))?;
        let attempt = prime_with_subset(&entry, &deps_dir, &subset, log_capture).await?;
        if attempt.warned {
            println!(
                "failing path: {} pc-size={:?}",
                path.display(),
                attempt.pc_size
            );
            failing.push(path.clone());
        }
    }

    println!("failing paths: {}", failing.len());
    Ok(())
}

#[test]
#[ignore]
fn compare_hoon_variant_inventory() -> Result<(), Box<dyn std::error::Error>> {
    let deps_dir = repo_hoon_dir()?;
    let target_rel = std::env::var("HOONC_VARIANT_TARGET")
        .unwrap_or_else(|_| "apps/bridge/bridge.hoon".to_string());
    let base_rel =
        std::env::var("HOONC_VARIANT_BASE").unwrap_or_else(|_| KERNEL_ENTRIES[0].to_string());
    let target = resolve_hoon_path(&deps_dir, &target_rel)?;
    let base = resolve_hoon_path(&deps_dir, &base_rel)?;

    let target_counts = inventory_hoon_variants(&target)?;
    let base_counts = inventory_hoon_variants(&base)?;
    let diffs = sorted_variant_diffs(&base_counts, &target_counts);

    println!("variant inventory target: {}", target.display());
    println!("variant inventory base:   {}", base.display());

    println!("variants only in target:");
    for (name, _, _, target_count) in diffs.iter().filter(|(_, _, b, _)| *b == 0) {
        println!("  {name}: {target_count}");
    }

    println!("variants only in base:");
    for (name, _, base_count, _) in diffs.iter().filter(|(_, _, _, t)| *t == 0) {
        println!("  {name}: {base_count}");
    }

    println!("variant count diffs (target - base):");
    for (name, diff, base_count, target_count) in diffs {
        if base_count != 0 && target_count != 0 {
            println!("  {name}: {diff} (base={base_count}, target={target_count})");
        }
    }

    Ok(())
}
