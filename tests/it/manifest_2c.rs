//! Phase 2c tests (Task 143).
#![allow(clippy::unwrap_used, clippy::expect_used)]
use std::collections::BTreeSet;
use std::path::Path;
use wingetrs::install::layout::Layout;
use wingetrs::install::manifest::{Arch, Helper, InstallerSpec, ManifestError, Resolved, resolve};
use wingetrs::install::planner::{PlanError, PlanInput, plan_install};

fn res(extra: &str) -> Result<Resolved, ManifestError> {
    let raw = format!(r#"{{"version":"2","url":"https://x/a.zip","hash":"aa"{extra}}}"#);
    resolve(&raw, Arch::X64)
}
fn ok(extra: &str) -> Resolved {
    res(extra).unwrap()
}
fn is_invalid<T: std::fmt::Debug>(r: Result<T, ManifestError>) -> bool {
    matches!(r, Err(ManifestError::Invalid(_)))
}
fn plan(m: &Resolved) -> Result<wingetrs::install::plan::InstallPlan, PlanError> {
    let l = Layout::new(r"C:\W");
    plan_install(&PlanInput {
        app: "app",
        bucket: "main",
        manifest: m,
        layout: &l,
        shortcut_root: Path::new(r"C:\SM\wingetrs Apps"),
        installed: None,
        launcher_present: true,
        keep_old: false,
        manifest_sha256: "",
    })
}

#[test]
fn hooks_string_and_array() {
    let m = ok(r#","pre_install":"a","post_install":["l1","l2"]"#);
    assert_eq!(m.hooks.pre_install.as_deref(), Some("a"));
    assert_eq!(m.hooks.post_install.as_deref(), Some("l1\r\nl2"));
}

#[test]
fn empty_hook_is_none() {
    let m = ok(r#","pre_install":"","post_install":[]"#);
    assert_eq!(m.hooks.pre_install, None);
    assert_eq!(m.hooks.post_install, None);
    assert!(is_invalid(res(r#","pre_install":3"#)));
}

#[test]
fn variant_installer_overrides_root() {
    let m = ok(
        r#","installer":{"file":"root.exe"},"architecture":{"64bit":{"installer":{"file":"v64.exe"}}}"#,
    );
    assert_eq!(m.installer.unwrap().file.as_deref(), Some("v64.exe"));
    let m = ok(r#","installer":{"file":"root.exe"},"architecture":{"64bit":{}}"#);
    assert_eq!(m.installer.unwrap().file.as_deref(), Some("root.exe"));
}

#[test]
fn variant_hook_overrides_root_key_by_key() {
    let m = ok(
        r#","pre_install":"R","post_install":"RP","architecture":{"64bit":{"post_install":"V"}}"#,
    );
    assert_eq!(m.hooks.pre_install.as_deref(), Some("R"));
    assert_eq!(m.hooks.post_install.as_deref(), Some("V"));
}

#[test]
fn installer_fields() {
    let m = ok(r#","installer":{"file":"setup.exe","args":["/S","/D=$dir"],"keep":true}"#);
    assert_eq!(
        m.installer,
        Some(InstallerSpec {
            file: Some("setup.exe".into()),
            args: vec!["/S".into(), "/D=$dir".into()],
            keep: true,
            script: None,
        })
    );
    let m = ok(r#","installer":{"file":"setup.exe","args":"/S"}"#);
    assert_eq!(m.installer.unwrap().args, vec!["/S".to_string()]);
}

#[test]
fn installer_args_control_char_refused() {
    assert!(is_invalid(res(
        r#","installer":{"file":"a.exe","args":"/S\u0007"}"#
    )));
}

#[test]
fn installer_file_unsafe_refused() {
    for f in [r"..\\x.exe", r"C:\\x.exe", "a/../../x.exe"] {
        let r = res(&format!(r#","installer":{{"file":"{f}"}}"#));
        assert_eq!(
            r.unwrap_err(),
            ManifestError::Invalid("installer.file: caminho inseguro".into()),
            "{f}"
        );
    }
}

#[test]
fn uninstaller_script_only() {
    let m = ok(r#","uninstaller":{"script":["a","b"]}"#);
    let u = m.uninstaller.unwrap();
    assert_eq!(u.script.as_deref(), Some("a\r\nb"));
    assert_eq!(u.file, None);
}

#[test]
fn psmodule_rules() {
    assert!(ok(r#","psmodule":{"name":"PSFzf"}"#).psmodule.is_some());
    assert_eq!(
        res(r#","psmodule":{}"#).unwrap_err(),
        ManifestError::Invalid("psmodule sem name".into())
    );
    assert_eq!(
        res(r#","psmodule":{"name":"..\\x"}"#).unwrap_err(),
        ManifestError::Invalid("psmodule.name: nome inseguro".into())
    );
    assert!(is_invalid(res(r#","psmodule":{"name":"a/b"}"#)));
}

#[test]
fn psmodule_in_variant_only_is_ignored() {
    let m = ok(r#","architecture":{"64bit":{"psmodule":{"name":"X"}}}"#);
    assert_eq!(m.psmodule, None);
}

#[test]
fn innosetup_values() {
    assert!(ok(r#","innosetup":true"#).innosetup);
    assert!(!ok(r#","innosetup":false"#).innosetup);
    assert!(!ok("").innosetup);
    assert!(is_invalid(res(r#","innosetup":"yes""#)));
}

fn helpers(extra: &str) -> BTreeSet<Helper> {
    ok(extra).helpers
}

#[test]
fn helpers_follow_scoop_rule() {
    let set = |h: &[Helper]| h.iter().copied().collect::<BTreeSet<_>>();
    assert_eq!(helpers(r#","innosetup":true"#), set(&[Helper::Innounp]));
    assert_eq!(
        helpers(r#","post_install":"Expand-7zipArchive \"$dir\\x.7z\"""#),
        set(&[Helper::SevenZip])
    );
    assert_eq!(
        helpers(r#","pre_install":"expand-msiarchive x""#),
        set(&[Helper::Lessmsi])
    );
    assert_eq!(
        helpers(r#","installer":{"script":"Expand-DarkArchive x"}"#),
        set(&[Helper::Dark])
    );
    assert_eq!(
        helpers(r#","post_install":"Expand-InnoArchive x""#),
        set(&[Helper::Innounp])
    );
    assert_eq!(
        helpers(r#","pre_uninstall":"Expand-7zipArchive x""#),
        set(&[])
    );
    assert_eq!(
        helpers(r##","post_install":"# Expand-InnoArchive x""##),
        set(&[])
    );
    assert_eq!(
        helpers(r#","post_install":"Expand-InnoArchiveX x""#),
        set(&[])
    );
}

#[test]
fn raw_json_kept_verbatim() {
    let raw = "{ \"url\" :\"https://x/a.zip\",\n  \"hash\":\"aa\",   \"version\":\"2\" }";
    assert_eq!(resolve(raw, Arch::X64).unwrap().raw_json, raw);
}

#[test]
fn planner_refuses_each_2c_key_with_old_text() {
    let cases = [
        r#","pre_uninstall":"x""#,
        r#","post_uninstall":"x""#,
        r#","installer":{"file":"a.ps1"}"#,
        r#","uninstaller":{"file":"a.ps1"}"#,
    ];
    let keys = [
        "pre_uninstall",
        "post_uninstall",
        "installer",
        "uninstaller",
    ];
    for (c, k) in cases.iter().zip(keys) {
        let e = plan(&ok(c)).unwrap_err();
        assert!(matches!(e, PlanError::Unsupported(_)), "{k}");
        assert_eq!(
            e.to_string(),
            format!("não suportado: usa {k} (PowerShell/instalador próprio): suportado na fase 2c")
        );
    }
    // `innosetup` is planned since Task 147 (extracted through innounp), the install hooks since
    // Task 156 (script host).
    assert!(plan(&ok(r#","pre_install":"x""#)).is_ok());
    assert!(plan(&ok(r#","post_install":"x""#)).is_ok());
    assert!(plan(&ok(r#","innosetup":true"#)).is_ok());
}

#[test]
fn simple_manifest_still_plans() {
    assert!(plan(&ok("")).is_ok());
}

fn invalid_msg(extra: &str, msg: &str) {
    assert_eq!(
        res(extra).unwrap_err(),
        ManifestError::Invalid(msg.into()),
        "{extra}"
    );
}

#[test]
fn each_new_refusal_has_its_own_fixture() {
    invalid_msg(r#","installer":"x""#, "installer deve ser um objeto");
    invalid_msg(
        r#","installer":{"file":"a.exe","keep":"yes"}"#,
        "installer.keep deve ser true ou false",
    );
    invalid_msg(
        r#","post_install":["a",3]"#,
        "post_install: deve ser texto ou lista de textos",
    );
    invalid_msg(r#","psmodule":"x""#, "psmodule deve ser um objeto");
}

#[test]
fn empty_variant_value_falls_back_to_root() {
    let m = ok(r#","pre_install":"R","architecture":{"64bit":{"pre_install":""}}"#);
    assert_eq!(m.hooks.pre_install.as_deref(), Some("R"));
    let m = ok(r#","post_install":"R","architecture":{"64bit":{"post_install":[]}}"#);
    assert_eq!(m.hooks.post_install.as_deref(), Some("R"));
    let m = ok(r#","installer":{"file":"r.exe"},"architecture":{"64bit":{"installer":{}}}"#);
    assert_eq!(m.installer.unwrap().file.as_deref(), Some("r.exe"));
}
