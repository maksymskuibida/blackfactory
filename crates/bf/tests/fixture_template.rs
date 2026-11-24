//! U-120 (D-140): the fixtures of `Fx::new`, `Fx::with_checks` and `Fx::workspace` are copied from a
//! template built once per key under a lock, and a copy holds no path of the folder it was built in.
//! `Fx::new_real()` keeps a real `bf init`.

mod common;
use common::*;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

const LIMIT: Duration = Duration::from_secs(10);

/// The files of a copy that may still hold the build folder's path: the sealed log and its head, the
/// receipts, and the signed guard table until the engine next writes it.
fn allowed(fx: &Fx, p: &Path) -> bool {
    let root = &fx.root;
    p.starts_with(root.join("factory/log"))
        || p.starts_with(root.join("factory/receipts"))
        || p == root.join(".bf/guard.json")
        || p == fx.state_dir().join("head.json")
}

/// Every file under `dir` whose bytes hold `needle`.
fn holding(dir: &Path, needle: &str, found: &mut Vec<PathBuf>) {
    for e in fs::read_dir(dir).unwrap().flatten() {
        let ft = e.file_type().unwrap();
        let p = e.path();
        if ft.is_dir() {
            holding(&p, needle, found);
        } else if ft.is_file() {
            let b = fs::read(&p).unwrap();
            if b.windows(needle.len()).any(|w| w == needle.as_bytes()) {
                found.push(p);
            }
        }
    }
}

/// The files of the copy, outside the allowed ones, that hold the build folder's path.
fn scan(fx: &Fx, build: &Path) -> Vec<PathBuf> {
    let top = fx.root.parent().unwrap();
    let mut found = Vec::new();
    holding(top, &build.to_string_lossy(), &mut found);
    found.retain(|p| !allowed(fx, p));
    found
}

fn private_store(dir: &Path, ident: &str) -> TemplateStore {
    TemplateStore::at(dir, ident)
}

fn template_names(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .flatten()
        .filter(|e| e.file_type().unwrap().is_dir())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();
    v.sort();
    v
}

#[test]
fn a_copied_project_verifies_and_holds_no_path_of_its_build_folder() {
    for kind in [
        Kind::New,
        Kind::WithChecks,
        Kind::Workspace(vec!["app".into()]),
    ] {
        let (fx, build) = TemplateStore::shared().fixture(&kind, Switch::Off);
        assert!(
            !build.exists(),
            "the build folder {} exists",
            build.display()
        );
        assert!(
            !fx.root.starts_with(&build),
            "the copy lies in the build folder"
        );
        fx.must(&["log", "--verify"]);
        let real = match &kind {
            Kind::New => Fx::new_real_with(Switch::Off),
            Kind::WithChecks => Fx::with_checks_real_with(Switch::Off),
            Kind::Workspace(_) => Fx::workspace_real_with(&["app"], Switch::Off),
        };
        let d_copy = fx.bf(&["doctor"]);
        let d_real = real.bf(&["doctor"]);
        assert_eq!(
            d_copy.code,
            d_real.code,
            "doctor differs\ncopy:\n{}\nreal:\n{}",
            d_copy.all(),
            d_real.all()
        );
        let found = scan(&fx, &build);
        assert!(
            found.is_empty(),
            "{kind:?}: files hold the build folder's path: {found:?}"
        );
        // The scan finds the path where it is planted.
        let planted = fx.root.join("src/planted.txt");
        fs::create_dir_all(planted.parent().unwrap()).unwrap();
        fs::write(&planted, format!("x {} y\n", build.display())).unwrap();
        assert_eq!(scan(&fx, &build), vec![planted.clone()]);
        fs::remove_file(&planted).unwrap();
        // The copy's state folder lives in the copy.
        assert!(fx.state_dir().starts_with(fx.root.parent().unwrap()));
        assert!(fx.state_dir().join("key").is_file());
    }
}

