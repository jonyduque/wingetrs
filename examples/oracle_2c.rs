//! Phase 2c oracle (Task 142): classifies every scoop manifest of a local catalog by
//! sub-case (S-psm, S-inno, S-file, S-ps1, S-hook) and ranks the commands its scripts
//! call; Task 150 adds the static gate verdict (`script_scan`) of every S-hook manifest;
//! Task 163 adds whether each 2c manifest plans (`plan_install`) and the `if ($global)` probe.
//! Reads the catalog read-only; never runs a script. Prints Markdown.
//!
//! Usage: `cargo run --release --example oracle_2c -- <catalog.db>`
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::Path;

use anyhow::Context;
use rusqlite::{Connection, OpenFlags};
use wingetrs::install::classify_2c::{SubCase, classify, script_commands};
use wingetrs::install::layout::Layout;
use wingetrs::install::manifest::{Arch, Resolved, resolve};
use wingetrs::install::planner::{PlanError, PlanInput, plan_install};
use wingetrs::install::prelude::PRELUDE_FUNCTIONS;
use wingetrs::install::script_scan::{FlagKind, SCOOP_FUNCTIONS, Verdict, scan_manifest};

/// Approved verbs (`Get-Verb`, PowerShell 7.6.7): `Verb-Noun` with one of them counts as a
/// cmdlet when it is not a Scoop function.
const APPROVED_VERBS: &[&str] = &[
    "Add",
    "Approve",
    "Assert",
    "Backup",
    "Block",
    "Build",
    "Checkpoint",
    "Clear",
    "Close",
    "Compare",
    "Complete",
    "Compress",
    "Confirm",
    "Connect",
    "Convert",
    "ConvertFrom",
    "ConvertTo",
    "Copy",
    "Debug",
    "Deny",
    "Deploy",
    "Disable",
    "Disconnect",
    "Dismount",
    "Edit",
    "Enable",
    "Enter",
    "Exit",
    "Expand",
    "Export",
    "Find",
    "Format",
    "Get",
    "Grant",
    "Group",
    "Hide",
    "Import",
    "Initialize",
    "Install",
    "Invoke",
    "Join",
    "Limit",
    "Lock",
    "Measure",
    "Merge",
    "Mount",
    "Move",
    "New",
    "Open",
    "Optimize",
    "Out",
    "Ping",
    "Pop",
    "Protect",
    "Publish",
    "Push",
    "Read",
    "Receive",
    "Redo",
    "Register",
    "Remove",
    "Rename",
    "Repair",
    "Request",
    "Reset",
    "Resize",
    "Resolve",
    "Restart",
    "Restore",
    "Resume",
    "Revoke",
    "Save",
    "Search",
    "Select",
    "Send",
    "Set",
    "Show",
    "Skip",
    "Split",
    "Start",
    "Step",
    "Stop",
    "Submit",
    "Suspend",
    "Switch",
    "Sync",
    "Test",
    "Trace",
    "Unblock",
    "Undo",
    "Uninstall",
    "Unlock",
    "Unprotect",
    "Unpublish",
    "Unregister",
    "Update",
    "Use",
    "Wait",
    "Watch",
    "Write",
];

