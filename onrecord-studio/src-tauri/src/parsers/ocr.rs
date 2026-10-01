use std::ffi::{OsStr, OsString};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use uuid::Uuid;

use crate::error::AppError;

const MAX_OCR_INPUT_BYTES: u64 = 512 * 1024 * 1024;
const MAX_OCR_OUTPUT_BYTES: usize = 128 * 1024 * 1024;
const MAX_PROCESS_STDERR_BYTES: usize = 256 * 1024;
const MAX_PDF_OCR_PAGES: usize = 25_000;
const MAX_RASTER_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const DEFAULT_OCR_TIMEOUT_SECONDS: u64 = 900;
const MAX_OCR_TIMEOUT_SECONDS: u64 = 86_400;
const OCR_CAPABILITY_PROBE_TIMEOUT_SECONDS: u64 = 5;
const MAXIMUM_REPORTED_LANGUAGE_PACKS: usize = 512;

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OcrCapability {
    pub engine: String,
    pub engine_available: bool,
    pub engine_version: Option<String>,
    pub language_packs: Vec<String>,
    pub language_probe_complete: bool,
    pub pdf_renderer: Option<String>,
    pub pdf_renderer_available: bool,
    pub pdf_renderer_version: Option<String>,
    pub image_ocr_available: bool,
    pub pdf_ocr_available: bool,
    pub diagnostics: Vec<String>,
}

/// Package-manager prefixes searched after PATH. An app launched from the Finder inherits launchd's
/// PATH (`/usr/bin:/bin:/usr/sbin:/sbin`), which never contains them, so without this a Homebrew
/// or MacPorts Tesseract would be reported unavailable in every installed build.
const FALLBACK_BIN_DIRS: &[&str] = &["/opt/homebrew/bin", "/usr/local/bin", "/opt/local/bin"];

fn program_command(program: &OsStr) -> Command {
    Command::new(resolve_program(
        program,
        std::env::var_os("PATH").as_deref(),
        FALLBACK_BIN_DIRS.iter().map(Path::new),
    ))
}

/// A bare program name is resolved against PATH and then the fallback directories; the first
/// existing executable wins. An explicit path, or a name found nowhere, is returned unchanged so
/// the spawn reports the failure itself.
fn resolve_program<'a>(
    program: &OsStr,
    path: Option<&OsStr>,
    fallbacks: impl IntoIterator<Item = &'a Path>,
) -> OsString {
    if program.is_empty() || Path::new(program).components().count() != 1 {
        return program.to_os_string();
    }
    let path_dirs = path.map(std::env::split_paths).into_iter().flatten();
    path_dirs
        .chain(fallbacks.into_iter().map(Path::to_path_buf))
        .map(|dir| dir.join(program))
        .find(|candidate| is_executable_file(candidate))
        .map_or_else(|| program.to_os_string(), PathBuf::into_os_string)
}

fn is_executable_file(path: &Path) -> bool {
    let Ok(metadata) = std::fs::metadata(path) else {
        return false;
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        metadata.is_file()
    }
}

pub fn probe_ocr_capability() -> OcrCapability {
    if let Some(engine) = std::env::var_os("ONRECORD_OCR_ENGINE") {
        let probe = probe_executable(&engine, &["--version"]);
        return OcrCapability {
            engine: engine.to_string_lossy().into_owned(),
            engine_available: probe.available,
            engine_version: probe.version,
            language_packs: Vec::new(),
            language_probe_complete: false,
            pdf_renderer: None,
            pdf_renderer_available: false,
            pdf_renderer_version: None,
            image_ocr_available: probe.available,
            pdf_ocr_available: probe.available,
            diagnostics: probe.diagnostic.into_iter().collect(),
        };
    }

    let engine = OsString::from("tesseract");
    let engine_probe = probe_executable(&engine, &["--version"]);
    let language_probe = probe_language_packs(&engine);
    let renderer =
        std::env::var_os("ONRECORD_PDF_RENDERER").unwrap_or_else(|| OsString::from("pdftoppm"));
    let renderer_probe = probe_executable(&renderer, &["-v"]);
    let mut diagnostics = Vec::new();
    diagnostics.extend(engine_probe.diagnostic);
    diagnostics.extend(language_probe.diagnostic);
    diagnostics.extend(renderer_probe.diagnostic);
    let english_available = language_probe
        .languages
        .iter()
        .any(|language| language == "eng");
    if language_probe.complete && !english_available {
        diagnostics.push(
            "Tesseract's default 'eng' language pack is not installed; default OCR cannot run"
                .to_string(),
        );
    }
    let image_ocr_available = engine_probe.available && english_available;
    let pdf_ocr_available = image_ocr_available && renderer_probe.available;
    if !engine_probe.available || !renderer_probe.available {
        diagnostics.push(
            "Install both tools with `brew install tesseract poppler` (or MacPorts), then relaunch"
                .to_string(),
        );
    }
    OcrCapability {
        engine: "tesseract".to_string(),
        engine_available: engine_probe.available,
        engine_version: engine_probe.version,
        language_packs: language_probe.languages,
        language_probe_complete: language_probe.complete,
        pdf_renderer: Some(renderer.to_string_lossy().into_owned()),
        pdf_renderer_available: renderer_probe.available,
        pdf_renderer_version: renderer_probe.version,
        image_ocr_available,
        pdf_ocr_available,
        diagnostics,
    }
}