#[test]
fn a_copy_has_the_git_tree_of_a_real_init() {
    let status = |fx: &Fx| {
        let mut v: Vec<String> = git(&fx.root, &["status", "--porcelain"])
            .lines()
            .map(|l| l.trim().to_string())
            .collect();
        v.sort();
        v
    };
    for kind in [
        Kind::New,
        Kind::WithChecks,
        Kind::Workspace(vec!["app".into()]),
    ] {
        let (copy, _) = TemplateStore::shared().fixture(&kind, Switch::Off);
        let real = match &kind {
            Kind::New => Fx::new_real_with(Switch::Off),
            Kind::WithChecks => Fx::with_checks_real_with(Switch::Off),
            Kind::Workspace(_) => Fx::workspace_real_with(&["app"], Switch::Off),
        };
        assert_eq!(status(&copy), status(&real), "{kind:?}");
    }
}

#[test]
fn two_copies_have_their_own_state_folders() {
    let a = Fx::new_with(Switch::Off);
    let b = Fx::new_with(Switch::Off);
    assert_ne!(a.state_dir(), b.state_dir());
    assert!(a.state_dir().is_dir() && b.state_dir().is_dir());
    a.must(&["log", "--verify"]);
    b.must(&["log", "--verify"]);
}

#[test]
fn the_engine_fails_on_an_old_path() {
    // Pointing the engine back at the build folder fails, since that folder is gone.
    let (fx, build) = TemplateStore::shared().fixture(&Kind::New, Switch::Off);
    let old_state = build.join("state");
    let o = fx.bf_env(
        &fx.root,
        &["log", "--verify"],
        "",
        &[("BF_STATE_HOME", old_state.to_str().unwrap())],
    );
    assert_ne!(
        o.code,
        0,
        "the engine used a path that is gone: {}",
        o.all()
    );
}

#[test]
fn a_template_is_rebuilt_when_the_binary_identity_changes() {
    let dir = tempfile::TempDir::new().unwrap();
    let s1 = private_store(dir.path(), "binary one");
    let (_a, _) = s1.fixture(&Kind::New, Switch::Off);
    let (_b, _) = s1.fixture(&Kind::WithChecks, Switch::Off);
    let k1 = s1.key(&Kind::New, Switch::Off);
    let k1c = s1.key(&Kind::WithChecks, Switch::Off);
    assert_eq!(template_names(dir.path()), {
        let mut v = vec![k1.clone(), k1c.clone()];
        v.sort();
        v
    });
    // A temporary folder left by a builder that died is removed at the next build of its key.
    let s2 = private_store(dir.path(), "binary two");
    let k2 = s2.key(&Kind::New, Switch::Off);
    assert_ne!(k1, k2);
    let dead = dir.path().join(format!("{k2}.partial-999999"));
    fs::create_dir_all(dead.join("junk")).unwrap();
    let (c, _) = s2.fixture(&Kind::New, Switch::Off);
    c.must(&["log", "--verify"]);
    assert!(!dead.exists(), "the dead builder's folder is still there");
    assert_eq!(template_names(dir.path()), vec![k2.clone()]);
    // No template of the current binary is removed, however many kinds exist.
    let k2c = s2.key(&Kind::WithChecks, Switch::Off);
    let (_d, _) = s2.fixture(&Kind::WithChecks, Switch::Off);
    let mut want = vec![k2, k2c];
    want.sort();
    assert_eq!(template_names(dir.path()), want);
}

#[test]
fn the_switch_is_part_of_the_key() {
    let s = TemplateStore::shared();
    assert_ne!(
        s.key(&Kind::New, Switch::Off),
        s.key(&Kind::New, Switch::All)
    );
    assert_ne!(
        s.key(&Kind::New, Switch::Off),
        s.key(&Kind::WithChecks, Switch::Off)
    );
}

/// A flag one thread sets and another waits for, up to a limit.
#[derive(Default)]
struct Flag(Mutex<u32>, Condvar);

impl Flag {
    fn add(&self) {
        *self.0.lock().unwrap() += 1;
        self.1.notify_all();
    }
    fn wait_for(&self, n: u32, limit: Duration) -> bool {
        let start = Instant::now();
        let mut g = self.0.lock().unwrap();
        while *g < n {
            let left = match limit.checked_sub(start.elapsed()) {
                Some(l) => l,
                None => return false,
            };
            g = self.1.wait_timeout(g, left).unwrap().0;
        }
        true
    }
}

