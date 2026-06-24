use assert_cmd::Command;
use predicates::prelude::*;

#[test]
fn help_lists_daily_docker_workflow_commands() {
    let mut cmd = Command::cargo_bin("dockerctl").expect("binary");

    cmd.arg("--help").assert().success().stdout(
        predicate::str::contains("Linux 日常 Docker 项目管理")
            .and(predicate::str::contains("list"))
            .and(predicate::str::contains("inbox"))
            .and(predicate::str::contains("safe-prune"))
            .and(predicate::str::contains("completion")),
    );
}

#[test]
fn demo_command_is_removed_from_public_cli() {
    let mut cmd = Command::cargo_bin("dockerctl").expect("binary");

    cmd.args(["demo", "--help"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("unrecognized subcommand"));
}

#[test]
fn completion_generates_bash_script_without_docker_daemon() {
    let mut cmd = Command::cargo_bin("dockerctl").expect("binary");

    cmd.args(["completion", "bash"])
        .assert()
        .success()
        .stdout(predicate::str::contains("dockerctl"));
}

#[test]
fn purge_help_documents_scripted_confirmation_token() {
    let mut cmd = Command::cargo_bin("dockerctl").expect("binary");

    cmd.args(["purge", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--confirm-token"));
}

#[test]
fn safe_prune_help_documents_dry_run_and_strong_confirmation() {
    let mut cmd = Command::cargo_bin("dockerctl").expect("binary");

    cmd.args(["safe-prune", "--help"])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("--dry-run")
                .and(predicate::str::contains("--yes"))
                .and(predicate::str::contains("--confirm-token")),
        );
}

#[test]
fn init_config_refuses_to_overwrite_existing_config_without_force() {
    let temp = std::env::temp_dir().join(format!("dockerctl-test-{}", std::process::id()));
    let config_dir = temp.join("config");
    let dockerctl_dir = config_dir.join("dockerctl");
    std::fs::create_dir_all(&dockerctl_dir).expect("config dir");
    std::fs::write(dockerctl_dir.join("config.toml"), "theme = \"custom\"").expect("config");

    let mut cmd = Command::cargo_bin("dockerctl").expect("binary");
    cmd.env("XDG_CONFIG_HOME", &config_dir)
        .args(["init-config"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("配置已存在"));

    let _ = std::fs::remove_dir_all(temp);
}
