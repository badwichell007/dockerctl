use std::fs;

#[test]
fn release_metadata_targets_public_v022_repository() {
    let manifest = fs::read_to_string("Cargo.toml").expect("Cargo.toml");
    assert!(manifest.contains("version = \"0.2.2\""));
    assert!(manifest.contains("repository = \"https://github.com/badwichell007/dockerctl\""));
    assert!(manifest.contains("homepage = \"https://github.com/badwichell007/dockerctl\""));
    assert!(
        manifest.contains("documentation = \"https://github.com/badwichell007/dockerctl#readme\"")
    );

    let readme = fs::read_to_string("README.md").expect("README.md");
    assert!(readme.contains("badwichell007/dockerctl"));
    assert!(readme.contains("DOCKERCTL_VERSION=v0.2.2"));
    assert!(readme.contains("dockerctl demo"));
    assert!(readme.contains("Ops Inbox"));

    let install = fs::read_to_string("scripts/install.sh").expect("scripts/install.sh");
    assert!(install.contains("REPO=\"${DOCKERCTL_REPO:-badwichell007/dockerctl}\""));

    let config = fs::read_to_string("examples/config.toml").expect("examples/config.toml");
    assert!(config.contains("theme = \"cockpit\""));

    let sha = fs::read_to_string("dockerctl-0.2.2-source.tar.gz.sha256")
        .expect("dockerctl-0.2.2-source.tar.gz.sha256");
    assert!(sha.contains("dockerctl-0.2.2-source.tar.gz"));
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
