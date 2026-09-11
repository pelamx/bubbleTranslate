//! A fresh install must never be born unlimited.
//!
//! The counter treats "config.toml exists but usage.json does not" as an
//! install that predates metering and grants it unlimited use forever. That
//! rule is right for a genuine upgrade and catastrophic for a new install, so
//! every command that writes the config has to write the counter in the same
//! breath.
//!
//! `--check` and `--translate` once wrote only the config, which meant the
//! next launch grandfathered itself. `--check` is the command the installer
//! prints at the end, so this was the ordinary first thing a user ran rather
//! than an obscure corner.
//!
//! Driven through the real binary because the bug lived in the order two
//! loads happen in at startup, which no unit test of either one can see. The
//! commands here reach the network and are expected to fail without it;
//! nothing below looks at their exit status, only at what they left on disk.

use std::path::{Path, PathBuf};
use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_bubbleTranslate");

/// A private XDG home, so a test never reads or spends the real allowance.
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

    fn run(&self, args: &[&str]) {
        Command::new(BIN)
            .args(args)
            .env("XDG_CONFIG_HOME", self.root.join("config"))
            .env("XDG_DATA_HOME", self.root.join("data"))
            .output()
            .expect("could not run the binary");
    }

    fn config(&self) -> PathBuf {
        self.root.join("config/bubbleTranslate/config.toml")
    }

    fn usage(&self) -> PathBuf {
        self.root.join("data/bubbleTranslate/usage.json")
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn grandfathered(usage: &Path) -> bool {
    let raw = std::fs::read_to_string(usage).expect("no counter was written");
    // Matched as text rather than parsed, so this test keeps working if the
    // counter grows fields, and fails loudly if the flag is renamed.
    assert!(
        raw.contains("legacy_unlimited"),
        "the counter has no legacy_unlimited field; this test needs rewriting",
    );
    raw.contains("\"legacy_unlimited\": true")
}

/// Every command that creates the config must create the counter too. If one
/// of these leaves the counter missing, the *next* launch reads the lone
/// config file as a pre-metering install and hands out unlimited use.
#[test]
fn no_command_leaves_the_config_without_a_counter() {
    for (name, args) in [
        ("license", vec!["--license"]),
        ("check", vec!["--check"]),
        ("translate", vec!["--translate", "merhaba"]),
    ] {
        let box_ = Sandbox::new(name);
        box_.run(&args);

        assert!(
            box_.config().exists(),
            "`{name}` did not write a config; this test is no longer exercising the bug",
        );
        assert!(
            box_.usage().exists(),
            "`{name}` wrote a config with no counter — the next launch will grandfather itself",
        );
        assert!(
            !grandfathered(&box_.usage()),
            "`{name}` produced a brand-new install that is already unlimited",
        );
    }
}

/// The installer prints `--check` as the thing to run first, so this is the
/// exact sequence a new user follows. It must end on the free tier.
#[test]
fn the_sequence_the_installer_prints_stays_metered() {
    let box_ = Sandbox::new("installer");
    box_.run(&["--check"]);
    box_.run(&["--license"]);

    assert!(
        !grandfathered(&box_.usage()),
        "checking the backends before the first launch bought a free upgrade",
    );
}

/// The rule this all rests on still has to work the way it was meant to: an
/// install carrying a config from before metering, with no counter beside it,
/// keeps its unlimited use. Fixing the bug above must not take that away.
#[test]
fn a_genuine_pre_metering_install_keeps_its_unlimited_use() {
    let box_ = Sandbox::new("legacy");
    box_.run(&["--license"]);
    // What such an install looks like: the config it has always had, and no
    // counter, because the build that wrote the config had none to write.
    std::fs::remove_file(box_.usage()).expect("no counter to remove");

    box_.run(&["--license"]);
    assert!(
        grandfathered(&box_.usage()),
        "an install that predates the allowance lost its unlimited use",
    );
}
