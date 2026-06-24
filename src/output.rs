use std::io::{self, Write};

use serde::Serialize;

use crate::AppResult;
use crate::domain::Project;

pub fn print_json<T: Serialize>(value: &T) -> AppResult<()> {
    let stdout = io::stdout();
    let mut lock = stdout.lock();
    serde_json::to_writer_pretty(&mut lock, value)?;
    writeln!(lock)?;
    Ok(())
}

pub fn project_line(project: &Project) -> String {
    format!(
        "[{}|{:?}] {:<24} active {:>2}/{:<2} stopped {:>2} networks {:>2} volumes {:>2}",
        project.state_code(),
        project.kind,
        project.name,
        project.active(),
        project.containers.len(),
        project.stopped,
        project.networks.len(),
        project.volumes.len()
    )
}

pub fn print_projects(projects: &[Project]) {
    for project in projects {
        println!("{}", project_line(project));
    }
}