struct ExecutableProbe {
    available: bool,
    version: Option<String>,
    diagnostic: Option<String>,
}

struct LanguageProbe {
    languages: Vec<String>,
    complete: bool,
    diagnostic: Option<String>,
}

fn probe_executable(program: &OsStr, arguments: &[&str]) -> ExecutableProbe {
    let mut command = program_command(program);
    command.args(arguments);
    match run_command(
        &mut command,
        Instant::now() + Duration::from_secs(OCR_CAPABILITY_PROBE_TIMEOUT_SECONDS),
        "OCR_CAPABILITY_PROBE",
        Path::new("startup capability probe"),
    ) {
        Ok(output) => {
            let version =
                first_report_line(&output.stdout).or_else(|| first_report_line(&output.stderr));
            ExecutableProbe {
                // Reaching an exit status proves the configured executable launched. A non-zero
                // version probe remains diagnostic because custom engines may not implement that
                // optional flag even though they accept the document-path contract.
                available: true,
                version,
                diagnostic: (!output.status.success()).then(|| {
                    format!(
                        "'{} {}' exited with {}",
                        program.to_string_lossy(),
                        arguments.join(" "),
                        output.status
                    )
                }),
            }
        }
        Err(error) => ExecutableProbe {
            available: false,
            version: None,
            diagnostic: Some(error.to_string()),
        },
    }
}

fn probe_language_packs(engine: &OsStr) -> LanguageProbe {
    let mut command = program_command(engine);
    command.arg("--list-langs");
    match run_command(
        &mut command,
        Instant::now() + Duration::from_secs(OCR_CAPABILITY_PROBE_TIMEOUT_SECONDS),
        "OCR_LANGUAGE_PROBE",
        Path::new("startup capability probe"),
    ) {
        Ok(output) if output.status.success() => {
            let report = String::from_utf8_lossy(&output.stdout);
            let mut languages = report
                .lines()
                .map(str::trim)
                .filter(|line| {
                    !line.is_empty()
                        && !line
                            .to_ascii_lowercase()
                            .starts_with("list of available languages")
                })
                .take(MAXIMUM_REPORTED_LANGUAGE_PACKS + 1)
                .map(str::to_string)
                .collect::<Vec<_>>();
            if languages.len() > MAXIMUM_REPORTED_LANGUAGE_PACKS {
                languages.truncate(MAXIMUM_REPORTED_LANGUAGE_PACKS);
                return LanguageProbe {
                    languages,
                    complete: false,
                    diagnostic: Some(format!(
                        "OCR language-pack reporting exceeded {MAXIMUM_REPORTED_LANGUAGE_PACKS} entries"
                    )),
                };
            }
            LanguageProbe {
                languages,
                complete: true,
                diagnostic: None,
            }
        }
        Ok(output) => LanguageProbe {
            languages: Vec::new(),
            complete: false,
            diagnostic: Some(format!(
                "'{} --list-langs' exited with {}",
                engine.to_string_lossy(),
                output.status
            )),
        },
        Err(error) => LanguageProbe {
            languages: Vec::new(),
            complete: false,
            diagnostic: Some(error.to_string()),
        },
    }
}

fn first_report_line(bytes: &[u8]) -> Option<String> {
    String::from_utf8_lossy(bytes)
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(|line| line.chars().take(512).collect())
}

