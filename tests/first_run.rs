//! The free allowance cannot be enlarged by touching its file.
//!
//! The counter lives on the user's machine, so deleting it or editing it is
//! the obvious way to ask for more. Deleting it once handed out unlimited use
//! forever, because a config with no counter beside it was read as an install
//! from before metering. Now a missing, edited or foreign counter reads as a
//! day already spent, and only a machine with no files at all starts with ten.
//!
//! Driven through the real binary because the rule depends on the order two
//! files are loaded in at startup, which no unit test of either one can see.
//! Some commands here reach the network and are expected to fail without it;
//! nothing below looks at their exit status, only at what they report.

use std::path::PathBuf;
use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_bubbleTranslate");

/// A private home, so a test never reads or spends the real allowance.
///
/// `BUBBLETRANSLATE_HOME` rather than the XDG variables: those are honoured on
/// Linux and nowhere else, so on macOS and Windows a sandbox built out of them
/// is not a sandbox at all — it is the user's own install, with this test
/// deleting the counter out of it.
struct Sandbox {
    root: PathBuf,
}

impl Sandbox {
    fn new(name: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "bubbletranslate-first-run-{name}-{}",
            std::process::id(),
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("could not make a sandbox");
        Self { root }
    }

    fn run(&self, args: &[&str]) -> String {
        let out = Command::new(BIN)
            .args(args)
            .env("BUBBLETRANSLATE_HOME", &self.root)
            .output()
            .expect("could not run the binary");
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    /// How many free translations `--license` says are left today, or `None`
    /// when it reports the install as unlimited.
    fn left_today(&self) -> Option<u32> {
        let report = self.run(&["--license"]);
        assert!(
            report.contains("allowance"),
            "--license no longer reports the allowance; this test needs rewriting:\n{report}",
        );
        if report.contains("allowance unlimited") {
            return None;
        }
        let line = report
            .lines()
            .find(|l| l.starts_with("used"))
            .expect("no `used` line");
        let left = line
            .split(", ")
            .nth(1)
            .expect("no `left` in the `used` line");
        Some(left.split_whitespace().next().unwrap().parse().unwrap())
    }

    fn config(&self) -> PathBuf {
        self.root.join("config.toml")
    }

    fn usage(&self) -> PathBuf {
        self.root.join("usage.json")
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// Every command that creates the config must create the counter too, and a
/// brand-new install starts metered with its full ten.
#[test]
fn a_new_install_starts_with_ten() {
    for (name, args) in [
        ("license", vec!["--license"]),
        ("check", vec!["--check"]),
        ("translate", vec!["--translate", "merhaba"]),
    ] {
        let box_ = Sandbox::new(name);
        box_.run(&args);
        assert!(box_.config().exists(), "`{name}` did not write a config");
        assert!(
            box_.usage().exists(),
            "`{name}` wrote a config with no counter"
        );
        // `--translate` may have spent one if the network was there.
        let left = box_.left_today().expect("a new install is unlimited");
        assert!(left >= 9, "`{name}` left a new install with {left}");
    }
}

/// Deleting the counter used to grant unlimited use forever. It now grants
/// nothing: today reads as spent, and the install stays metered.
#[test]
fn deleting_the_counter_buys_nothing() {
    let box_ = Sandbox::new("deleted");
    assert_eq!(box_.left_today(), Some(10));
    std::fs::remove_file(box_.usage()).expect("no counter to remove");
    assert_eq!(
        box_.left_today(),
        Some(0),
        "deleting the counter bought translations"
    );
}

/// Editing the counter by hand breaks its seal, which also reads as spent.
#[test]
fn editing_the_counter_buys_nothing() {
    let box_ = Sandbox::new("edited");
    assert_eq!(box_.left_today(), Some(10));
    let raw = std::fs::read_to_string(box_.usage()).unwrap();
    let edited = raw.replacen("\"used\": 0", "\"used\": 3", 1);
    assert_ne!(
        raw, edited,
        "the counter's shape changed; this test needs rewriting"
    );
    std::fs::write(box_.usage(), edited).unwrap();
    assert_eq!(box_.left_today(), Some(0), "an edited counter was trusted");

    // And the old flag that once meant "unlimited" means nothing now.
    let raw = std::fs::read_to_string(box_.usage()).unwrap();
    let flagged = raw.replacen("{", "{\n  \"legacy_unlimited\": true,", 1);
    std::fs::write(box_.usage(), flagged).unwrap();
    assert_eq!(
        box_.left_today(),
        Some(0),
        "the legacy flag still grants unlimited use"
    );
}

/// An honest counter, written and read back on the same machine, keeps its
/// count across launches -- the seal must not cost anyone who left it alone.
#[test]
fn an_untouched_counter_is_trusted() {
    let box_ = Sandbox::new("untouched");
    assert_eq!(box_.left_today(), Some(10));
    assert_eq!(box_.left_today(), Some(10));
}
