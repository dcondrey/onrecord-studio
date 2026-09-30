use std::path::{Path, PathBuf};

use chrono::{DateTime, SecondsFormat, Utc};
use msg_parser::{Outlook, Person};
use scraper::{node::Node, Html};
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;
use uuid::Uuid;

use crate::blocking::spawn as spawn_blocking;
use crate::error::AppError;
use crate::file_name::sanitized_component;
use crate::parsers::source::read_regular_file;
use crate::types::{DocumentMetadata, EmailMetadata, EmailParticipantMetadata};

const MAX_ATTACHMENT_COUNT: usize = 10_000;
const MAX_TOTAL_ATTACHMENT_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const MAX_MESSAGE_BYTES: u64 = 256 * 1024 * 1024;
const MAX_JAVASCRIPT_SAFE_INTEGER: u64 = (1_u64 << 53) - 1;
const SYNTHETIC_DOCUMENT_NAMESPACE: u64 = 1_u64 << 52;

#[derive(Debug, Clone)]
pub struct ExtractedEmailPayload {
    pub body_text: String,
    pub metadata: EmailMetadata,
    pub metadata_summary: String,
    pub attachment_documents: Vec<ExtractedEmailDocument>,
    /// Attachments the message carries that were not extracted because the caller asked for
    /// none. Zero when attachments were collected or the message has none.
    pub withheld_attachment_count: usize,
}

#[derive(Debug, Clone)]
pub struct ExtractedEmailDocument {
    pub metadata: DocumentMetadata,
}

pub struct MsgExtractionContext<'a> {
    pub source_document: &'a DocumentMetadata,
    pub cache_directory: &'a Path,
    pub collect_attachments: bool,
}

struct ParsedMessagePayload {
    body_text: String,
    metadata: EmailMetadata,
    metadata_summary: String,
    attachments: Vec<PendingAttachment>,
    withheld_attachment_count: usize,
}

struct PendingAttachment {
    id: u64,
    file_name: String,
    content_type: String,
    description: String,
    bytes: Vec<u8>,
}