/// Built-in aliases (`pwsh -NoProfile`, `Get-Alias`, PowerShell 7.6.7) and built-in
/// functions without a module (`Get-Command -CommandType Function`).
const PWSH_BUILTINS: &[&str] = &[
    "?",
    "%",
    "ac",
    "cat",
    "cd",
    "chdir",
    "clc",
    "clear",
    "clhy",
    "cli",
    "clp",
    "cls",
    "clv",
    "cnsn",
    "compare",
    "copy",
    "cp",
    "cpi",
    "cpp",
    "cvpa",
    "dbp",
    "del",
    "diff",
    "dir",
    "dnsn",
    "ebp",
    "echo",
    "epal",
    "epcsv",
    "erase",
    "etsn",
    "exsn",
    "fc",
    "fhx",
    "fl",
    "foreach",
    "ft",
    "fw",
    "gal",
    "gbp",
    "gc",
    "gci",
    "gcm",
    "gcs",
    "gdr",
    "gerr",
    "ghy",
    "gi",
    "gjb",
    "gl",
    "gm",
    "gmo",
    "gp",
    "gps",
    "gpv",
    "group",
    "gsn",
    "gsv",
    "gu",
    "gv",
    "h",
    "history",
    "icm",
    "iex",
    "ihy",
    "ii",
    "ipal",
    "ipcsv",
    "ipmo",
    "irm",
    "iwr",
    "kill",
    "ls",
    "man",
    "md",
    "measure",
    "mi",
    "mount",
    "move",
    "mp",
    "mv",
    "nal",
    "ndr",
    "ni",
    "nmo",
    "nsn",
    "nv",
    "ogv",
    "oh",
    "popd",
    "ps",
    "pushd",
    "pwd",
    "r",
    "rbp",
    "rcjb",
    "rcsn",
    "rd",
    "rdr",
    "ren",
    "ri",
    "rjb",
    "rm",
    "rmdir",
    "rmo",
    "rni",
    "rnp",
    "rp",
    "rsn",
    "rv",
    "rvpa",
    "sajb",
    "sal",
    "saps",
    "sasv",
    "sbp",
    "select",
    "set",
    "shcm",
    "si",
    "sl",
    "sleep",
    "sls",
    "sort",
    "sp",
    "spjb",
    "spps",
    "spsv",
    "start",
    "sv",
    "tee",
    "type",
    "where",
    "wjb",
    "write",
    "cd..",
    "Clear-Host",
    "help",
    "mkdir",
    "oss",
    "Pause",
    "prompt",
    "TabExpansion2",
];

/// Cmdlets of the `Microsoft.PowerShell.*` modules whose verb is not approved
/// (`Get-Command -Module Microsoft.PowerShell.*`, PowerShell 7.6.7).
const UNAPPROVED_CMDLETS: &[&str] = &[
    "ForEach-Object",
    "Sort-Object",
    "Tee-Object",
    "Where-Object",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Scoop,
    Cmdlet,
    External,
}

impl Kind {
    fn label(self) -> &'static str {
        match self {
            Kind::Scoop => "função do Scoop",
            Kind::Cmdlet => "cmdlet",
            Kind::External => "externo ou definido no script",
        }
    }
}

/// Kind of a lowercase command name: Scoop function first, then pwsh builtin alias /
/// function or `Verb-Noun` with an approved verb, else external program (or a function
/// defined outside the manifest's scripts).
fn kind(lower: &str, scoop_fns: &HashSet<String>) -> Kind {
    if scoop_fns.contains(lower) {
        return Kind::Scoop;
    }
    if PWSH_BUILTINS
        .iter()
        .chain(UNAPPROVED_CMDLETS)
        .any(|b| b.eq_ignore_ascii_case(lower))
    {
        return Kind::Cmdlet;
    }
    match lower.split_once('-') {
        Some((verb, noun))
            if !noun.is_empty() && APPROVED_VERBS.iter().any(|v| v.eq_ignore_ascii_case(verb)) =>
        {
            Kind::Cmdlet
        }
        _ => Kind::External,
    }
}

#[derive(Default)]
struct Bucket {
    total: usize,
    invalid: usize,
    c2: usize,
    worst: BTreeMap<SubCase, usize>,
    present: BTreeMap<SubCase, usize>,
    no_powershell: usize,
}

/// Static gate tally (Task 150) over the S-hook manifests.
#[derive(Default)]
struct Gate {
    total: usize,
    unresolved: usize,
    /// Index 0: `supported = &[]`; index 1: `supported = SCOOP_FUNCTIONS`.
    by_verdict: [BTreeMap<Verdict, usize>; 2],
    /// Flag kind -> manifests with it (`supported = SCOOP_FUNCTIONS`).
    kinds: BTreeMap<String, usize>,
    /// Manifests whose verdict rises only because of `HiddenCode` flags.
    hidden_changed: [usize; 2],
    /// Manifests whose verdict rises only because of the prelude rules (review of 153).
    prelude_changed: [usize; 2],
}