#[test]
fn two_copies_run_at_once() {
    let dir = tempfile::TempDir::new().unwrap();
    let ident = binary_identity();
    // The template exists before the two copies start.
    let _ = private_store(dir.path(), &ident).fixture(&Kind::New, Switch::Off);
    let inside = Arc::new(Flag::default());
    let start = Instant::now();
    let hs: Vec<_> = (0..2)
        .map(|_| {
            let inside = inside.clone();
            let d = dir.path().to_path_buf();
            let ident = ident.clone();
            std::thread::spawn(move || {
                let i2 = inside.clone();
                set_copy_pause(Some(Box::new(move || {
                    i2.add();
                    assert!(
                        i2.wait_for(2, LIMIT),
                        "the other copy did not enter its copy within 10 s"
                    );
                })));
                let (fx, _) = private_store(&d, &ident).fixture(&Kind::New, Switch::Off);
                set_copy_pause(None);
                fx.must(&["log", "--verify"]);
            })
        })
        .collect();
    for h in hs {
        h.join().expect("a copy failed");
    }
    assert!(start.elapsed() < LIMIT + LIMIT);
}

#[test]
fn a_copy_of_one_key_finishes_while_another_key_is_built() {
    let dir = tempfile::TempDir::new().unwrap();
    let ident = binary_identity();
    let _ = private_store(dir.path(), &ident).fixture(&Kind::New, Switch::Off);
    let began = Arc::new(Flag::default());
    let copied = Arc::new(Flag::default());
    let b = {
        let (began, copied) = (began.clone(), copied.clone());
        let d = dir.path().to_path_buf();
        let ident = ident.clone();
        std::thread::spawn(move || {
            set_build_pause(Some(Box::new(move || {
                began.add();
                assert!(
                    copied.wait_for(2, LIMIT),
                    "no copy of the other key finished within 10 s while this key was built"
                );
            })));
            let (fx, _) = private_store(&d, &ident).fixture(&Kind::WithChecks, Switch::Off);
            set_build_pause(None);
            fx.must(&["log", "--verify"]);
        })
    };
    assert!(began.wait_for(1, LIMIT * 6), "the build never began");
    // Two tests copy key A while key B is being built: both get whole copies.
    let hs: Vec<_> = (0..2)
        .map(|_| {
            let copied = copied.clone();
            let d = dir.path().to_path_buf();
            let ident = ident.clone();
            std::thread::spawn(move || {
                let (fx, _) = private_store(&d, &ident).fixture(&Kind::New, Switch::Off);
                fx.must(&["log", "--verify"]);
                assert!(fx.root.join("factory/config.toml").is_file());
                copied.add();
            })
        })
        .collect();
    for h in hs {
        h.join().expect("a copy failed");
    }
    b.join().expect("the build of the other key failed");
}

/// The receipt's `sandbox` value expected when the fixture asks for the sandbox.
fn expected_on() -> &'static str {
    if sandbox_probe() {
        "sandbox-exec"
    } else {
        eprintln!("sandbox_probe = false");
        "none"
    }
}

#[test]
fn a_template_with_the_switch_off_is_never_copied_for_the_switch_on() {
    let want = expected_on();
    assert_eq!(receipt_sandbox(&Fx::new_with(Switch::Off)), "none");
    assert_eq!(receipt_sandbox(&Fx::new_with(Switch::All)), want);
    assert_eq!(receipt_sandbox(&Fx::with_checks_with(Switch::Off)), "none");
    assert_eq!(receipt_sandbox(&Fx::with_checks_with(Switch::All)), want);
    assert_eq!(
        receipt_sandbox(&Fx::workspace_with(&["app"], Switch::Off)),
        "none"
    );
    assert_eq!(
        receipt_sandbox(&Fx::workspace_with(&["app"], Switch::All)),
        want
    );
}

#[test]
fn a_real_fixture_still_runs_bf_init() {
    let fx = Fx::new_real();
    let o = fx.must(&["log", "--json"]);
    assert!(o.stdout.contains("project.init"), "{}", o.stdout);
}

#[test]
fn required_sandbox_is_met_for_templates() {
    require_sandbox_from_env(&Fx::new());
    require_sandbox_from_env(&Fx::with_checks());
    require_sandbox_from_env(&Fx::workspace(&["app"]));
    require_sandbox_from_env(&Fx::new_real());
}