pub fn execute_local_ocr_scan<P: AsRef<Path>>(document_path: P) -> Result<String, AppError> {
    let path = document_path.as_ref();
    validate_input(path)?;
    let deadline = Instant::now() + ocr_timeout();

    if let Some(engine) = std::env::var_os("ONRECORD_OCR_ENGINE") {
        let mut command = program_command(&engine);
        command.arg(path);
        let output = run_command(&mut command, deadline, "OCR_ENGINE", path)?;
        return decode_ocr_output(path, output, false);
    }

    if is_pdf(path)? {
        return ocr_pdf(path, deadline);
    }

    ocr_image(path, deadline)
}

fn is_pdf(path: &Path) -> Result<bool, AppError> {
    if path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("pdf"))
    {
        return Ok(true);
    }
    let mut file = std::fs::File::open(path).map_err(|error| {
        AppError::Internal(format!(
            "OCR_INPUT_OPEN_FAILED: failed to open '{}': {error}",
            path.display()
        ))
    })?;
    let mut signature = [0_u8; 5];
    let read = file.read(&mut signature).map_err(|error| {
        AppError::Internal(format!(
            "OCR_INPUT_READ_FAILED: failed to read '{}': {error}",
            path.display()
        ))
    })?;
    Ok(read == signature.len() && signature == *b"%PDF-")
}

fn validate_input(path: &Path) -> Result<(), AppError> {
    let metadata = std::fs::metadata(path).map_err(|error| {
        AppError::Internal(format!(
            "OCR_INPUT_INSPECTION_FAILED: failed to inspect '{}': {error}",
            path.display()
        ))
    })?;
    if !metadata.is_file() {
        return Err(AppError::DataParsing(format!(
            "OCR_INPUT_INVALID: '{}' is not a regular file",
            path.display()
        )));
    }
    if metadata.len() > MAX_OCR_INPUT_BYTES {
        return Err(AppError::DataParsing(format!(
            "OCR_INPUT_SIZE_EXCEEDED: '{}' exceeds {MAX_OCR_INPUT_BYTES} bytes",
            path.display()
        )));
    }
    Ok(())
}