impl Gate {
    fn add(&mut self, raw: &str) {
        self.total += 1;
        let Ok(m) = resolve(raw, Arch::X64) else {
            self.unresolved += 1;
            return;
        };
        let sets: [&[&str]; 2] = [&[], SCOOP_FUNCTIONS];
        for (i, supported) in sets.into_iter().enumerate() {
            let reports = scan_manifest(&m, supported);
            let verdict = reports
                .iter()
                .map(|(_, r)| r.verdict)
                .max()
                .unwrap_or(Verdict::Confirm);
            *self.by_verdict[i].entry(verdict).or_default() += 1;
            let without = |excluded: &[FlagKind]| {
                reports
                    .iter()
                    .flat_map(|(_, r)| r.flags.iter())
                    .filter(|f| !excluded.contains(&f.kind))
                    .map(|f| f.verdict)
                    .max()
                    .unwrap_or(Verdict::Confirm)
            };
            if without(&[FlagKind::HiddenCode]) != verdict {
                self.hidden_changed[i] += 1;
            }
            let prelude_kinds = [
                FlagKind::PreludeInternal,
                FlagKind::PreludeRedefinition,
                FlagKind::EnvironmentVariable,
            ];
            if without(&prelude_kinds) != verdict {
                self.prelude_changed[i] += 1;
            }
            if i == 1 {
                let kinds: BTreeSet<String> = reports
                    .iter()
                    .flat_map(|(_, r)| r.flags.iter().map(|f| format!("{:?}", f.kind)))
                    .collect();
                for k in kinds {
                    *self.kinds.entry(k).or_default() += 1;
                }
            }
        }
    }

    fn print(&self) {
        let resolved = self.total - self.unresolved;
        println!(
            "\n## Veredito do portão estático (`script_scan`, Task 150)\n\n{} manifestos S-hook; {} não resolvem para x64 e ficam de fora. Veredito do manifesto = o pior entre os seus scripts. Nada foi executado. Última coluna: manifestos cujo veredito sobe só por `HiddenCode` (regra de recusa vista apenas em comentário ou here-string) e só pelas regras do prelúdio (`__wr`, redefinição de função do prelúdio, `SetEnvironmentVariable`).\n",
            self.total, self.unresolved
        );
        println!(
            "| funções do Scoop suportadas | Confirm | ConfirmStrong | Refuse | mudam por HiddenCode | mudam pelas regras do prelúdio |"
        );
        println!("|---|---:|---:|---:|---:|---:|");
        let labels = [
            "nenhuma (`&[]`)".to_string(),
            format!("todas ({})", SCOOP_FUNCTIONS.len()),
        ];
        let changed = self.hidden_changed.into_iter().zip(self.prelude_changed);
        for ((label, tally), (hidden, prelude)) in labels.iter().zip(&self.by_verdict).zip(changed)
        {
            let col = |v: Verdict| pct(tally.get(&v).copied().unwrap_or(0), resolved);
            println!(
                "| {label} | {} | {} | {} | {} | {} |",
                col(Verdict::Confirm),
                col(Verdict::ConfirmStrong),
                col(Verdict::Refuse),
                pct(hidden, resolved),
                pct(prelude, resolved)
            );
        }
        let mut kinds: Vec<(&String, &usize)> = self.kinds.iter().collect();
        kinds.sort_by(|a, b| b.1.cmp(a.1).then_with(|| a.0.cmp(b.0)));
        println!("\n| flag (com todas as funções suportadas) | manifestos |");
        println!("|---|---:|");
        for (k, n) in kinds {
            println!("| {k} | {} |", pct(*n, resolved));
        }
    }
}

/// Outcome of planning one 2c manifest (Task 163).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Outcome {
    Installable,
    GateRefused,
    OtherError,
}

/// "Installable by 2c" tally (Task 163): `resolve(host)` + `plan_install` without error
/// (the planner's gate uses `supported = PRELUDE_FUNCTIONS`).
#[derive(Default)]
struct Installable {
    /// (bucket, worst sub-case) -> outcome -> manifests.
    cells: BTreeMap<(String, SubCase), BTreeMap<Outcome, usize>>,
    /// Refusal detail -> manifests with it (each detail counted once per manifest).
    gate_reasons: BTreeMap<String, usize>,
    /// Refusing flag kind -> manifests with it.
    gate_kinds: BTreeMap<String, usize>,
    /// Grouped non-gate error -> manifests.
    other_reasons: BTreeMap<String, usize>,
    /// Gate-refused manifests that plan once the `if ($global) { ... }` bodies are blanked.
    guard_only: Vec<String>,
    /// Of `guard_only`, those whose every refusing flag is `GlobalSwitch`.
    guard_only_global_switch: Vec<String>,
    /// Of `guard_only`, those that assign `$global` somewhere (the guard could then run).
    guard_only_reassign: Vec<String>,
    /// Gate-refused manifests with at least one `if ($global)` block.
    refused_with_guard: usize,
}

