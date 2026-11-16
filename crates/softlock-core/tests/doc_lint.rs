//! Doc lint. README and CONTRIBUTING keep the sentences the spec requires.

mod common;

use std::fs;

const FAIL_CLOSED: &str = "If SoftLock fails while locked, it switches to the real Windows lock, and your agents will pause until you sign in.";
const SMARTSCREEN: &str = "More info → Run anyway";
const OPEN_CORE: &str = "Planned paid features are never added to this repository. That includes allowlists, rules, fleet management, remote triggers, and the command-line tool.";
const DCO: &str =
    "A DCO sign-off (Signed-off-by) becomes required once outside pull requests start.";

#[test]
fn readme_states_fail_closed_and_the_unsigned_smartscreen_step() {
    let readme = read_repo("README.md");
    assert!(
        readme.contains("pre-release"),
        "README must say the project is pre-release"
    );
    assert!(
        readme.contains("unsigned"),
        "README must say builds are unsigned"
    );
    assert!(
        readme.contains(FAIL_CLOSED),
        "README is missing the fail-closed sentence"
    );
    assert!(
        readme.contains(SMARTSCREEN),
        "README is missing the SmartScreen note"
    );
}

#[test]
fn contributing_states_the_open_core_rule_and_the_dco_note() {
    let contributing = read_repo("CONTRIBUTING.md");
    assert!(
        contributing.contains(OPEN_CORE),
        "CONTRIBUTING is missing the open-core rule"
    );
    assert!(
        contributing.contains(DCO),
        "CONTRIBUTING is missing the DCO note"
    );
}

fn read_repo(name: &str) -> String {
    let path = common::repo_root().join(name);
    fs::read_to_string(&path).unwrap_or_else(|error| panic!("missing {}: {error}", path.display()))
}
