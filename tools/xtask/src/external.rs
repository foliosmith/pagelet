use std::{
    env, fs, io,
    io::Read,
    path::{Path, PathBuf},
    process::Command,
};

use sha2::{Digest, Sha256};

use super::{require_schema_version, strip_toml_comment, toml_string, XtaskError};

const EXTERNAL_MANIFEST_PATH: &str = "tests/corpus-manifest.toml";
const DEFAULT_EXTERNAL_ROOT: &str = "target/pagelet-external";

pub(super) fn run(args: &[String]) -> Result<(), XtaskError> {
    match args {
        [action, locked] if action == "sync" && locked == "--locked" => external_sync(),
        [action] if action == "sync" => Err(XtaskError::Usage(
            "external sync requires --locked to prevent floating downloads".into(),
        )),
        [action] if action == "verify" => external_verify(),
        [] => {
            print_help();
            Ok(())
        }
        [action] if matches!(action.as_str(), "-h" | "--help" | "help") => {
            print_help();
            Ok(())
        }
        [action, ..] => Err(XtaskError::Usage(format!(
            "unknown external command or options: {action}"
        ))),
    }
}

pub(super) fn lint_manifest(path: &Path) -> Result<(), XtaskError> {
    let text = fs::read_to_string(path)?;
    parse_external_manifest(path, &text).map(|_| ())
}

fn external_sync() -> Result<(), XtaskError> {
    let manifest = read_external_manifest()?;
    let root = external_root();
    fs::create_dir_all(&root)?;

    for artifact in manifest.artifacts() {
        sync_artifact(&root, artifact)?;
    }

    print_verified_summary(&manifest, &root, "synced")
}

fn external_verify() -> Result<(), XtaskError> {
    let manifest = read_external_manifest()?;
    let root = external_root();
    for artifact in manifest.artifacts() {
        verify_artifact(&root.join(&artifact.file_name), artifact)?;
    }
    print_verified_summary(&manifest, &root, "verified")
}

fn read_external_manifest() -> Result<ExternalManifest, XtaskError> {
    let path = Path::new(EXTERNAL_MANIFEST_PATH);
    let text = fs::read_to_string(path)?;
    parse_external_manifest(path, &text)
}

fn external_root() -> PathBuf {
    env::var_os("PAGELET_EXTERNAL_ROOT")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_EXTERNAL_ROOT))
}

fn sync_artifact(root: &Path, artifact: &ExternalArtifact) -> Result<(), XtaskError> {
    let path = root.join(&artifact.file_name);
    if path.exists() {
        verify_artifact(&path, artifact)?;
        println!("external artifact={} status=already-verified", artifact.id);
        return Ok(());
    }

    let temporary = root.join(format!(".{}.part", artifact.file_name));
    let resuming = temporary.exists();

    let status = Command::new("curl")
        .args([
            "--fail",
            "--location",
            "--silent",
            "--show-error",
            "--header",
            "Accept: application/octet-stream",
            "--header",
            "User-Agent: pagelet-xtask/0.1",
            "--retry",
            "3",
            "--connect-timeout",
            "30",
            "--proto",
            "=https",
            "--proto-redir",
            "=https",
            "--continue-at",
            "-",
            "--output",
        ])
        .arg(&temporary)
        .arg(&artifact.url)
        .status()
        .map_err(|error| {
            if error.kind() == io::ErrorKind::NotFound {
                XtaskError::Command(
                    "external sync requires curl to download pinned artifacts".into(),
                )
            } else {
                XtaskError::Io(error)
            }
        })?;

    if !status.success() {
        return Err(XtaskError::Command(format!(
            "failed to download external artifact {}; partial download retained for retry",
            artifact.id,
        )));
    }

    if let Err(error) = verify_artifact(&temporary, artifact) {
        remove_file_if_present(&temporary)?;
        return Err(error);
    }
    fs::rename(&temporary, &path)?;
    println!(
        "external artifact={} status={}",
        artifact.id,
        if resuming { "resumed" } else { "downloaded" }
    );
    Ok(())
}

