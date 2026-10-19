//! Measures the real HKLM 64/32 uninstallers of this machine against the elevated helper's
//! rules (Task 182), read only: every visible entry goes through the real `RunUninstaller`
//! checks of `bridge::worker::serve` and the owner/DACL chain rule; nothing is run (the probe's
//! `run_uninstaller` only records the program). Prints a table per result and reason.
//!
//! `cargo run --release --example elevated_arp_probe`
#![allow(clippy::expect_used, clippy::unwrap_used)]

#[cfg(windows)]
mod probe {
    use std::collections::BTreeMap;
    use std::io::BufReader;
    use std::os::windows::fs::MetadataExt;
    use std::path::{Path, PathBuf};
    use windows::Win32::Foundation::{ERROR_SUCCESS, HLOCAL, LocalFree};
    use windows::Win32::Security::Authorization::{
        ConvertSecurityDescriptorToStringSecurityDescriptorW, GetNamedSecurityInfoW,
        SDDL_REVISION_1, SE_FILE_OBJECT,
    };
    use windows::Win32::Security::{
        DACL_SECURITY_INFORMATION, OWNER_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR,
    };
    use windows::core::{HSTRING, PWSTR};
    use wingetrs::bridge::arp_ops_win;
    use wingetrs::bridge::worker::{self, MachineHive, Reply, Request, WorkerOps};
    use wingetrs::installed::Hive;
    use wingetrs::winget_install::arp_read_win::read_details;
    use wingetrs::winget_install::elevated_rules::KnownFolders;
    use wingetrs::winget_install::elevated_rules::{AclLevel, sddl_admin_only};
    use wingetrs::winget_install::launch_win::msiexec_path;
    use wingetrs::winget_install::uninstall_cmd::ArpDetails;
    use wingetrs::winget_install::uninstall_cmd::uninstall_command;

    #[derive(Default)]
    struct ProbeOps {
        /// The program and the other files (a rundll32 DLL) the helper would check and run.
        planned: Option<Vec<PathBuf>>,
    }

    impl WorkerOps for ProbeOps {
        fn run(
            &mut self,
            program: &str,
            _args: &[String],
            _on_line: &mut dyn FnMut(&str),
        ) -> std::io::Result<(i32, Vec<String>)> {
            panic!("unexpected run of {program}")
        }

        fn known_folders(&self) -> std::io::Result<KnownFolders> {
            arp_ops_win::known_folders()
        }

        fn read_arp(&mut self, hive: Hive, key: &str) -> std::io::Result<Option<ArpDetails>> {
            read_details(hive, key)
        }

        fn exists(&self, path: &Path) -> bool {
            arp_ops_win::exists(path)
        }

        fn msiexec(&self) -> std::io::Result<PathBuf> {
            msiexec_path()
        }

        /// Records the program; never runs it.
        fn long_path(&self, path: &Path) -> std::io::Result<PathBuf> {
            arp_ops_win::long_path(path)
        }

        fn run_uninstaller(
            &mut self,
            program: &Path,
            _args: &str,
            also_checked: &[PathBuf],
            _on_line: &mut dyn FnMut(&str),
        ) -> std::io::Result<i32> {
            let mut all = vec![program.to_path_buf()];
            all.extend_from_slice(also_checked);
            self.planned = Some(all);
            Ok(0)
        }
    }

    fn sddl_of(path: &Path) -> Result<String, String> {
        let info = OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION;
        let mut sd = PSECURITY_DESCRIPTOR::default();
        // SAFETY: `sd` receives a LocalAlloc descriptor, freed below.
        let rc = unsafe {
            GetNamedSecurityInfoW(
                &HSTRING::from(path.as_os_str()),
                SE_FILE_OBJECT,
                info,
                None,
                None,
                None,
                None,
                &mut sd,
            )
        };
        if rc != ERROR_SUCCESS {
            return Err(format!("GetNamedSecurityInfo {}", rc.0));
        }
        let mut text = PWSTR::null();
        // SAFETY: `sd` is valid; `text` receives a LocalAlloc string.
        let r = unsafe {
            ConvertSecurityDescriptorToStringSecurityDescriptorW(
                sd,
                SDDL_REVISION_1,
                info,
                &mut text,
                None,
            )
        };
        // SAFETY: a NUL-terminated string from the call above, when it succeeded.
        let out = r
            .map_err(|e| e.to_string())
            .and_then(|()| unsafe { text.to_string() }.map_err(|e| e.to_string()));
        // SAFETY: both allocated by the calls above, freed once.
        unsafe {
            if !text.is_null() {
                LocalFree(Some(HLOCAL(text.0.cast())));
            }
            LocalFree(Some(HLOCAL(sd.0)));
        }
        out
    }

