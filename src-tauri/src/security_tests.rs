//! Adversarial, recovery and property tests that cut across modules. Everything runs against local fixtures
//! (temporary folders, mock executables, in-memory state); nothing here contacts a public host.

use crate::adapters::locator::SystemExecutableLocator;
use crate::adapters::registry::{ManifestRegistry, MAX_MANIFEST_BYTES};
use crate::adapters::result_saver::validate_destination;
use crate::adapters::sqlite_store::{prepare_database, SqliteJobStore};
use crate::domain::activity::redact;
use crate::domain::container::validate_host_mount;
use crate::domain::manifest::{build_command, ParamValue, ToolManifest};
use crate::domain::scope::{parse_target, Endpoint, TargetForm};
use crate::ports::{ExecutableLocator, JobStore, ToolQuery, ToolRegistry, TrustLevel};
use proptest::prelude::*;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

const NMAP: &str = include_str!("../../manifests/nmap.toml");
const GOOD: &str = crate::domain::manifest::fixtures::VALID;

fn dir_with(files: &[(&str, &[u8])]) -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    for (name, bytes) in files {
        std::fs::write(d.path().join(name), bytes).unwrap();
    }
    d
}

// ------------------------------------------------------------------ corrupted or hostile catalog

#[test]
fn a_missing_or_unreadable_catalog_folder_gives_an_empty_catalog_and_a_reason() {
    let reg = ManifestRegistry::load_dir_or_empty(Path::new("C:\\definitely\\not\\here"), TrustLevel::Bundled);
    assert!(reg.list(&ToolQuery::default()).is_empty());
    assert_eq!(reg.rejected.len(), 1);
    assert!(reg.rejected[0].to_string().contains("could not be read"));
}

#[test]
fn hostile_manifest_files_are_rejected_one_by_one_and_never_hide_good_ones() {
    let huge = vec![b'#'; MAX_MANIFEST_BYTES as usize + 10];
    let d = dir_with(&[
        ("good.toml", GOOD.as_bytes()),
        ("huge.toml", &huge),
        ("binary.toml", &[0xff, 0xfe, 0x00, 0x01, 0x80]),
        ("empty.toml", b""),
        ("garbage.toml", b"[[[[ this is not toml"),
        ("script.toml", GOOD.replace("id = \"nmap\"", "id = \"evil\"").replace("[[install]]", "post_install_script = \"calc\"\n[[install]]").as_bytes()),
        ("notes.txt", b"ignored: wrong extension"),
    ]);
    let reg = ManifestRegistry::load_dir(d.path(), TrustLevel::Bundled).unwrap();
    let ids: Vec<String> = reg.list(&ToolQuery::default()).iter().map(|e| e.manifest.id.to_string()).collect();
    assert_eq!(ids, ["nmap"], "only the valid manifest is loaded");
    assert_eq!(reg.rejected.len(), 5, "{:?}", reg.rejected);
    assert!(reg.rejected.iter().any(|e| e.to_string().contains("larger than")));
}

#[test]
fn the_manifest_digest_tracks_the_exact_bytes_loaded() {
    let a = dir_with(&[("a.toml", GOOD.as_bytes())]);
    let b = dir_with(&[("a.toml", format!("{GOOD}\n# a comment changes the bytes").as_bytes())]);
    let da = ManifestRegistry::load_dir(a.path(), TrustLevel::Bundled).unwrap().list(&ToolQuery::default())[0].digest.clone();
    let db = ManifestRegistry::load_dir(b.path(), TrustLevel::Bundled).unwrap().list(&ToolQuery::default())[0].digest.clone();
    assert_eq!(da.len(), 64);
    assert_ne!(da, db);
    let again = ManifestRegistry::load_dir(a.path(), TrustLevel::Bundled).unwrap().list(&ToolQuery::default())[0].digest.clone();
    assert_eq!(da, again);
}

#[test]
fn a_manifest_that_is_a_link_is_not_followed() {
    let d = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("real.toml"), GOOD).unwrap();
    let link = d.path().join("link.toml");
    // Creating symbolic links needs a privilege on Windows; without it there is nothing to test.
    if std::os::windows::fs::symlink_file(outside.path().join("real.toml"), &link).is_err() {
        eprintln!("skipped: this account cannot create symbolic links");
        return;
    }
    let reg = ManifestRegistry::load_dir(d.path(), TrustLevel::Bundled).unwrap();
    assert!(reg.list(&ToolQuery::default()).is_empty());
    assert_eq!(reg.rejected.len(), 1);
}