fn ocr_timeout() -> Duration {
    let seconds = std::env::var("ONRECORD_OCR_TIMEOUT_SECONDS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|seconds| (1..=MAX_OCR_TIMEOUT_SECONDS).contains(seconds))
        .unwrap_or(DEFAULT_OCR_TIMEOUT_SECONDS);
    Duration::from_secs(seconds)
}

fn ocr_image(path: &Path, deadline: Instant) -> Result<String, AppError> {
    ocr_image_with_empty_policy(path, deadline, false)
}

fn ocr_image_with_empty_policy(
    path: &Path,
    deadline: Instant,
    allow_empty: bool,
) -> Result<String, AppError> {
    let mut command = program_command(OsStr::new("tesseract"));
    command.arg(path).arg("stdout");
    let output = run_command(&mut command, deadline, "OCR_ENGINE", path)?;
    decode_ocr_output(path, output, allow_empty)
}

fn ocr_pdf(path: &Path, deadline: Instant) -> Result<String, AppError> {
    let workspace = OcrWorkspace::new()?;
    let renderer =
        std::env::var_os("ONRECORD_PDF_RENDERER").unwrap_or_else(|| OsString::from("pdftoppm"));
    let page_count = pdf_page_count(path, deadline)?;
    if page_count == 0 {
        return Err(AppError::DataParsing(format!(
            "OCR_PAGE_COUNT_INVALID: '{}' contains no pages",
            path.display()
        )));
    }
    if page_count > MAX_PDF_OCR_PAGES {
        return Err(AppError::DataParsing(format!(
            "OCR_PAGE_COUNT_EXCEEDED: '{}' contains {page_count} pages; limit is {MAX_PDF_OCR_PAGES}",
            path.display(),
        )));
    }

    let mut raster_bytes = 0_u64;
    let mut text = String::new();
    for page_number in 1..=page_count {
        let prefix = workspace.path.join(format!("page-{page_number:08}"));
        let page = prefix.with_extension("png");
        let mut command = program_command(&renderer);
        command
            .arg("-f")
            .arg(page_number.to_string())
            .arg("-l")
            .arg(page_number.to_string())
            .arg("-singlefile")
            .arg("-png")
            .arg("-r")
            .arg("300")
            .arg(path)
            .arg(&prefix);
        let output = run_command(&mut command, deadline, "PDF_RENDERER", path)?;
        ensure_success(path, "PDF_RENDERER", &output)?;
        let page_size = std::fs::metadata(&page)
            .map_err(|error| {
                AppError::Internal(format!(
                    "OCR_RASTER_INSPECTION_FAILED: failed to inspect '{}': {error}",
                    page.display()
                ))
            })?
            .len();
        raster_bytes = raster_bytes.checked_add(page_size).ok_or_else(|| {
            AppError::DataParsing("OCR_RASTER_SIZE_OVERFLOW: raster size overflow".to_string())
        })?;
        if raster_bytes > MAX_RASTER_BYTES {
            return Err(AppError::DataParsing(format!(
                "OCR_RASTER_SIZE_EXCEEDED: rendered pages exceed {MAX_RASTER_BYTES} bytes"
            )));
        }

        let page_text = ocr_image_with_empty_policy(&page, deadline, true)?;
        std::fs::remove_file(&page).map_err(|error| {
            AppError::Internal(format!(
                "OCR_RASTER_CLEANUP_FAILED: failed to remove '{}': {error}",
                page.display()
            ))
        })?;
        if page_text.is_empty() {
            continue;
        }
        let separator_bytes = usize::from(!text.is_empty());
        if text
            .len()
            .checked_add(separator_bytes)
            .and_then(|size| size.checked_add(page_text.len()))
            .is_none_or(|size| size > MAX_OCR_OUTPUT_BYTES)
        {
            return Err(AppError::DataParsing(format!(
                "OCR_OUTPUT_SIZE_EXCEEDED: extracted text exceeds {MAX_OCR_OUTPUT_BYTES} bytes"
            )));
        }
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(&page_text);
    }

    if text.trim().is_empty() {
        return Err(AppError::DataParsing(format!(
            "OCR_OUTPUT_EMPTY: '{}' produced no searchable text",
            path.display()
        )));
    }
    Ok(text)
}

fn pdf_page_count(path: &Path, deadline: Instant) -> Result<usize, AppError> {
    if let Ok(document) = lopdf::Document::load(path) {
        return Ok(document.get_pages().len());
    }
    let inspector =
        std::env::var_os("ONRECORD_PDF_INFO").unwrap_or_else(|| OsString::from("pdfinfo"));
    let mut command = program_command(&inspector);
    command.arg(path);
    let output = run_command(&mut command, deadline, "PDF_INFO", path)?;
    ensure_success(path, "PDF_INFO", &output)?;
    let report = std::str::from_utf8(&output.stdout).map_err(|error| {
        AppError::DataParsing(format!(
            "PDF_INFO_OUTPUT_INVALID: '{}' produced invalid UTF-8 metadata: {error}",
            path.display()
        ))
    })?;
    report
        .lines()
        .find_map(|line| {
            line.split_once(':').and_then(|(name, value)| {
                name.trim()
                    .eq_ignore_ascii_case("pages")
                    .then(|| value.trim().parse::<usize>().ok())
                    .flatten()
            })
        })
        .ok_or_else(|| {
            AppError::DataParsing(format!(
                "PDF_INFO_PAGE_COUNT_MISSING: could not determine page count for '{}'",
                path.display()
            ))
        })
}

struct ProcessOutput {
    status: ExitStatus,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

fn run_command(
    command: &mut Command,
    deadline: Instant,
    label: &str,
    source: &Path,
) -> Result<ProcessOutput, AppError> {
    if Instant::now() >= deadline {
        return Err(AppError::DataParsing(format!(
            "{label}_TIMEOUT: processing deadline expired for '{}'",
            source.display()
        )));
    }
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| {
            AppError::Internal(format!(
                "{label}_LAUNCH_FAILED: failed to process '{}': {error}",
                source.display()
            ))
        })?;
    let stdout = child.stdout.take().ok_or_else(|| {
        AppError::Internal(format!("{label}_PIPE_FAILED: stdout pipe was unavailable"))
    })?;
    let stderr = child.stderr.take().ok_or_else(|| {
        AppError::Internal(format!("{label}_PIPE_FAILED: stderr pipe was unavailable"))
    })?;
    let stdout_reader = thread::spawn(move || read_bounded(stdout, MAX_OCR_OUTPUT_BYTES));
    let stderr_reader = thread::spawn(move || read_bounded(stderr, MAX_PROCESS_STDERR_BYTES));

    let status = loop {
        match child.try_wait().map_err(|error| {
            AppError::Internal(format!("{label}_WAIT_FAILED: process wait failed: {error}"))
        })? {
            Some(status) => break status,
            None if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = stdout_reader.join();
                let _ = stderr_reader.join();
                return Err(AppError::DataParsing(format!(
                    "{label}_TIMEOUT: processing deadline expired for '{}'",
                    source.display()
                )));
            }
            None => thread::sleep(Duration::from_millis(25)),
        }
    };

    let stdout = join_reader(stdout_reader, label, "stdout")?;
    let stderr = join_reader(stderr_reader, label, "stderr")?;
    Ok(ProcessOutput {
        status,
        stdout,
        stderr,
    })
}

