use std::fs;

#[test]
fn release_metadata_targets_public_v051_repository() {
    let manifest = fs::read_to_string("Cargo.toml").expect("Cargo.toml");
    assert!(manifest.contains("name = \"hugdocker\""));
    assert!(manifest.contains("version = \"0.5.1\""));
    assert!(manifest.contains("repository = \"https://github.com/badwichell007/hugdocker\""));
    assert!(manifest.contains("homepage = \"https://github.com/badwichell007/hugdocker\""));
    assert!(
        manifest.contains("documentation = \"https://github.com/badwichell007/hugdocker#readme\"")
    );
    assert!(manifest.contains("name = \"dockerctl\""));

    let readme = fs::read_to_string("README.md").expect("README.md");
    assert!(readme.contains("badwichell007/hugdocker"));
    assert!(readme.contains("HUGDOCKER_VERSION=v0.5.1"));
    assert!(readme.contains("hugdocker inbox --json"));
    assert!(readme.contains("### v0.5.1"));
    assert!(readme.contains("hugdocker update myapp --dry-run"));
    assert!(readme.contains("hugdocker --context staging list"));
    assert!(readme.contains("hugdocker audit export --tail 500 --json"));
    assert!(readme.contains("修复 TUI Logs 面板"));
    assert!(readme.contains("Ops Inbox"));
    assert!(readme.contains("Ops Fingerprint"));

    let install = fs::read_to_string("scripts/install.sh").expect("scripts/install.sh");
    assert!(install.contains("BIN_NAME=\"hugdocker\""));
    assert!(install.contains("badwichell007/hugdocker"));
    assert!(install.contains("ln -sf \"${BIN_NAME}\" \"${DEST_DIR}/dockerctl\""));

    let config = fs::read_to_string("examples/config.toml").expect("examples/config.toml");
    assert!(config.contains("theme = \"cockpit\""));

    let release_notes =
        fs::read_to_string(".github/release-notes/v0.5.1.md").expect("release notes");
    assert!(release_notes.contains("TUI Log Viewer Fix"));
    assert!(release_notes.contains("TUI Logs now shows real logs"));
}

#[test]
fn public_release_files_do_not_reference_placeholder_repository() {
    for path in [
        "Cargo.toml",
        "README.md",
        "RELEASE_CHECKLIST.md",
        "scripts/install.sh",
    ] {
        let content = fs::read_to_string(path).expect(path);
        assert!(
            !content.contains("docker-x/dockerctl"),
            "{path} still references placeholder repository"
        );
    }
}