fn verify_artifact(path: &Path, artifact: &ExternalArtifact) -> Result<(), XtaskError> {
    let file = fs::File::open(path).map_err(|error| {
        if error.kind() == io::ErrorKind::NotFound {
            XtaskError::Command(format!(
                "external artifact {} is missing; run cargo xtask external sync --locked",
                artifact.id
            ))
        } else {
            XtaskError::Io(error)
        }
    })?;
    let actual = sha256_reader_hex(file)?;
    if actual != artifact.sha256 {
        return Err(XtaskError::Command(format!(
            "external artifact {} sha256 mismatch: expected {}, actual {}",
            artifact.id, artifact.sha256, actual
        )));
    }
    Ok(())
}

fn sha256_reader_hex(mut reader: impl Read) -> Result<String, XtaskError> {
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = reader.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn remove_file_if_present(path: &Path) -> Result<(), XtaskError> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn print_verified_summary(
    manifest: &ExternalManifest,
    root: &Path,
    status: &str,
) -> Result<(), XtaskError> {
    let root = root
        .to_str()
        .ok_or_else(|| XtaskError::Command("external root is not valid UTF-8".into()))?;
    println!(
        "external status={status} root={root} target_profile={} w3c_commit={} epubcheck_version={}",
        manifest.target_profile, manifest.w3c_commit, manifest.epubcheck_version
    );
    Ok(())
}

fn parse_external_manifest(path: &Path, text: &str) -> Result<ExternalManifest, XtaskError> {
    let path_label = path.to_string_lossy();
    require_schema_version(&path_label, text)?;
    let mut section = None;
    let mut draft = ExternalManifestDraft::default();

    for (index, raw_line) in text.lines().enumerate() {
        let line_number = index + 1;
        let line = strip_toml_comment(raw_line).trim();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('[') {
            section = (line == "[standards]").then_some("standards");
            continue;
        }
        if section != Some("standards") {
            continue;
        }

        let Some((key, value)) = line.split_once('=') else {
            return Err(XtaskError::Command(format!(
                "{}:{line_number} expected key = value",
                path.display()
            )));
        };
        let key = key.trim();
        if !EXTERNAL_KEYS.contains(&key) {
            continue;
        }
        let value = toml_string(path, line_number, value.trim())?;
        draft.set(path, line_number, key, value)?;
    }

    draft.finish(path)
}

const EXTERNAL_KEYS: [&str; 10] = [
    "target_profile",
    "w3c_commit",
    "w3c_archive",
    "w3c_url",
    "w3c_sha256",
    "epubcheck_version",
    "epubcheck_profile",
    "epubcheck_archive",
    "epubcheck_url",
    "epubcheck_sha256",
];

#[derive(Debug, Clone, Eq, PartialEq)]
struct ExternalManifest {
    target_profile: String,
    w3c_commit: String,
    w3c: ExternalArtifact,
    epubcheck_version: String,
    epubcheck_profile: String,
    epubcheck: ExternalArtifact,
}

impl ExternalManifest {
    fn artifacts(&self) -> [&ExternalArtifact; 2] {
        [&self.w3c, &self.epubcheck]
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct ExternalArtifact {
    id: &'static str,
    file_name: String,
    url: String,
    sha256: String,
}

#[derive(Debug, Default)]
struct ExternalManifestDraft {
    target_profile: Option<String>,
    w3c_commit: Option<String>,
    w3c_archive: Option<String>,
    w3c_url: Option<String>,
    w3c_sha256: Option<String>,
    epubcheck_version: Option<String>,
    epubcheck_profile: Option<String>,
    epubcheck_archive: Option<String>,
    epubcheck_url: Option<String>,
    epubcheck_sha256: Option<String>,
}

impl ExternalManifestDraft {
    fn set(
        &mut self,
        path: &Path,
        line_number: usize,
        key: &str,
        value: String,
    ) -> Result<(), XtaskError> {
        let slot = match key {
            "target_profile" => &mut self.target_profile,
            "w3c_commit" => &mut self.w3c_commit,
            "w3c_archive" => &mut self.w3c_archive,
            "w3c_url" => &mut self.w3c_url,
            "w3c_sha256" => &mut self.w3c_sha256,
            "epubcheck_version" => &mut self.epubcheck_version,
            "epubcheck_profile" => &mut self.epubcheck_profile,
            "epubcheck_archive" => &mut self.epubcheck_archive,
            "epubcheck_url" => &mut self.epubcheck_url,
            "epubcheck_sha256" => &mut self.epubcheck_sha256,
            _ => return Ok(()),
        };
        if slot.replace(value).is_some() {
            return Err(XtaskError::Command(format!(
                "{}:{line_number} duplicate standards key: {key}",
                path.display()
            )));
        }
        Ok(())
    }

    fn finish(self, path: &Path) -> Result<ExternalManifest, XtaskError> {
        let target_profile = required(path, "target_profile", self.target_profile)?;
        if target_profile.trim().is_empty() {
            return Err(invalid_manifest(path, "target_profile must not be empty"));
        }

        let w3c_commit = required(path, "w3c_commit", self.w3c_commit)?;
        validate_lower_hex(path, "w3c_commit", &w3c_commit, 40)?;
        let w3c = ExternalArtifact {
            id: "w3c-epub-tests",
            file_name: required(path, "w3c_archive", self.w3c_archive)?,
            url: required(path, "w3c_url", self.w3c_url)?,
            sha256: required(path, "w3c_sha256", self.w3c_sha256)?,
        };

        let epubcheck_version = required(path, "epubcheck_version", self.epubcheck_version)?;
        if epubcheck_version.trim().is_empty() {
            return Err(invalid_manifest(
                path,
                "epubcheck_version must not be empty",
            ));
        }
        let epubcheck_profile = required(path, "epubcheck_profile", self.epubcheck_profile)?;
        if epubcheck_profile.trim().is_empty() {
            return Err(invalid_manifest(
                path,
                "epubcheck_profile must not be empty",
            ));
        }
        let epubcheck = ExternalArtifact {
            id: "epubcheck",
            file_name: required(path, "epubcheck_archive", self.epubcheck_archive)?,
            url: required(path, "epubcheck_url", self.epubcheck_url)?,
            sha256: required(path, "epubcheck_sha256", self.epubcheck_sha256)?,
        };

        validate_artifact(path, &w3c, &w3c_commit)?;
        validate_artifact(path, &epubcheck, &epubcheck_version)?;

        Ok(ExternalManifest {
            target_profile,
            w3c_commit,
            w3c,
            epubcheck_version,
            epubcheck_profile,
            epubcheck,
        })
    }
}

fn required(path: &Path, key: &str, value: Option<String>) -> Result<String, XtaskError> {
    value.ok_or_else(|| invalid_manifest(path, &format!("missing standards key: {key}")))
}

fn validate_artifact(
    path: &Path,
    artifact: &ExternalArtifact,
    pinned_version: &str,
) -> Result<(), XtaskError> {
    validate_file_name(path, &artifact.file_name)?;
    validate_lower_hex(
        path,
        &format!("{}_sha256", artifact.id),
        &artifact.sha256,
        64,
    )?;
    if artifact.sha256.bytes().all(|byte| byte == b'0') {
        return Err(invalid_manifest(
            path,
            &format!("{} sha256 must not be a placeholder", artifact.id),
        ));
    }
    if !artifact.url.starts_with("https://") {
        return Err(invalid_manifest(
            path,
            &format!("{} URL must use https", artifact.id),
        ));
    }
    if artifact.url.contains("latest") || !artifact.url.contains(pinned_version) {
        return Err(invalid_manifest(
            path,
            &format!(
                "{} URL must contain pinned version {} and must not use latest",
                artifact.id, pinned_version
            ),
        ));
    }
    Ok(())
}

fn validate_file_name(path: &Path, file_name: &str) -> Result<(), XtaskError> {
    let file_path = Path::new(file_name);
    if file_name.is_empty()
        || file_path.is_absolute()
        || file_path.components().count() != 1
        || file_name.contains(['/', '\\'])
        || file_name == "."
        || file_name == ".."
    {
        return Err(invalid_manifest(
            path,
            "external archive must be a safe file name",
        ));
    }
    Ok(())
}

fn validate_lower_hex(
    path: &Path,
    key: &str,
    value: &str,
    length: usize,
) -> Result<(), XtaskError> {
    if value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        Ok(())
    } else {
        Err(invalid_manifest(
            path,
            &format!("{key} must be {length} lowercase hexadecimal characters"),
        ))
    }
}

fn invalid_manifest(path: &Path, message: &str) -> XtaskError {
    XtaskError::Command(format!("{} {message}", path.display()))
}

fn print_help() {
    println!("Usage:");
    println!("  cargo xtask external sync --locked");
    println!("  cargo xtask external verify");
    println!();
    println!("Environment:");
    println!("  PAGELET_EXTERNAL_ROOT  Artifact directory (default: {DEFAULT_EXTERNAL_ROOT})");
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    const W3C_COMMIT: &str = "d707d58cec8518d3cb7cbbe061c8be444cf1ed24";
    const W3C_SHA256: &str = "4d0f257acd4f441bbbc1a7c7e6b6c7dac60f250008f2bd25dd150af2dcdfcdfa";
    const EPUBCHECK_SHA256: &str =
        "6c07e68584b2e2ce2f89fe06e1246dfead3eb36b46b340e7d93524f29dcff6c5";

    #[test]
    fn manifest_parses_versioned_external_artifacts() {
        let manifest = parse_external_manifest(Path::new("manifest.toml"), &manifest_text())
            .expect("external manifest");

        assert_eq!(manifest.target_profile, "epub-3.3-project-baseline");
        assert_eq!(manifest.w3c_commit, W3C_COMMIT);
        assert_eq!(manifest.w3c.sha256, W3C_SHA256);
        assert_eq!(manifest.epubcheck_version, "5.3.0");
        assert_eq!(manifest.epubcheck_profile, "EPUB 3.3");
    }

    #[test]
    fn manifest_rejects_floating_external_url() {
        let text = manifest_text().replace(
            &format!("https://example.test/epub-tests/{W3C_COMMIT}.zip"),
            "https://example.test/epub-tests/latest.zip",
        );

        let error = parse_external_manifest(Path::new("manifest.toml"), &text)
            .expect_err("floating URL must fail");

        assert!(error.to_string().contains("must contain pinned version"));
    }

    #[test]
    fn artifact_verification_rejects_sha256_mismatch() {
        let root = unique_temp_dir();
        fs::create_dir_all(&root).expect("create temp root");
        let artifact = ExternalArtifact {
            id: "fixture",
            file_name: "fixture.zip".into(),
            url: "https://example.test/fixture/1.0.zip".into(),
            sha256: sha256_reader_hex(&b"expected"[..]).expect("hash fixture"),
        };
        let path = root.join(&artifact.file_name);
        fs::write(&path, b"corrupt").expect("write fixture");

        let error = verify_artifact(&path, &artifact).expect_err("checksum must fail");

        assert!(error.to_string().contains("sha256 mismatch"));
        fs::remove_dir_all(root).expect("remove temp root");
    }

    #[test]
    fn artifact_verification_accepts_matching_sha256() {
        let root = unique_temp_dir();
        fs::create_dir_all(&root).expect("create temp root");
        let artifact = ExternalArtifact {
            id: "fixture",
            file_name: "fixture.zip".into(),
            url: "https://example.test/fixture/1.0.zip".into(),
            sha256: sha256_reader_hex(&b"expected"[..]).expect("hash fixture"),
        };
        let path = root.join(&artifact.file_name);
        fs::write(&path, b"expected").expect("write fixture");

        verify_artifact(&path, &artifact).expect("matching checksum");

        fs::remove_dir_all(root).expect("remove temp root");
    }

    #[test]
    fn sync_requires_locked_flag() {
        let error = run(&["sync".into()]).expect_err("unlocked sync must fail");

        assert!(error.to_string().contains("requires --locked"));
    }

    fn manifest_text() -> String {
        format!(
            r#"
schema_version = 1

[standards]
target_profile = "epub-3.3-project-baseline"
w3c_commit = "{W3C_COMMIT}"
w3c_archive = "w3c.zip"
w3c_url = "https://example.test/epub-tests/{W3C_COMMIT}.zip"
w3c_sha256 = "{W3C_SHA256}"
epubcheck_version = "5.3.0"
epubcheck_profile = "EPUB 3.3"
epubcheck_archive = "epubcheck.zip"
epubcheck_url = "https://example.test/epubcheck/5.3.0.zip"
epubcheck_sha256 = "{EPUBCHECK_SHA256}"

[[books]]
id = "generated/minimal-epub3"
"#
        )
    }

    fn unique_temp_dir() -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time")
            .as_nanos();
        env::temp_dir().join(format!(
            "pagelet-external-test-{}-{nonce}",
            std::process::id()
        ))
    }
}