fn strip_html_markup(html: &str) -> String {
    let document = Html::parse_fragment(html);
    let mut output = String::new();
    let mut current_block = None;
    for node in document.tree.root().descendants() {
        let Node::Text(text) = node.value() else {
            continue;
        };
        if node.ancestors().any(|ancestor| {
            matches!(
                ancestor.value(),
                Node::Element(element) if is_inactive_html_element(element.name())
            )
        }) {
            continue;
        }
        let normalized = collapse_html_whitespace(text);
        if normalized.is_empty() {
            continue;
        }
        let block = node
            .ancestors()
            .find(|ancestor| {
                matches!(
                    ancestor.value(),
                    Node::Element(element) if is_block_html_element(element.name())
                )
            })
            .map(|ancestor| ancestor.id());
        if !output.is_empty() {
            if block.is_some() && block != current_block {
                output.push('\n');
            } else if !output.ends_with(char::is_whitespace) {
                output.push(' ');
            }
        }
        output.push_str(&normalized);
        current_block = block.or(current_block);
    }
    output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn collapse_html_whitespace(value: &str) -> String {
    let mut output = String::new();
    let mut pending_space = false;
    for character in value.chars() {
        if character.is_whitespace() {
            pending_space = !output.is_empty();
        } else {
            if pending_space {
                output.push(' ');
                pending_space = false;
            }
            output.push(character);
        }
    }
    output
}

fn is_inactive_html_element(name: &str) -> bool {
    matches!(name, "script" | "style" | "noscript" | "template" | "svg")
}

fn is_block_html_element(name: &str) -> bool {
    matches!(
        name,
        "address"
            | "article"
            | "aside"
            | "blockquote"
            | "div"
            | "dl"
            | "fieldset"
            | "figcaption"
            | "figure"
            | "footer"
            | "form"
            | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "header"
            | "hr"
            | "li"
            | "main"
            | "nav"
            | "ol"
            | "p"
            | "pre"
            | "section"
            | "table"
            | "tr"
            | "ul"
    )
}

fn optional_string(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

fn normalized_timestamp(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    DateTime::parse_from_rfc3339(value)
        .or_else(|_| DateTime::parse_from_rfc2822(value))
        .map(|timestamp| {
            timestamp
                .with_timezone(&Utc)
                .to_rfc3339_opts(SecondsFormat::AutoSi, true)
        })
        .ok()
        .or_else(|| Some(value.to_string()))
}

fn participant_metadata(person: &Person) -> Option<EmailParticipantMetadata> {
    let participant = EmailParticipantMetadata {
        name: optional_string(&person.name),
        email: optional_string(&person.email),
    };
    if participant.name.is_none() && participant.email.is_none() {
        None
    } else {
        Some(participant)
    }
}

fn build_email_metadata(message: &Outlook) -> EmailMetadata {
    let sent_at = [
        message.message_delivery_time.as_str(),
        message.client_submit_time.as_str(),
        message.headers.date.as_str(),
    ]
    .into_iter()
    .find_map(normalized_timestamp);

    EmailMetadata {
        sender: participant_metadata(&message.sender),
        to: message.to.iter().filter_map(participant_metadata).collect(),
        cc: message.cc.iter().filter_map(participant_metadata).collect(),
        bcc: message
            .bcc
            .iter()
            .filter_map(participant_metadata)
            .collect(),
        subject: optional_string(&message.subject),
        sent_at,
        message_id: optional_string(&message.headers.message_id),
    }
}

fn format_participant(participant: &EmailParticipantMetadata) -> String {
    match (&participant.name, &participant.email) {
        (None, None) => String::new(),
        (None, Some(email)) => email.clone(),
        (Some(name), None) => name.clone(),
        (Some(name), Some(email)) if name == email => name.clone(),
        (Some(name), Some(email)) => format!("{name} <{email}>"),
    }
}

fn format_participants(participants: &[EmailParticipantMetadata]) -> String {
    participants
        .iter()
        .map(format_participant)
        .filter(|participant| !participant.is_empty())
        .collect::<Vec<_>>()
        .join("; ")
}

fn build_metadata_summary(metadata: &EmailMetadata) -> String {
    let mut fields = Vec::new();
    if let Some(sender) = &metadata.sender {
        fields.push(format!("From: {}", format_participant(sender)));
    }

    let to = format_participants(&metadata.to);
    if !to.is_empty() {
        fields.push(format!("To: {to}"));
    }

    let cc = format_participants(&metadata.cc);
    if !cc.is_empty() {
        fields.push(format!("Cc: {cc}"));
    }

    let bcc = format_participants(&metadata.bcc);
    if !bcc.is_empty() {
        fields.push(format!("Bcc: {bcc}"));
    }

    if let Some(subject) = &metadata.subject {
        fields.push(format!("Subject: {subject}"));
    }

    if let Some(sent_at) = &metadata.sent_at {
        fields.push(format!("Date: {sent_at}"));
    }

    if let Some(message_id) = &metadata.message_id {
        fields.push(format!("Message-ID: {message_id}"));
    }

    fields.join("\n")
}

fn sanitize_attachment_name(
    long_file_name: &str,
    file_name: &str,
    display_name: &str,
    extension: &str,
    sequence: usize,
) -> String {
    let candidate = [long_file_name, file_name, display_name]
        .into_iter()
        .find(|value| !value.trim().is_empty())
        .map(str::trim)
        .unwrap_or("");
    let mut sanitized = sanitized_component(candidate, 180);

    if sanitized.is_empty() {
        sanitized = format!("attachment_{sequence}");
    }

    let normalized_extension = extension
        .trim()
        .trim_start_matches('.')
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .take(16)
        .collect::<String>();
    if Path::new(&sanitized).extension().is_none() && !normalized_extension.is_empty() {
        sanitized.push('.');
        sanitized.push_str(&normalized_extension);
    }

    sanitized
}

fn attachment_id(source_document_id: u64, chain: &str, name: &str, bytes: &[u8]) -> u64 {
    let mut digest = Sha256::new();
    digest.update(source_document_id.to_le_bytes());
    for value in [chain.as_bytes(), name.as_bytes(), bytes] {
        digest.update((value.len() as u64).to_le_bytes());
        digest.update(value);
    }
    let hash = digest.finalize();
    let mut identifier_bytes = [0_u8; 8];
    identifier_bytes.copy_from_slice(&hash[..8]);
    let identifier = u64::from_le_bytes(identifier_bytes);
    let payload_mask = MAX_JAVASCRIPT_SAFE_INTEGER ^ SYNTHETIC_DOCUMENT_NAMESPACE;
    SYNTHETIC_DOCUMENT_NAMESPACE | (identifier & payload_mask)
}

fn collect_attachments(
    message: Outlook,
    source_document_id: u64,
    parent_chain: &str,
    attachments: &mut Vec<PendingAttachment>,
    total_bytes: &mut u64,
) -> Result<(), AppError> {
    for (index, attachment) in message.attachments.into_iter().enumerate() {
        if attachments.len() >= MAX_ATTACHMENT_COUNT {
            return Err(AppError::DataParsing(format!(
                "MSG_ATTACHMENT_COUNT_EXCEEDED: attachment count exceeds {MAX_ATTACHMENT_COUNT}"
            )));
        }

        let sequence = attachments.len() + 1;
        let file_name = sanitize_attachment_name(
            &attachment.long_file_name,
            &attachment.file_name,
            &attachment.display_name,
            &attachment.extension,
            sequence,
        );
        let chain = format!("{parent_chain}/{index}:{file_name}");
        let byte_count = u64::try_from(attachment.payload_bytes.len()).map_err(|error| {
            AppError::DataParsing(format!(
                "MSG_ATTACHMENT_SIZE_INVALID: attachment '{chain}' size cannot be represented: {error}"
            ))
        })?;
        *total_bytes = total_bytes.checked_add(byte_count).ok_or_else(|| {
            AppError::DataParsing(
                "MSG_ATTACHMENT_SIZE_OVERFLOW: aggregate size overflow".to_string(),
            )
        })?;
        if *total_bytes > MAX_TOTAL_ATTACHMENT_BYTES {
            return Err(AppError::DataParsing(format!(
                "MSG_ATTACHMENT_BYTES_EXCEEDED: aggregate attachment size exceeds {MAX_TOTAL_ATTACHMENT_BYTES} bytes"
            )));
        }

        let id = attachment_id(
            source_document_id,
            &chain,
            &file_name,
            &attachment.payload_bytes,
        );
        let is_embedded_message = attachment.is_embedded_message()
            || Path::new(&file_name)
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case("msg"))
            || attachment
                .mime_tag
                .eq_ignore_ascii_case("application/vnd.ms-outlook");
        // The sanitized long/display name is the identity later supplied to the queue's format
        // detector. Normalize the media type from that same identity so an attachment whose short
        // name omits `.msg` cannot be classified differently by the parser and the queue.
        let content_type = if is_embedded_message {
            "application/vnd.ms-outlook".to_string()
        } else if attachment.mime_tag.trim().is_empty() {
            "application/octet-stream".to_string()
        } else {
            attachment.mime_tag.trim().to_ascii_lowercase()
        };
        attachments.push(PendingAttachment {
            id,
            file_name,
            content_type,
            description: format!("Outlook attachment at {chain}"),
            bytes: attachment.payload_bytes,
        });
    }

    Ok(())
}

fn parse_message(
    bytes: &[u8],
    source_document_id: u64,
    collect_nested_attachments: bool,
) -> Result<ParsedMessagePayload, AppError> {
    let message = Outlook::from_slice(bytes)
        .map_err(|error| AppError::DataParsing(format!("MSG_PARSE_FAILED: {error}")))?;
    let body_text = if !message.body.trim().is_empty() {
        message.body.trim().to_string()
    } else if !message.html.trim().is_empty() {
        strip_html_markup(&message.html)
    } else {
        String::new()
    };
    let metadata = build_email_metadata(&message);
    let metadata_summary = build_metadata_summary(&metadata);
    let mut attachments = Vec::new();
    let mut total_bytes = 0u64;
    let mut withheld_attachment_count = 0;
    if collect_nested_attachments {
        collect_attachments(
            message,
            source_document_id,
            "message",
            &mut attachments,
            &mut total_bytes,
        )?;
    } else {
        withheld_attachment_count = message.attachments.len();
    }

    Ok(ParsedMessagePayload {
        body_text,
        metadata,
        metadata_summary,
        attachments,
        withheld_attachment_count,
    })
}

async fn persist_attachment(
    cache_directory: &Path,
    attachment: &PendingAttachment,
) -> Result<PathBuf, AppError> {
    tokio::fs::create_dir_all(cache_directory)
        .await
        .map_err(|error| {
            AppError::Internal(format!(
                "MSG_CACHE_CREATE_FAILED: failed to create '{}': {}",
                cache_directory.display(),
                error
            ))
        })?;

    let final_path = cache_directory.join(format!("{}_{}", attachment.id, attachment.file_name));
    if tokio::fs::try_exists(&final_path).await.map_err(|error| {
        AppError::Internal(format!(
            "MSG_CACHE_INSPECTION_FAILED: failed to inspect '{}': {}",
            final_path.display(),
            error
        ))
    })? {
        let existing = tokio::fs::read(&final_path).await.map_err(|error| {
            AppError::Internal(format!(
                "MSG_CACHE_READ_FAILED: failed to validate '{}': {}",
                final_path.display(),
                error
            ))
        })?;
        if existing == attachment.bytes {
            return Ok(final_path);
        }
        return Err(AppError::DataParsing(format!(
            "MSG_CACHE_COLLISION: '{}' already exists with different content",
            final_path.display()
        )));
    }

    let temporary_path = cache_directory.join(format!(".{}.{}.tmp", attachment.id, Uuid::new_v4()));
    let mut temporary_file = tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary_path)
        .await
        .map_err(|error| {
            AppError::Internal(format!(
                "MSG_CACHE_TEMP_CREATE_FAILED: failed to create '{}': {}",
                temporary_path.display(),
                error
            ))
        })?;

    if let Err(error) = temporary_file.write_all(&attachment.bytes).await {
        drop(temporary_file);
        let cleanup_error = tokio::fs::remove_file(&temporary_path).await.err();
        return Err(AppError::Internal(format!(
            "MSG_CACHE_WRITE_FAILED: failed to write '{}': {}{}",
            temporary_path.display(),
            error,
            cleanup_failure(cleanup_error.as_ref())
        )));
    }
    if let Err(error) = temporary_file.sync_all().await {
        drop(temporary_file);
        let cleanup_error = tokio::fs::remove_file(&temporary_path).await.err();
        return Err(AppError::Internal(format!(
            "MSG_CACHE_SYNC_FAILED: failed to sync '{}': {}{}",
            temporary_path.display(),
            error,
            cleanup_failure(cleanup_error.as_ref())
        )));
    }
    drop(temporary_file);

    match tokio::fs::hard_link(&temporary_path, &final_path).await {
        Ok(()) => {
            tokio::fs::remove_file(&temporary_path)
                .await
                .map_err(|error| {
                    AppError::Internal(format!(
                        "MSG_CACHE_TEMP_CLEANUP_FAILED: attachment was committed to '{}' but temporary link '{}' could not be removed: {}",
                        final_path.display(),
                        temporary_path.display(),
                        error
                    ))
                })?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            let cleanup_error = tokio::fs::remove_file(&temporary_path).await.err();
            let existing = tokio::fs::read(&final_path).await.map_err(|read_error| {
                AppError::Internal(format!(
                    "MSG_CACHE_RACE_VALIDATION_FAILED: failed to validate '{}': {}{}",
                    final_path.display(),
                    read_error,
                    cleanup_failure(cleanup_error.as_ref())
                ))
            })?;
            if existing != attachment.bytes {
                return Err(AppError::DataParsing(format!(
                    "MSG_CACHE_COLLISION: '{}' was concurrently created with different content{}",
                    final_path.display(),
                    cleanup_failure(cleanup_error.as_ref())
                )));
            }
            if let Some(cleanup_error) = cleanup_error {
                return Err(AppError::Internal(format!(
                    "MSG_CACHE_TEMP_CLEANUP_FAILED: attachment was committed to '{}' but temporary link '{}' could not be removed: {}",
                    final_path.display(),
                    temporary_path.display(),
                    cleanup_error
                )));
            }
        }
        Err(error) => {
            let cleanup_error = tokio::fs::remove_file(&temporary_path).await.err();
            return Err(AppError::Internal(format!(
                "MSG_CACHE_COMMIT_FAILED: failed to commit '{}' as '{}': {}{}",
                temporary_path.display(),
                final_path.display(),
                error,
                cleanup_failure(cleanup_error.as_ref())
            )));
        }
    }

    Ok(final_path)
}