    /// The same chain rule as `arp_ops_win::hold_admin_chain`, by path (no handles held).
    fn chain(program: &Path) -> Result<(), (String, String)> {
        let mut objects = vec![(program.to_path_buf(), AclLevel::File)];
        for (n, dir) in program.ancestors().skip(1).enumerate() {
            let level = if n == 0 {
                AclLevel::Parent
            } else {
                AclLevel::Ancestor
            };
            objects.push((dir.to_path_buf(), level));
        }
        for (path, level) in objects {
            let meta = std::fs::symlink_metadata(&path)
                .map_err(|e| ("metadados ilegíveis".to_string(), e.to_string()))?;
            if meta.file_attributes() & 0x400 != 0 {
                return Err(("ponto de reanálise".into(), path.display().to_string()));
            }
            let sddl = sddl_of(&path).map_err(|e| ("ACL ilegível".to_string(), e))?;
            if !sddl_admin_only(&sddl, level) {
                return Err((
                    format!("DACL/dono ({level:?})"),
                    format!("{} {sddl}", path.display()),
                ));
            }
        }
        Ok(())
    }

    fn kind_of(d: &ArpDetails) -> &'static str {
        if d.windows_installer {
            "msi"
        } else if d.key.to_ascii_lowercase().ends_with("_is1") {
            "inno"
        } else {
            "outro"
        }
    }

    #[derive(Default)]
    struct Row {
        msi: usize,
        inno: usize,
        other: usize,
        names: Vec<String>,
        sample: Option<String>,
    }

    /// Sends one request to the real `serve` loop; its only reply.
    fn ask(req: &Request, ops: &mut ProbeOps) -> Reply {
        let mut out = Vec::new();
        let line = worker::encode(req).expect("encode");
        worker::serve(&mut BufReader::new(line.as_bytes()), &mut out, ops).expect("serve");
        let text = String::from_utf8(out).expect("utf-8");
        let first = text.lines().next().expect("one reply");
        worker::decode::<Reply>(first).expect("reply")
    }

    pub fn main() {
        let entries = arp_ops_win::arp_entries().expect("ARP");
        let mut rows: BTreeMap<String, Row> = BTreeMap::new();
        let mut totals: BTreeMap<&str, usize> = BTreeMap::new();
        for d in entries {
            let Some(name) = d
                .display_name
                .clone()
                .filter(|n| !n.trim().is_empty() && !d.system_component)
            else {
                continue;
            };
            let kind = kind_of(&d);
            *totals.entry(kind).or_default() += 1;
            let (reason, sample) = match uninstall_command(&d, false) {
                None => ("sem comando de desinstalação".to_string(), None),
                Some(cmd) => {
                    let hive = match d.hive {
                        Hive::MachineX64 => MachineHive::Hklm64,
                        Hive::MachineX86 => MachineHive::Hklm32,
                        Hive::User => continue,
                    };
                    let mut ops = ProbeOps::default();
                    let reply = ask(
                        &Request::RunUninstaller {
                            hive,
                            key: d.key.clone(),
                            rule: cmd.rule,
                            nsis_hint: false,
                        },
                        &mut ops,
                    );
                    match (reply, ops.planned) {
                        (Reply::End { .. }, Some(files)) => {
                            match files.iter().try_for_each(|f| chain(f)) {
                                Ok(()) => (format!("passa ({:?})", cmd.rule), None),
                                Err((why, sample)) => (why, Some(sample)),
                            }
                        }
                        (Reply::Error { message, not_found }, _) => {
                            let short = if not_found {
                                "desinstalador não encontrado".to_string()
                            } else {
                                message
                            };
                            let raw = format!("{} {}", cmd.program, cmd.args);
                            (short, Some(raw))
                        }
                        (other, _) => (format!("inesperado {other:?}"), None),
                    }
                }
            };
            let row = rows.entry(reason).or_default();
            match kind {
                "msi" => row.msi += 1,
                "inno" => row.inno += 1,
                _ => row.other += 1,
            }
            if row.names.len() < 5 {
                row.names.push(name);
            }
            if row.sample.is_none() {
                row.sample = sample;
            }
        }
        println!("totais: {totals:?}");
        println!("| resultado / motivo | total | msi | inno | outro | exemplos |");
        println!("|---|---|---|---|---|---|");
        for (reason, r) in &rows {
            println!(
                "| {reason} | {} | {} | {} | {} | {} |",
                r.msi + r.inno + r.other,
                r.msi,
                r.inno,
                r.other,
                r.names.join("; ")
            );
        }
        for (reason, r) in &rows {
            if let Some(s) = &r.sample {
                println!("amostra [{reason}]: {s}");
            }
        }
    }
}

fn main() {
    #[cfg(windows)]
    probe::main();
}