fn read_bounded<R: Read>(reader: R, limit: usize) -> Result<Vec<u8>, io::Error> {
    let mut output = Vec::new();
    reader
        .take(u64::try_from(limit).unwrap_or(u64::MAX) + 1)
        .read_to_end(&mut output)?;
    Ok(output)
}

fn join_reader(
    reader: thread::JoinHandle<Result<Vec<u8>, io::Error>>,
    label: &str,
    stream: &str,
) -> Result<Vec<u8>, AppError> {
    let output = reader
        .join()
        .map_err(|_| AppError::Internal(format!("{label}_PIPE_FAILED: {stream} reader panicked")))?
        .map_err(|error| {
            AppError::Internal(format!(
                "{label}_PIPE_FAILED: failed to read {stream}: {error}"
            ))
        })?;
    let limit = if stream == "stdout" {
        MAX_OCR_OUTPUT_BYTES
    } else {
        MAX_PROCESS_STDERR_BYTES
    };
    if output.len() > limit {
        return Err(AppError::DataParsing(format!(
            "{label}_OUTPUT_SIZE_EXCEEDED: {stream} exceeded {limit} bytes"
        )));
    }
    Ok(output)
}

fn ensure_success(path: &Path, label: &str, output: &ProcessOutput) -> Result<(), AppError> {
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    Err(AppError::DataParsing(format!(
        "{label}_FAILED: '{}' exited with status {}: {}",
        path.display(),
        output.status,
        stderr
    )))
}

fn decode_ocr_output(
    path: &Path,
    output: ProcessOutput,
    allow_empty: bool,
) -> Result<String, AppError> {
    ensure_success(path, "OCR_ENGINE", &output)?;
    let text = String::from_utf8(output.stdout).map_err(|error| {
        AppError::DataParsing(format!(
            "OCR_OUTPUT_INVALID_UTF8: '{}' produced invalid UTF-8: {error}",
            path.display()
        ))
    })?;
    let text = text.trim().to_string();
    if text.is_empty() && !allow_empty {
        return Err(AppError::DataParsing(format!(
            "OCR_OUTPUT_EMPTY: '{}' produced no searchable text",
            path.display()
        )));
    }
    Ok(text)
}

struct OcrWorkspace {
    path: PathBuf,
}

impl OcrWorkspace {
    fn new() -> Result<Self, AppError> {
        let path = std::env::temp_dir().join(format!("onrecord-ocr-{}", Uuid::new_v4()));
        std::fs::create_dir(&path).map_err(|error| {
            AppError::Internal(format!(
                "OCR_WORKSPACE_CREATE_FAILED: failed to create '{}': {error}",
                path.display()
            ))
        })?;
        Ok(Self { path })
    }
}

impl Drop for OcrWorkspace {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::resolve_program;
    use std::ffi::{OsStr, OsString};
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;

    #[test]
    fn bare_name_falls_back_to_package_manager_directories_after_path() {
        let path_dir = tempfile::tempdir().expect("path dir");
        let fallback = tempfile::tempdir().expect("fallback dir");
        let installed = fallback.path().join("tesseract");
        std::fs::write(&installed, "#!/bin/sh\n").expect("write stub");
        std::fs::set_permissions(&installed, std::fs::Permissions::from_mode(0o755))
            .expect("mark executable");
        std::fs::write(path_dir.path().join("pdftoppm"), "").expect("non-executable file");

        let path = OsString::from(path_dir.path());
        let resolve =
            |name: &str| resolve_program(OsStr::new(name), Some(&path), [fallback.path()]);

        assert_eq!(resolve("tesseract"), installed.as_os_str());
        assert_eq!(resolve("pdftoppm"), OsStr::new("pdftoppm"));
        assert_eq!(
            resolve_program(
                installed.as_os_str(),
                Some(&path),
                [Path::new("/nonexistent")]
            ),
            installed.as_os_str()
        );
    }
}