fn cleanup_failure(error: Option<&std::io::Error>) -> String {
    error
        .map(|error| format!("; temporary-file cleanup failed: {error}"))
        .unwrap_or_default()
}

pub async fn extract_msg_layers<P: AsRef<Path>>(
    file_path: P,
    context: MsgExtractionContext<'_>,
) -> Result<ExtractedEmailPayload, AppError> {
    let path = file_path.as_ref();
    if context.cache_directory.as_os_str().is_empty() {
        return Err(AppError::DataParsing(
            "MSG_CACHE_PATH_INVALID: cache directory must not be empty".to_string(),
        ));
    }
    let source_document_id = context.source_document.id;
    let collect_attachments = context.collect_attachments;
    let source_path = path.to_path_buf();
    let parsed = spawn_blocking("MSG parser", move || {
        let bytes = read_regular_file(&source_path, "MSG", MAX_MESSAGE_BYTES)?;
        parse_message(&bytes, source_document_id, collect_attachments)
    })
    .await??;

    let cache_directory = context.cache_directory.to_path_buf();
    let mut attachment_documents = Vec::with_capacity(parsed.attachments.len());

    for attachment in &parsed.attachments {
        let persisted_path = persist_attachment(&cache_directory, attachment).await?;
        let file_size = u64::try_from(attachment.bytes.len()).map_err(|error| {
            AppError::DataParsing(format!(
                "MSG_ATTACHMENT_SIZE_INVALID: attachment '{}' size cannot be represented: {}",
                attachment.file_name, error
            ))
        })?;
        let document = DocumentMetadata {
            id: attachment.id,
            file_name: Some(attachment.file_name.clone()),
            file_size: Some(file_size),
            description: Some(attachment.description.clone()),
            content_type: Some(attachment.content_type.clone()),
            download_url: None,
            created_at: context.source_document.created_at.clone(),
            updated_at: context.source_document.updated_at.clone(),
            visibility: context.source_document.visibility.clone(),
            local_path: Some(persisted_path.to_string_lossy().into_owned()),
            extracted_text_sha256: None,
            was_ocr_processed: false,
        };
        attachment_documents.push(ExtractedEmailDocument { metadata: document });
    }

    Ok(ExtractedEmailPayload {
        body_text: parsed.body_text,
        metadata: parsed.metadata,
        metadata_summary: parsed.metadata_summary,
        attachment_documents,
        withheld_attachment_count: parsed.withheld_attachment_count,
    })
}

