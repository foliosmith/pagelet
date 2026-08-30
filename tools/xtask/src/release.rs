use std::{fs, io, path::Path, process::Command};

use super::{external, golden_check, manifest_lint, run_corpus, GoldenSelection, XtaskError};

pub(super) fn run(args: &[String]) -> Result<(), XtaskError> {
    match args {
        [action] if action == "verify" => verify(),
        [action] if action == "dry-run" => publish_dry_run(false),
        [action, flag] if action == "dry-run" && flag == "--allow-dirty" => publish_dry_run(true),
        [action, version_flag, version] if action == "publish" && version_flag == "--version" => {
            publish(version)
        }
        [] => {
            print_help();
            Ok(())
        }
        [arg] if matches!(arg.as_str(), "-h" | "--help" | "help") => {
            print_help();
            Ok(())
        }
        _ => Err(XtaskError::Usage(
            "unknown release command or options".into(),
        )),
    }
}

fn verify() -> Result<(), XtaskError> {
    verify_publish_policy()?;
    audit_unsafe()?;
    manifest_lint()?;
    external::verified_artifacts()?;
    golden_check(&GoldenSelection::All)?;
    run_corpus(&["--profile".into(), "smoke".into(), "--required".into()])?;
    println!("release policy verification passed");
    Ok(())
}

fn publish_dry_run(allow_dirty: bool) -> Result<(), XtaskError> {
    verify()?;
    let mut command = Command::new("cargo");
    command.args(["publish", "-p", "pagelet", "--dry-run", "--locked"]);
    if allow_dirty {
        command.arg("--allow-dirty");
    }
    command_success(&mut command, "cargo publish dry-run")
}

fn publish(version: &str) -> Result<(), XtaskError> {
    verify()?;
    let expected = workspace_version()?;
    if version != expected {
        return Err(XtaskError::Command(format!(
            "publish version mismatch: workspace={expected}, requested={version}"
        )));
    }
    let changelog = fs::read_to_string("CHANGELOG.md")?;
    if !changelog.contains(&format!("## {version}")) {
        return Err(XtaskError::Command(format!(
            "CHANGELOG.md is missing release heading: ## {version}"
        )));
    }
    let status = command_text(
        Command::new("git").args(["status", "--porcelain"]),
        "git status",
    )?;
    if !status.trim().is_empty() {
        return Err(XtaskError::Command(
            "formal publish requires a clean worktree".into(),
        ));
    }
    let tag = command_text(
        Command::new("git").args(["describe", "--tags", "--exact-match", "--match", "v*"]),
        "git tag verification",
    )?;
    if tag.trim() != format!("v{version}") {
        return Err(XtaskError::Command(format!(
            "formal publish requires exact tag v{version}, got {}",
            tag.trim()
        )));
    }
    command_success(
        Command::new("cargo").args(["publish", "-p", "pagelet", "--locked"]),
        "cargo publish",
    )
    .map_err(|error| {
        XtaskError::Command(format!(
            "{error}; inspect crates.io before retrying. If a bad version was published, run `cargo yank --version {version} pagelet`"
        ))
    })
}

fn verify_publish_policy() -> Result<(), XtaskError> {
    let root = fs::read_to_string("Cargo.toml")?;
    let members = workspace_members(&root)?;
    let mut publishable = Vec::new();
    for member in members {
        let manifest_path = Path::new(&member).join("Cargo.toml");
        let manifest = fs::read_to_string(&manifest_path)?;
        let name = package_name(&manifest).ok_or_else(|| {
            XtaskError::Command(format!(
                "{} is missing package.name",
                manifest_path.display()
            ))
        })?;
        if package_publish_false(&manifest) {
            continue;
        }
        publishable.push(name.to_owned());
    }
    if publishable != ["pagelet"] {
        return Err(XtaskError::Command(format!(
            "only pagelet may be publishable; found {}",
            publishable.join(", ")
        )));
    }
    let pagelet = fs::read_to_string("crates/pagelet/Cargo.toml")?;
    for key in [
        "description = ",
        "readme = ",
        "documentation = ",
        "keywords = ",
        "categories = ",
    ] {
        if !pagelet.lines().any(|line| line.trim().starts_with(key)) {
            return Err(XtaskError::Command(format!(
                "crates/pagelet/Cargo.toml is missing {key}metadata"
            )));
        }
    }
    if !fs::read_to_string("CHANGELOG.md")?.contains("## Unreleased") {
        return Err(XtaskError::Command(
            "CHANGELOG.md is missing ## Unreleased".into(),
        ));
    }
    workspace_version().map(|_| ())
}

fn workspace_members(manifest: &str) -> Result<Vec<String>, XtaskError> {
    let mut in_members = false;
    let mut members = Vec::new();
    for line in manifest.lines().map(str::trim) {
        if line.starts_with("members = [") {
            in_members = true;
            continue;
        }
        if in_members && line == "]" {
            break;
        }
        if in_members {
            let member = line.trim_end_matches(',').trim_matches('"');
            if !member.is_empty() {
                members.push(member.to_owned());
            }
        }
    }
    if members.is_empty() {
        Err(XtaskError::Command(
            "Cargo.toml workspace.members is empty or unsupported".into(),
        ))
    } else {
        Ok(members)
    }
}