#[test]
fn manifests_cannot_set_environment_variables_that_redirect_or_inject_code() {
    for (name, value) in [("PATH", "C:\\evil"), ("JAVA_TOOL_OPTIONS", "-javaagent:x.jar"), ("NODE_OPTIONS", "--require x"), ("PYTHONSTARTUP", "x.py"), ("COMSPEC", "x.exe"), ("DOTNET_STARTUP_HOOKS", "x.dll"), ("API_TOKEN", "x")] {
        let m = NMAP.replacen("expected_output =", &format!("env = [{{ name = \"{name}\", value = \"{value}\" }}]\nexpected_output ="), 1);
        let e = ToolManifest::from_toml(&m).unwrap_err().to_string();
        assert!(e.contains("env name"), "{name}: {e}");
    }
}

#[test]
fn executables_and_locations_in_manifests_cannot_point_elsewhere() {
    for exe in ["..\\evil.exe", "C:\\evil.exe", "sub\\evil.exe", "evil.bat", "evil.cmd", "evil", "ev il.exe", "evil.exe ", "ev;il.exe", "a/b.exe"] {
        let m = NMAP.replacen("executable = \"nmap.exe\"", &format!("executable = \"{exe}\""), 1);
        assert!(ToolManifest::from_toml(&m).is_err(), "executable {exe:?} must be rejected");
    }
    for loc in ["C:\\Windows", "{ProgramFiles}\\..\\Windows", "..\\x", "{ProgramFiles}\\\\..", "\\\\server\\share", "{Nope}\\x"] {
        let m = NMAP.replacen("locations = [\"{ProgramFilesX86}\\\\Nmap\", \"{ProgramFiles}\\\\Nmap\"]", &format!("locations = [\"{}\"]", loc.replace('\\', "\\\\")), 1);
        assert!(ToolManifest::from_toml(&m).is_err(), "location {loc:?} must be rejected");
    }
}

// ------------------------------------------------------------------ database corruption

#[test]
fn a_damaged_database_is_moved_aside_and_a_fresh_one_works() {
    for damage in [&b"this is not a database at all"[..], &[0u8; 4096][..], b"SQLite format 3\0garbage after a plausible header"] {
        let d = tempfile::tempdir().unwrap();
        let db = d.path().join("vixalune.db");
        std::fs::write(&db, damage).unwrap();
        std::fs::write(d.path().join("vixalune.db-wal"), b"stale wal").unwrap();
        let moved = prepare_database(&db).unwrap().expect("a damaged file is detected");
        assert!(moved.exists() && std::fs::read(&moved).unwrap() == damage, "the evidence is kept, not deleted");
        assert!(!db.exists());
        assert!(!d.path().join("vixalune.db-wal").exists(), "stale journal files must not follow into the new database");
        let store = SqliteJobStore::open(&db).unwrap();
        assert!(store.list_jobs(10).unwrap().is_empty());
        assert!(prepare_database(&db).unwrap().is_none(), "a healthy database is left alone");
    }
}

#[test]
fn a_truncated_database_is_detected() {
    let d = tempfile::tempdir().unwrap();
    let db = d.path().join("vixalune.db");
    {
        let s = SqliteJobStore::open(&db).unwrap();
        for i in 0..200 {
            s.append_output(crate::domain::OperationId::new(), crate::domain::process::OutputStream::Stdout, &format!("line {i} {}", "x".repeat(200))).unwrap();
        }
    }
    let bytes = std::fs::read(&db).unwrap();
    assert!(bytes.len() > 8192);
    // Keep the header and cut the rest off mid-page.
    std::fs::write(&db, &bytes[..bytes.len() / 2 + 7]).unwrap();
    let _ = std::fs::remove_file(d.path().join("vixalune.db-wal"));
    assert!(prepare_database(&db).unwrap().is_some(), "a truncated file must not be opened as if it were fine");
}