#[cfg(test)]
mod tests {
    use std::fs::File;
    use std::io::Write;
    use std::path::Path;

    use cfb::CompoundFile;
    use tempfile::tempdir;

    use super::{
        attachment_id, extract_msg_layers, parse_message, sanitize_attachment_name,
        strip_html_markup, MsgExtractionContext, MAX_JAVASCRIPT_SAFE_INTEGER,
    };
    use crate::error::AppError;
    use crate::types::DocumentMetadata;

    fn utf16le(value: &str) -> Vec<u8> {
        value
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect::<Vec<_>>()
    }

    fn write_msg_stream(
        compound: &mut CompoundFile<File>,
        path: &str,
        bytes: &[u8],
    ) -> Result<(), AppError> {
        let mut stream = compound.create_stream(path).map_err(|error| {
            AppError::Internal(format!("MSG_TEST_STREAM_CREATE_FAILED: {path}: {error}"))
        })?;
        stream.write_all(bytes).map_err(|error| {
            AppError::Internal(format!("MSG_TEST_STREAM_WRITE_FAILED: {path}: {error}"))
        })
    }

    struct MsgFixtureAttachment<'a> {
        bytes: &'a [u8],
        extension: &'a str,
        short_file_name: &'a str,
        long_file_name: &'a str,
        mime_tag: &'a str,
    }

    fn write_msg_fixture(path: &Path) -> Result<(), AppError> {
        write_msg_fixture_with_attachment(
            path,
            MsgFixtureAttachment {
                bytes: b"attachment evidence",
                extension: ".txt",
                short_file_name: "EVIDEN~1.TXT",
                long_file_name: "evidence.txt",
                mime_tag: "text/plain",
            },
        )
    }

    fn write_msg_fixture_with_attachment(
        path: &Path,
        attachment: MsgFixtureAttachment<'_>,
    ) -> Result<(), AppError> {
        let file = File::options()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(path)
            .map_err(|error| {
                AppError::Internal(format!(
                    "MSG_TEST_FIXTURE_CREATE_FAILED: failed to create '{}': {}",
                    path.display(),
                    error
                ))
            })?;
        let mut compound =
            CompoundFile::create_with_version(cfb::Version::V3, file).map_err(|error| {
                AppError::Internal(format!(
                    "MSG_TEST_FIXTURE_CREATE_FAILED: failed to create '{}': {}",
                    path.display(),
                    error
                ))
            })?;
        write_msg_stream(
            &mut compound,
            "/__substg1.0_0037001F",
            &utf16le("Forensic review"),
        )?;
        write_msg_stream(
            &mut compound,
            "/__substg1.0_1013001F",
            &utf16le(
                "<style>.hidden{display:none}</style><p>HTML <b>fallback</b></p><script>discard()</script>",
            ),
        )?;
        write_msg_stream(
            &mut compound,
            "/__substg1.0_0C1A001F",
            &utf16le("Records Officer"),
        )?;
        write_msg_stream(
            &mut compound,
            "/__substg1.0_5D01001F",
            &utf16le("records@example.gov"),
        )?;
        write_msg_stream(
            &mut compound,
            "/__substg1.0_007D001F",
            &utf16le(
                "Date: Mon, 24 Aug 2026 13:15:00 -0700\r\nMessage-ID: <case-42@example.gov>\r\n",
            ),
        )?;
        let attachment_storage = "/__attach_version1.0_#00000000";
        compound
            .create_storage(attachment_storage)
            .map_err(|error| {
                AppError::Internal(format!("MSG_TEST_ATTACHMENT_STORAGE_FAILED: {error}"))
            })?;
        write_msg_stream(
            &mut compound,
            &format!("{attachment_storage}/__substg1.0_37010102"),
            attachment.bytes,
        )?;
        write_msg_stream(
            &mut compound,
            &format!("{attachment_storage}/__substg1.0_3703001F"),
            &utf16le(attachment.extension),
        )?;
        write_msg_stream(
            &mut compound,
            &format!("{attachment_storage}/__substg1.0_3704001F"),
            &utf16le(attachment.short_file_name),
        )?;
        write_msg_stream(
            &mut compound,
            &format!("{attachment_storage}/__substg1.0_3707001F"),
            &utf16le(attachment.long_file_name),
        )?;
        write_msg_stream(
            &mut compound,
            &format!("{attachment_storage}/__substg1.0_370E001F"),
            &utf16le(attachment.mime_tag),
        )?;
        compound
            .flush()
            .map_err(|error| AppError::Internal(format!("MSG_TEST_FIXTURE_FLUSH_FAILED: {error}")))
    }

    #[test]
    fn html_fallback_removes_markup_and_inactive_blocks() -> Result<(), AppError> {
        let html = "<style>.x{display:none}</style><p>Hello&nbsp;<b>world</b></p><script>alert(1)</script><div>Next</div>";
        assert_eq!(strip_html_markup(html), "Hello world\nNext");
        Ok(())
    }

    #[test]
    fn attachment_names_cannot_escape_the_cache_directory() {
        assert_eq!(
            sanitize_attachment_name("../../evidence", "", "", ".pdf", 1),
            "evidence.pdf"
        );
    }

    #[test]
    fn attachment_identifiers_are_stable_and_json_safe() {
        let first = attachment_id(91, "message/0:file.pdf", "file.pdf", b"evidence");
        let second = attachment_id(91, "message/0:file.pdf", "file.pdf", b"evidence");
        assert_eq!(first, second);
        assert!(first > 0);
        assert!(first <= MAX_JAVASCRIPT_SAFE_INTEGER);
    }

    #[test]
    fn embedded_messages_are_exported_one_level_deep_using_their_detected_identity(
    ) -> Result<(), AppError> {
        let directory = tempdir().map_err(|error| {
            AppError::Internal(format!("MSG_TEST_TEMP_DIRECTORY_FAILED: {error}"))
        })?;
        let inner_path = directory.path().join("inner.msg");
        write_msg_fixture(&inner_path)?;
        let inner_bytes = std::fs::read(&inner_path).map_err(|error| {
            AppError::Internal(format!("MSG_TEST_FIXTURE_READ_FAILED: {error}"))
        })?;
        let outer_path = directory.path().join("outer.msg");
        // Reproduces the old identity split: the parser's short name has no extension and the
        // declared MIME is generic, while the exported long name is the `.msg` identity later
        // supplied to `detect_document_format`.
        write_msg_fixture_with_attachment(
            &outer_path,
            MsgFixtureAttachment {
                bytes: &inner_bytes,
                extension: ".msg",
                short_file_name: "NESTED~1",
                long_file_name: "nested.msg",
                mime_tag: "application/octet-stream",
            },
        )?;
        let outer_bytes = std::fs::read(&outer_path).map_err(|error| {
            AppError::Internal(format!("MSG_TEST_FIXTURE_READ_FAILED: {error}"))
        })?;

        let parsed = parse_message(&outer_bytes, 42, true)?;
        assert_eq!(
            parsed.attachments.len(),
            1,
            "the outer parse must export only the embedded message, not flatten its attachment"
        );
        let embedded = &parsed.attachments[0];
        assert_eq!(embedded.file_name, "nested.msg");
        assert_eq!(embedded.content_type, "application/vnd.ms-outlook");
        assert_eq!(embedded.bytes, inner_bytes);
        Ok(())
    }

    #[tokio::test]
    async fn extracts_real_msg_and_enqueues_cached_attachment() -> Result<(), AppError> {
        let directory = tempdir().map_err(|error| {
            AppError::Internal(format!("MSG_TEST_TEMP_DIRECTORY_FAILED: {error}"))
        })?;
        let message_path = directory.path().join("message.msg");
        let attachment_cache = directory.path().join("attachments");
        write_msg_fixture(&message_path)?;
        let message_bytes = std::fs::read(&message_path).map_err(|error| {
            AppError::Internal(format!("MSG_TEST_FIXTURE_READ_FAILED: {error}"))
        })?;
        assert!(parse_message(&message_bytes, 42, false)?
            .attachments
            .is_empty());
        let source_document = DocumentMetadata {
            id: 42,
            file_name: Some("message.msg".to_string()),
            file_size: None,
            description: None,
            content_type: Some("application/vnd.ms-outlook".to_string()),
            download_url: None,
            created_at: "2026-08-24T20:00:00Z".to_string(),
            updated_at: "2026-08-24T20:00:00Z".to_string(),
            visibility: "public".to_string(),
            local_path: Some(message_path.to_string_lossy().into_owned()),
            extracted_text_sha256: None,
            was_ocr_processed: false,
        };
        let extracted = extract_msg_layers(
            &message_path,
            MsgExtractionContext {
                source_document: &source_document,
                cache_directory: &attachment_cache,
                collect_attachments: true,
            },
        )
        .await?;

        assert_eq!(extracted.body_text, "HTML fallback");
        assert_eq!(
            extracted.metadata.subject.as_deref(),
            Some("Forensic review")
        );
        let sender = extracted
            .metadata
            .sender
            .ok_or_else(|| AppError::DataParsing("MSG_TEST_SENDER_METADATA_MISSING".to_string()))?;
        assert_eq!(sender.name.as_deref(), Some("Records Officer"));
        assert_eq!(sender.email.as_deref(), Some("records@example.gov"));
        assert_eq!(
            extracted.metadata.message_id.as_deref(),
            Some("<case-42@example.gov>")
        );
        assert_eq!(
            extracted.metadata.sent_at.as_deref(),
            Some("2026-08-24T20:15:00Z")
        );
        assert_eq!(extracted.attachment_documents.len(), 1);
        assert_eq!(extracted.withheld_attachment_count, 0);

        let withheld = extract_msg_layers(
            &message_path,
            MsgExtractionContext {
                source_document: &source_document,
                cache_directory: &attachment_cache,
                collect_attachments: false,
            },
        )
        .await?;
        assert!(withheld.attachment_documents.is_empty());
        assert_eq!(withheld.withheld_attachment_count, 1);

        let queued_document = extracted.attachment_documents[0].metadata.clone();
        assert_eq!(queued_document.file_name.as_deref(), Some("evidence.txt"));
        let cached_path = queued_document.local_path.ok_or_else(|| {
            AppError::DataParsing("MSG_TEST_ATTACHMENT_CACHE_PATH_MISSING".to_string())
        })?;
        let cached_bytes = tokio::fs::read(&cached_path).await.map_err(|error| {
            AppError::Internal(format!(
                "MSG_TEST_ATTACHMENT_CACHE_READ_FAILED: '{}': {}",
                cached_path, error
            ))
        })?;
        assert_eq!(cached_bytes, b"attachment evidence");
        Ok(())
    }
}