fn plan(app: &str, bucket: &str, m: &Resolved, layout: &Layout) -> Result<(), PlanError> {
    plan_install(&PlanInput {
        app,
        bucket,
        manifest: m,
        layout,
        shortcut_root: Path::new(r"C:\oracle\Start Menu\wingetrs Apps"),
        installed: None,
        launcher_present: false,
        keep_old: false,
        manifest_sha256: "",
    })
    .map(|_| ())
}

/// Stable group of a non-gate planning error (drops per-manifest detail after 60 chars).
fn group_error(e: &PlanError) -> String {
    match e {
        PlanError::UnsafePath(_) => "caminho inseguro no manifesto".into(),
        PlanError::UnsafeArgs(_) => "argumento inseguro no manifesto".into(),
        _ => e.to_string().chars().take(60).collect(),
    }
}

/// The `n` largest entries, ties by key.
fn top(m: &BTreeMap<String, usize>, n: usize) -> Vec<(&String, &usize)> {
    let mut v: Vec<(&String, &usize)> = m.iter().collect();
    v.sort_by(|a, b| b.1.cmp(a.1).then_with(|| a.0.cmp(b.0)));
    v.into_iter().take(n).collect()
}

fn word_char(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

/// Byte ranges of the bodies (inside the braces) of every `if ($global) { ... }` of a
/// script (case-insensitive; `$globalx` and `-not $global` do not match). Braces inside
/// quotes and `#` comments are skipped; here-strings are not (approximation).
fn global_guard_bodies(script: &str) -> Vec<(usize, usize)> {
    let lower = script.to_ascii_lowercase();
    let l = lower.as_bytes();
    let skip_ws = |mut i: usize| {
        while i < l.len() && l[i].is_ascii_whitespace() {
            i += 1;
        }
        i
    };
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(off) = lower[i..].find("if") {
        let at = i + off;
        i = at + 2;
        if at > 0 && (word_char(l[at - 1]) || l[at - 1] == b'-') {
            continue;
        }
        let mut j = skip_ws(at + 2);
        if l.get(j) != Some(&b'(') {
            continue;
        }
        j = skip_ws(j + 1);
        if !lower[j..].starts_with("$global") || l.get(j + 7).is_some_and(|c| word_char(*c)) {
            continue;
        }
        j = skip_ws(j + 7);
        if l.get(j) != Some(&b')') {
            continue;
        }
        j = skip_ws(j + 1);
        if l.get(j) != Some(&b'{') {
            continue;
        }
        let open = j;
        let mut depth = 0usize;
        let mut k = open;
        let mut quote: Option<u8> = None;
        let mut close = None;
        while k < l.len() {
            let c = l[k];
            match quote {
                Some(q) if c == q => quote = None,
                Some(_) => {}
                None => match c {
                    b'\'' | b'"' => quote = Some(c),
                    b'#' => {
                        while k < l.len() && l[k] != b'\n' {
                            k += 1;
                        }
                        continue;
                    }
                    b'{' => depth += 1,
                    b'}' => {
                        depth -= 1;
                        if depth == 0 {
                            close = Some(k);
                            break;
                        }
                    }
                    _ => {}
                },
            }
            k += 1;
        }
        if let Some(close) = close {
            out.push((open + 1, close));
            i = close;
        }
    }
    out
}

/// The script with every `if ($global)` body blanked (newlines kept, so lines do not move).
fn blank_global_guards(script: &str) -> String {
    let mut bytes = script.as_bytes().to_vec();
    for (s, e) in global_guard_bodies(script) {
        for c in &mut bytes[s..e] {
            if *c != b'\n' && *c != b'\r' {
                *c = b' ';
            }
        }
    }
    // Body bounds are ASCII braces, so every byte of a multi-byte char inside is blanked.
    String::from_utf8_lossy(&bytes).into_owned()
}

/// `$global = ...` (not `==`), `$script:global =`, `$local:global =`, `Set-Variable global`.
fn assigns_global(script: &str) -> bool {
    let lower = script.to_ascii_lowercase();
    let compact: String = lower.chars().filter(|c| !c.is_whitespace()).collect();
    let assigns = ["$global=", "$script:global=", "$local:global="]
        .iter()
        .any(|p| {
            compact
                .match_indices(p)
                .any(|(i, _)| !compact[i + p.len()..].starts_with('='))
        });
    assigns || lower.contains("set-variable global") || lower.contains("set-variable -name global")
}

fn scripts_of(m: &Resolved) -> Vec<&str> {
    [
        m.hooks.pre_install.as_deref(),
        m.installer.as_ref().and_then(|i| i.script.as_deref()),
        m.hooks.post_install.as_deref(),
        m.hooks.pre_uninstall.as_deref(),
        m.uninstaller.as_ref().and_then(|i| i.script.as_deref()),
        m.hooks.post_uninstall.as_deref(),
    ]
    .into_iter()
    .flatten()
    .collect()
}

fn blanked(m: &Resolved) -> Resolved {
    let mut m = m.clone();
    let b = |s: &mut Option<String>| {
        if let Some(t) = s {
            *t = blank_global_guards(t);
        }
    };
    b(&mut m.hooks.pre_install);
    b(&mut m.hooks.post_install);
    b(&mut m.hooks.pre_uninstall);
    b(&mut m.hooks.post_uninstall);
    if let Some(i) = &mut m.installer {
        b(&mut i.script);
    }
    if let Some(i) = &mut m.uninstaller {
        b(&mut i.script);
    }
    m
}

impl Installable {
    fn add(&mut self, bucket: &str, worst: SubCase, app: &str, raw: &str, layout: &Layout) {
        let outcome = match resolve(raw, Arch::host()) {
            Err(e) => {
                let g: String = format!("resolve: {e}").chars().take(60).collect();
                *self.other_reasons.entry(g).or_default() += 1;
                Outcome::OtherError
            }
            Ok(m) => match plan(app, bucket, &m, layout) {
                Ok(()) => Outcome::Installable,
                Err(PlanError::ScriptRefused { .. }) => {
                    self.refused(app, bucket, &m, layout);
                    Outcome::GateRefused
                }
                Err(e) => {
                    *self.other_reasons.entry(group_error(&e)).or_default() += 1;
                    Outcome::OtherError
                }
            },
        };
        *self
            .cells
            .entry((bucket.to_string(), worst))
            .or_default()
            .entry(outcome)
            .or_default() += 1;
    }

    fn refused(&mut self, app: &str, bucket: &str, m: &Resolved, layout: &Layout) {
        let reports = scan_manifest(m, PRELUDE_FUNCTIONS);
        let refusing: Vec<_> = reports
            .iter()
            .flat_map(|(_, r)| r.flags.iter())
            .filter(|f| f.verdict == Verdict::Refuse)
            .collect();
        let details: BTreeSet<&str> = refusing.iter().map(|f| f.detail.as_str()).collect();
        for d in details {
            *self.gate_reasons.entry(d.to_string()).or_default() += 1;
        }
        let kinds: BTreeSet<String> = refusing.iter().map(|f| format!("{:?}", f.kind)).collect();
        for k in kinds {
            *self.gate_kinds.entry(k).or_default() += 1;
        }
        let scripts = scripts_of(m);
        if scripts.iter().all(|s| global_guard_bodies(s).is_empty()) {
            return;
        }
        self.refused_with_guard += 1;
        if matches!(
            plan(app, bucket, &blanked(m), layout),
            Err(PlanError::ScriptRefused { .. })
        ) {
            return;
        }
        let name = format!("{bucket}/{app}");
        if refusing.iter().all(|f| f.kind == FlagKind::GlobalSwitch) {
            self.guard_only_global_switch.push(name.clone());
        }
        if scripts.iter().any(|s| assigns_global(s)) {
            self.guard_only_reassign.push(name.clone());
        }
        self.guard_only.push(name);
    }

    /// `[installable, gate refused, other error, total]` over the cells matching `pred`.
    fn sum(&self, pred: impl Fn(&str, SubCase) -> bool) -> [usize; 4] {
        let outcomes = [
            Outcome::Installable,
            Outcome::GateRefused,
            Outcome::OtherError,
        ];
        let mut t = [0usize; 4];
        for ((b, s), m) in &self.cells {
            if pred(b, *s) {
                for (i, o) in outcomes.iter().enumerate() {
                    let n = m.get(o).copied().unwrap_or(0);
                    t[i] += n;
                    t[3] += n;
                }
            }
        }
        t
    }

    fn print(&self) {
        println!(
            "\n## Instalável pela 2c (Task 163)\n\nInstalável = `resolve` (arquitetura do host) + `plan_install` sem erro, com o portão do planejador (`supported = PRELUDE_FUNCTIONS`, {} funções). Nada foi executado. Antes (2a): nenhum manifesto 2c planejava (a 2a recusava psmodule, innosetup, installer e scripts), então a coluna \"antes\" é 0.\n",
            PRELUDE_FUNCTIONS.len()
        );
        let header = |first: &str| {
            println!(
                "| {first} | 2c | instaláveis antes | instaláveis depois | recusados pelo portão | outro erro |"
            );
            println!("|---|---:|---:|---:|---:|---:|");
        };
        let row = |label: &str, t: [usize; 4]| {
            println!(
                "| {label} | {} | 0 | {} | {} | {} |",
                t[3],
                pct(t[0], t[3]),
                pct(t[1], t[3]),
                pct(t[2], t[3])
            );
        };
        header("bucket");
        let buckets: BTreeSet<&str> = self.cells.keys().map(|(b, _)| b.as_str()).collect();
        for b in &buckets {
            row(b, self.sum(|x, _| x == *b));
        }
        row("todos", self.sum(|_, _| true));
        println!();
        header("pior sub-caso (`worst`)");
        for s in SubCase::ALL {
            row(s.label(), self.sum(|_, x| x == s));
        }
        println!(
            "\n### As 10 razões de recusa mais frequentes do portão\n\nCada razão conta uma vez por manifesto; um manifesto pode ter várias.\n"
        );
        println!("| # | razão | manifestos |");
        println!("|---:|---|---:|");
        for (i, (d, n)) in top(&self.gate_reasons, 10).into_iter().enumerate() {
            println!("| {} | {} | {n} |", i + 1, d.replace('|', "\\|"));
        }
        println!("\n| tipo de recusa (`FlagKind`) | manifestos |");
        println!("|---|---:|");
        for (k, n) in top(&self.gate_kinds, usize::MAX) {
            println!("| {k} | {n} |");
        }
        println!("\n### Outros erros do planejador (não são do portão)\n");
        println!("| erro | manifestos |");
        println!("|---|---:|");
        for (k, n) in top(&self.other_reasons, 15) {
            println!("| {} | {n} |", k.replace('|', "\\|"));
        }
        let names = |v: &[String]| {
            v.iter()
                .take(10)
                .map(|n| format!("`{n}`"))
                .collect::<Vec<_>>()
                .join(", ")
        };
        println!(
            "\n### Recusas só dentro de `if ($global) {{ ... }}`\n\nRecusados pelo portão com algum bloco `if ($global)`: {}. Planejam com o corpo desses blocos apagado: {} ({}). Desses, todo flag de recusa é `-Global` (`GlobalSwitch`): {} ({}). Atribuem `$global` em algum script (o bloco poderia rodar): {} ({}).",
            self.refused_with_guard,
            self.guard_only.len(),
            names(&self.guard_only),
            self.guard_only_global_switch.len(),
            names(&self.guard_only_global_switch),
            self.guard_only_reassign.len(),
            names(&self.guard_only_reassign),
        );
    }
}
struct ScriptManifest {
    has_ps1: bool,
    /// Lowercase command names (PowerShell is case-insensitive).
    commands: Vec<String>,
}

fn pct(n: usize, t: usize) -> String {
    format!("{n} ({:.1}%)", n as f64 * 100.0 / t.max(1) as f64)
}

fn sum_by(buckets: &BTreeMap<String, Bucket>, f: impl Fn(&Bucket) -> usize) -> usize {
    buckets.values().map(f).sum()
}

fn main() -> anyhow::Result<()> {
    let db = std::env::args().nth(1).context("faltou o catalog.db")?;
    let conn = Connection::open_with_flags(
        &db,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    let mut st = conn
        .prepare("SELECT source, id, raw FROM packages WHERE kind = 'scoop' AND raw IS NOT NULL")?;
    let rows = st.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
        ))
    })?;
    let scoop_fns: HashSet<String> = SCOOP_FUNCTIONS
        .iter()
        .map(|f| f.to_ascii_lowercase())
        .collect();

    let mut buckets: BTreeMap<String, Bucket> = BTreeMap::new();
    let mut scripted: Vec<ScriptManifest> = Vec::new();
    let mut gate = Gate::default();
    let mut inst = Installable::default();
    let layout = Layout::new(r"C:\oracle\wingetrs");
    // lowercase name -> (first spelling seen, manifests using it)
    let mut ranking: BTreeMap<String, (String, usize)> = BTreeMap::new();
    for r in rows {
        let (source, id, raw) = r?;
        let name = source.strip_prefix("scoop:").unwrap_or(&source);
        let key = match name {
            "main" | "extras" => name.to_string(),
            _ => "demais".to_string(),
        };
        let b = buckets.entry(key.clone()).or_default();
        b.total += 1;
        let c = match classify(&raw) {
            Ok(Some(c)) => c,
            Ok(None) => continue,
            Err(_) => {
                b.invalid += 1;
                continue;
            }
        };
        b.c2 += 1;
        inst.add(&key, c.worst, &id, &raw, &layout);
        *b.worst.entry(c.worst).or_default() += 1;
        for s in &c.cases {
            *b.present.entry(*s).or_default() += 1;
        }
        if c.worst <= SubCase::File {
            b.no_powershell += 1;
        }
        if c.cases.contains(&SubCase::Hook) {
            gate.add(&raw);
        }
        let has_ps1 = c.cases.contains(&SubCase::Ps1);
        if has_ps1 || c.cases.contains(&SubCase::Hook) {
            let commands: Vec<String> = script_commands(&raw);
            for cmd in &commands {
                ranking
                    .entry(cmd.to_ascii_lowercase())
                    .or_insert_with(|| (cmd.clone(), 0))
                    .1 += 1;
            }
            scripted.push(ScriptManifest {
                has_ps1,
                commands: commands.iter().map(|c| c.to_ascii_lowercase()).collect(),
            });
        }
    }

    let all = Bucket {
        total: sum_by(&buckets, |b| b.total),
        invalid: sum_by(&buckets, |b| b.invalid),
        c2: sum_by(&buckets, |b| b.c2),
        worst: SubCase::ALL
            .iter()
            .map(|s| {
                (
                    *s,
                    sum_by(&buckets, |b| b.worst.get(s).copied().unwrap_or(0)),
                )
            })
            .collect(),
        present: SubCase::ALL
            .iter()
            .map(|s| {
                (
                    *s,
                    sum_by(&buckets, |b| b.present.get(s).copied().unwrap_or(0)),
                )
            })
            .collect(),
        no_powershell: sum_by(&buckets, |b| b.no_powershell),
    };
    let mut table: Vec<(&str, &Bucket)> = ["main", "extras", "demais"]
        .iter()
        .filter_map(|k| buckets.get(*k).map(|b| (*k, b)))
        .collect();
    table.push(("todos", &all));
    let labels: Vec<&str> = SubCase::ALL.iter().map(|s| s.label()).collect();

    println!("# Oráculo da fase 2c\n");
    println!(
        "Catálogo: `{db}` (somente leitura). Manifestos scoop: {}, JSON inválido: {}.\n",
        all.total, all.invalid
    );
    println!("## Manifestos 2c por bucket e pior sub-caso (`worst`)\n");
    println!(
        "| bucket | total | 2c | {} | só S-psm/S-inno/S-file (dos 2c) |",
        labels.join(" | ")
    );
    println!("|---|---:|---:|{}---:|", "---:|".repeat(labels.len()));
    for (name, b) in &table {
        let cols: Vec<String> = SubCase::ALL
            .iter()
            .map(|s| b.worst.get(s).copied().unwrap_or(0).to_string())
            .collect();
        println!(
            "| {name} | {} | {} | {} | {} |",
            b.total,
            pct(b.c2, b.total),
            cols.join(" | "),
            pct(b.no_powershell, b.c2)
        );
    }
    println!("\n## Presença de cada sub-caso (% dos 2c; um manifesto pode ter vários)\n");
    println!("| bucket | {} |", labels.join(" | "));
    println!("|---|{}", "---:|".repeat(labels.len()));
    for (name, b) in &table {
        let cols: Vec<String> = SubCase::ALL
            .iter()
            .map(|s| pct(b.present.get(s).copied().unwrap_or(0), b.c2))
            .collect();
        println!("| {name} | {} |", cols.join(" | "));
    }

    let mut ranked: Vec<(&String, &(String, usize))> = ranking.iter().collect();
    ranked.sort_by(|a, b| b.1.1.cmp(&a.1.1).then_with(|| a.0.cmp(b.0)));
    let kind_of = |lower: &str| kind(lower, &scoop_fns);
    let count_kind = |k: Kind| ranked.iter().filter(|(c, _)| kind_of(c) == k).count();
    println!(
        "\n## Comandos chamados pelos scripts\n\n{} manifestos S-hook/S-ps1; {} comandos distintos: {} funções do Scoop (lista de {}), {} cmdlets, {} externos ou definidos fora do manifesto. Funções definidas pelo próprio manifesto (`function`/`filter`) não entram.\n",
        scripted.len(),
        ranked.len(),
        count_kind(Kind::Scoop),
        SCOOP_FUNCTIONS.len(),
        count_kind(Kind::Cmdlet),
        count_kind(Kind::External),
    );
    println!("| # | comando | manifestos | tipo |");
    println!("|---:|---|---:|---|");
    for (i, (k, (shown, n))) in ranked.iter().enumerate().take(80) {
        println!("| {} | `{shown}` | {n} | {} |", i + 1, kind_of(k).label());
    }
    let list = |k: Kind| -> String {
        ranked
            .iter()
            .filter(|(c, _)| kind_of(c) == k)
            .map(|(_, (shown, n))| format!("`{shown}` ({n})"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    println!("\nTodas as funções do Scoop usadas: {}.", list(Kind::Scoop));
    println!(
        "\nTodos os externos ou definidos fora do manifesto: {}.",
        list(Kind::External)
    );

    // Metric B (decision D1 = B): own prelude for Scoop functions; cmdlets and native
    // commands run in the real pwsh. A manifest is covered when the Scoop functions it
    // calls are all among the N most used ones. S-ps1 is never covered.
    let scoop_ranked: Vec<&str> = ranked
        .iter()
        .filter(|(c, _)| kind_of(c) == Kind::Scoop)
        .map(|(c, _)| c.as_str())
        .collect();
    let uses_scoop = |m: &ScriptManifest| m.commands.iter().any(|c| c == "scoop");
    let n_uses_scoop = scripted.iter().filter(|m| uses_scoop(m)).count();
    let without_scoop: Vec<&ScriptManifest> = scripted.iter().filter(|m| !uses_scoop(m)).collect();
    let covered_b = |set: &[&ScriptManifest], top: &BTreeSet<&str>| {
        set.iter()
            .filter(|m| {
                !m.has_ps1
                    && m.commands
                        .iter()
                        .filter(|c| scoop_fns.contains(c.as_str()))
                        .all(|c| top.contains(c.as_str()))
            })
            .count()
    };
    let all_refs: Vec<&ScriptManifest> = scripted.iter().collect();
    println!(
        "\n## Cobertura acumulada (métrica B: prelúdio próprio com as funções do Scoop; cmdlets e comandos nativos rodam no pwsh real)\n\nManifesto coberto = sem S-ps1 (o conteúdo do `.ps1` não está no manifesto) e todas as funções do Scoop que os seus scripts chamam estão entre as N mais usadas; o que não é função do Scoop é ignorado. Denominador: {} manifestos S-hook/S-ps1 ({} com S-ps1). Usam `scoop` como comando externo: {} manifestos; a última coluna os exclui do denominador.\n",
        scripted.len(),
        scripted.iter().filter(|m| m.has_ps1).count(),
        n_uses_scoop
    );
    println!("| N funções do Scoop | cobertos | cobertos sem os que usam `scoop` |");
    println!("|---|---:|---:|");
    for n in [5usize, 10, 20, usize::MAX] {
        let label = if n == usize::MAX {
            format!("todas ({})", scoop_ranked.len())
        } else {
            n.to_string()
        };
        let top: BTreeSet<&str> = scoop_ranked.iter().take(n).copied().collect();
        println!(
            "| {label} | {} | {} |",
            pct(covered_b(&all_refs, &top), all_refs.len()),
            pct(covered_b(&without_scoop, &top), without_scoop.len())
        );
    }

    // Floor: every command (of any kind) must be in the set of the N most used.
    let all_ranked: Vec<&str> = ranked.iter().map(|(c, _)| c.as_str()).collect();
    let covered_floor = |top: &BTreeSet<&str>| {
        scripted
            .iter()
            .filter(|m| !m.has_ps1 && m.commands.iter().all(|c| top.contains(c.as_str())))
            .count()
    };
    println!("\n## Piso: implementar os N comandos mais usados de qualquer tipo (sem pwsh real)\n");
    println!("| N comandos | cobertos |");
    println!("|---|---:|");
    for n in [5usize, 10, 20, 30, 50, 100, usize::MAX] {
        let label = if n == usize::MAX {
            format!("todos ({})", all_ranked.len())
        } else {
            n.to_string()
        };
        let top: BTreeSet<&str> = all_ranked.iter().take(n).copied().collect();
        println!("| {label} | {} |", pct(covered_floor(&top), scripted.len()));
    }
    gate.print();
    inst.print();
    Ok(())
}