#[test]
fn unreadable_rows_are_skipped_not_fatal() {
    use rusqlite::Connection;
    let d = tempfile::tempdir().unwrap();
    let db = d.path().join("vixalune.db");
    {
        SqliteJobStore::open(&db).unwrap();
        let c = Connection::open(&db).unwrap();
        c.execute("INSERT INTO runs (id, started_at, record_json) VALUES ('x', '2026-01-01T00:00:00Z', '{not json')", []).unwrap();
        c.execute("INSERT INTO installations (tool_id, record_json) VALUES ('nmap', '[1,2,3]')", []).unwrap();
        c.execute("INSERT INTO jobs (id, tool_id, kind, phase, message, started_at, updated_at, allow_elevation) VALUES ('j', 'nmap', 'bogus', 'bogus', '', 'a', 'b', 0)", []).unwrap();
    }
    let s = SqliteJobStore::open(&db).unwrap();
    use crate::ports::RunHistory;
    assert!(s.list_runs(10).unwrap().is_empty());
    assert!(s.list_installations().unwrap().is_empty());
    assert!(s.list_jobs(10).unwrap().is_empty());
}

// ------------------------------------------------------------------ executable resolution

#[test]
fn folders_vixalune_knows_win_over_the_path_and_names_with_separators_are_refused() {
    let trusted = dir_with(&[("vixalune-sectest.exe", b"MZ trusted")]);
    let planted = dir_with(&[("vixalune-sectest.exe", b"MZ planted")]);
    let old = std::env::var_os("PATH").unwrap_or_default();
    let mut paths: Vec<PathBuf> = vec![planted.path().to_path_buf()];
    paths.extend(std::env::split_paths(&old));
    std::env::set_var("PATH", std::env::join_paths(paths).unwrap());
    let loc = SystemExecutableLocator;
    let found = loc.locate("vixalune-sectest.exe", &[trusted.path().to_path_buf()]).unwrap();
    assert!(found.starts_with(trusted.path()), "the known install folder must beat an earlier PATH entry: {}", found.display());
    // Without a known folder the PATH is still used (that is how winget\'s links folder works).
    assert!(loc.locate("vixalune-sectest.exe", &[]).unwrap().starts_with(planted.path()));
    std::env::set_var("PATH", old);
    for bad in ["..\\vixalune-sectest.exe", "sub\\x.exe", "C:\\Windows\\System32\\whoami.exe", "x.bat", "x.cmd", "x.ps1", "x", "x.exe:stream", ""] {
        assert!(loc.locate(bad, &[trusted.path().to_path_buf()]).is_none(), "{bad:?}");
        assert!(loc.locate_system(bad).is_none(), "{bad:?}");
    }
}

#[test]
fn system_programs_come_only_from_the_system_folder() {
    let planted = dir_with(&[("wsl.exe", b"MZ planted")]);
    let old = std::env::var_os("PATH").unwrap_or_default();
    let mut paths: Vec<PathBuf> = vec![planted.path().to_path_buf()];
    paths.extend(std::env::split_paths(&old));
    std::env::set_var("PATH", std::env::join_paths(paths).unwrap());
    let found = SystemExecutableLocator.locate_system("whoami.exe").unwrap();
    std::env::set_var("PATH", old);
    let root = PathBuf::from(std::env::var_os("SystemRoot").unwrap()).join("System32");
    assert!(found.starts_with(&root), "{}", found.display());
    assert!(SystemExecutableLocator.locate_system("vixalune-not-a-system-program.exe").is_none());
}

// ------------------------------------------------------------------ properties