fn package_name(manifest: &str) -> Option<&str> {
    let mut in_package = false;
    for line in manifest.lines().map(str::trim) {
        if line == "[package]" {
            in_package = true;
            continue;
        }
        if in_package && line.starts_with('[') {
            return None;
        }
        if in_package {
            if let Some(value) = line.strip_prefix("name = ") {
                return value.strip_prefix('"')?.strip_suffix('"');
            }
        }
    }
    None
}

fn package_publish_false(manifest: &str) -> bool {
    let mut in_package = false;
    for line in manifest.lines().map(str::trim) {
        if line == "[package]" {
            in_package = true;
            continue;
        }
        if in_package && line.starts_with('[') {
            return false;
        }
        if in_package && line == "publish = false" {
            return true;
        }
    }
    false
}

fn workspace_version() -> Result<String, XtaskError> {
    let manifest = fs::read_to_string("Cargo.toml")?;
    let mut in_workspace_package = false;
    for line in manifest.lines().map(str::trim) {
        if line == "[workspace.package]" {
            in_workspace_package = true;
            continue;
        }
        if in_workspace_package && line.starts_with('[') {
            break;
        }
        if in_workspace_package {
            if let Some(value) = line.strip_prefix("version = ") {
                return value
                    .strip_prefix('"')
                    .and_then(|value| value.strip_suffix('"'))
                    .map(str::to_owned)
                    .ok_or_else(|| XtaskError::Command("invalid workspace version".into()));
            }
        }
    }
    Err(XtaskError::Command(
        "Cargo.toml is missing workspace package version".into(),
    ))
}

fn audit_unsafe() -> Result<(), XtaskError> {
    let allowed = Path::new("crates/pagelet/src/ffi/native.rs");
    let mut files = Vec::new();
    collect_rust_files(Path::new("crates/pagelet/src"), &mut files)?;
    let mut violations = Vec::new();
    for path in files {
        let text = fs::read_to_string(&path)?;
        let lines: Vec<_> = text.lines().collect();
        let has_unsafe = text.contains("unsafe {")
            || text.contains("unsafe fn ")
            || text.contains("unsafe extern ")
            || text.contains("#![allow(unsafe_code)]");
        if has_unsafe && path != allowed {
            violations.push(format!(
                "unsafe code outside audited boundary: {}",
                path.display()
            ));
            continue;
        }
        if path == allowed {
            for (index, line) in lines.iter().enumerate() {
                if line.contains("unsafe {") {
                    let start = index.saturating_sub(3);
                    if !lines[start..index]
                        .iter()
                        .any(|previous| previous.contains("SAFETY:"))
                    {
                        violations.push(format!(
                            "{}:{} unsafe block lacks nearby SAFETY comment",
                            path.display(),
                            index + 1
                        ));
                    }
                }
            }
        }
    }
    if violations.is_empty() {
        Ok(())
    } else {
        Err(XtaskError::Command(format!(
            "unsafe audit failed:\n{}",
            violations.join("\n")
        )))
    }
}

fn collect_rust_files(path: &Path, out: &mut Vec<std::path::PathBuf>) -> Result<(), XtaskError> {
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_rust_files(&path, out)?;
        } else if path.extension().and_then(|extension| extension.to_str()) == Some("rs") {
            out.push(path);
        }
    }
    Ok(())
}

fn command_success(command: &mut Command, label: &str) -> Result<(), XtaskError> {
    let status = command.status().map_err(|error| {
        if error.kind() == io::ErrorKind::NotFound {
            XtaskError::Command(format!(
                "{label} requires {}",
                command.get_program().to_string_lossy()
            ))
        } else {
            XtaskError::Io(error)
        }
    })?;
    if status.success() {
        Ok(())
    } else {
        Err(XtaskError::Command(format!("{label} failed with {status}")))
    }
}

fn command_text(command: &mut Command, label: &str) -> Result<String, XtaskError> {
    let output = command.output()?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        Err(XtaskError::Command(format!(
            "{label} failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )))
    }
}

fn print_help() {
    println!("Usage:");
    println!("  cargo xtask release verify");
    println!("  cargo xtask release dry-run [--allow-dirty]");
    println!("  cargo xtask release publish --version <version>");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_member_parser_keeps_declared_order() {
        let members = workspace_members(
            "[workspace]\nmembers = [\n  \"crates/pagelet\",\n  \"tools/xtask\",\n]\n",
        )
        .expect("members");

        assert_eq!(members, ["crates/pagelet", "tools/xtask"]);
    }

    #[test]
    fn package_publish_policy_reads_only_package_section() {
        assert_eq!(
            package_name("[package]\nname = \"pagelet\"\n"),
            Some("pagelet")
        );
        assert!(package_publish_false(
            "[package]\nname = \"tool\"\npublish = false\n[dependencies]\n"
        ));
    }
}