fn service_scan() -> crate::domain::manifest::RunSpec {
    ToolManifest::from_toml(NMAP).unwrap().launch_profiles.into_iter().find(|p| p.id == "service-scan").unwrap()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    /// No input, however odd, may panic the target parser; whatever it accepts has a clean canonical form.
    #[test]
    fn target_parsing_never_panics_and_accepted_targets_are_clean(s in "\\PC{0,80}", form in 0u8..3) {
        let form = [TargetForm::Host, TargetForm::Ip, TargetForm::Url][form as usize];
        if let Ok(t) = parse_target(&s, form) {
            prop_assert!(!t.canonical.chars().any(|c| c.is_control() || c.is_whitespace()), "{:?}", t.canonical);
            prop_assert!(!t.canonical.starts_with('-'));
            if let Endpoint::Name(n) = &t.endpoint {
                prop_assert!(n.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.' || c == '_'), "{n:?}");
            }
        }
    }

    /// Text that merely looks like a host never becomes more than one argument, and never an option.
    #[test]
    fn hostlike_values_become_exactly_one_argument(s in "[ -~]{0,60}") {
        let p = service_scan();
        let values = HashMap::from([("target".to_string(), ParamValue::Text(s.clone())), ("ports".to_string(), ParamValue::Text("80".into()))]);
        if let Ok(cmd) = build_command(&p, &values) {
            prop_assert_eq!(cmd.args.len(), 5, "fixed shape: -sV --version-light -p <ports> <target>: {:?}", cmd.args);
            let target = cmd.args.last().unwrap();
            prop_assert!(!target.starts_with('-'), "{target:?}");
            prop_assert!(!target.chars().any(|c| c.is_whitespace() || matches!(c, ';' | '&' | '|' | '<' | '>' | '`' | '$' | '"' | '\'')), "{target:?}");
        }
    }

    /// Port lists: whatever is accepted is digits, commas and hyphens only, with each port in range.
    #[test]
    fn accepted_port_lists_contain_only_valid_ports(s in "[0-9,\\- ;&a-z]{0,40}") {
        let p = service_scan();
        let values = HashMap::from([("ports".to_string(), ParamValue::Text(s.clone()))]);
        if let Ok(cmd) = build_command(&p, &values) {
            let ports = &cmd.args[3];
            prop_assert!(ports.chars().all(|c| c.is_ascii_digit() || c == ',' || c == '-'), "{ports:?}");
            for part in ports.split(',') {
                for n in part.split('-') {
                    let v: u32 = n.parse().unwrap();
                    prop_assert!((1..=65535).contains(&v));
                }
            }
        }
    }

    /// The redactor must not leave an obvious credential in its output.
    #[test]
    fn redaction_removes_credentials(secret in "[A-Za-z0-9]{8,32}", key in prop::sample::select(vec!["password", "token", "secret", "api_key", "apikey", "passwd"])) {
        let secret = format!("Zq{secret}");
        for text in [
            format!("--{key} {secret}"),
            format!("{key}={secret}"),
            format!("Authorization: Bearer {secret}"),
            format!("https://user:{secret}@example.test/path"),
        ] {
            prop_assert!(!redact(&text).contains(&secret), "leaked from {text:?}: {:?}", redact(&text));
        }
    }

    /// Mount validation: nothing under a protected top-level folder or with a credential folder in its path is accepted.
    #[test]
    fn protected_folders_are_never_accepted_for_mounting(
        top in prop::sample::select(vec!["Windows", "Program Files", "Program Files (x86)", "ProgramData", "windows", "PROGRAM FILES"]),
        sub in "[a-z]{1,8}",
        cred in prop::sample::select(vec![".ssh", ".aws", ".azure", ".gnupg", ".kube", ".docker", "AppData", "appdata"]),
    ) {
        let a = format!("C:\\{}\\{}", top, sub);
        let b = format!("D:\\work\\{}\\{}", cred, sub);
        let c = format!("C:\\Users\\me\\{}", cred);
        prop_assert!(validate_host_mount(&a, None).is_err(), "{}", a);
        prop_assert!(validate_host_mount(&b, None).is_err(), "{}", b);
        prop_assert!(validate_host_mount(&c, Some(Path::new("C:\\Users\\me"))).is_err(), "{}", c);
    }

    /// Save destinations: only allow-listed document types, and never a file that carries another extension after the stem.
    #[test]
    fn save_destinations_need_an_allowed_extension(stem in "[a-z]{1,8}", ext in prop::sample::select(vec!["exe", "dll", "bat", "cmd", "ps1", "lnk", "js", "vbs", "msi", "scr", "reg", "hta", "jar", "com"])) {
        let dir = std::env::temp_dir();
        let name = format!("{stem}.{ext}");
        prop_assert!(validate_destination(&dir.join(name)).is_err());
    }
}
