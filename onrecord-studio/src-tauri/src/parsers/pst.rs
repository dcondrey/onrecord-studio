//! Bounded, atomic expansion of an Outlook PST mailbox: one text file per message, under its
//! folder path, plus each message's attachments.
//!
//! The evidence file is only ever opened for reading. `outlook_pst::open_store` and the crate's
//! `open` constructors also open a write handle, so they are not used here.

use std::collections::HashSet;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use chrono::{DateTime, SecondsFormat, Utc};
use outlook_pst::ltp::prop_context::PropertyValue;
use outlook_pst::ltp::table_context::TableContext;
use outlook_pst::messaging::attachment::{
    AnsiAttachment, Attachment, AttachmentData, UnicodeAttachment,
};
use outlook_pst::messaging::folder::Folder;
use outlook_pst::messaging::message::{AnsiMessage, Message, MessageProperties, UnicodeMessage};
use outlook_pst::messaging::store::{AnsiStore, EntryId, Store, UnicodeStore};
use outlook_pst::ndb::node_id::NodeId;
use outlook_pst::{AnsiPstFile, UnicodePstFile};
use uuid::Uuid;

use crate::error::AppError;
use crate::file_name::sanitized_component;
use crate::parsers::msg::{build_metadata_summary, strip_html_markup};
use crate::parsers::pst_isolation::{expand_in_child, ChildBudget};
use crate::parsers::rtf::extract_rtf_bytes;
use crate::parsers::source::inspect_regular_file;
use crate::parsers::staging::cleanup_failed_extraction;
use crate::parsers::text::windows_1252_char;
use crate::types::{EmailMetadata, EmailParticipantMetadata};

// PST budgets are their own, above the generic container caps (512 MB source, 2,000 entries),
// which a real production mailbox routinely exceeds. Each run happens in a child process (see
// `pst_isolation`) whose wall-clock time is bounded by `pst_isolation::child_timeout`.
/// Source size. The ingestion queue admits a PST up to this size for local import and parsing;
/// network downloads keep the generic 512 MB document limit.
pub const MAX_PST_BYTES: u64 = 8 * 1024 * 1024 * 1024;
/// Counts every directory and file written. A cached re-run is re-listed by
/// `existing_container_manifest` with this same limit, and it counts directories too, so counting
/// only messages here would let a first run pass that its own cache then refuses.
pub const MAX_ENTRY_COUNT: usize = 50_000;
const MAX_ENTRY_BYTES: u64 = 512 * 1024 * 1024;
/// Total bytes written for one mailbox.
pub const MAX_TOTAL_BYTES: u64 = 8 * 1024 * 1024 * 1024;
const MAX_FOLDER_COUNT: usize = 10_000;
const MAX_FOLDER_DEPTH: usize = 64;
/// Rows read from contents tables, including rows whose message cannot be opened.
const MAX_MESSAGE_ROWS: usize = 100_000;
/// Folders deeper than this share the deepest directory; the full path stays in the text. With
/// `MAX_NAME_BYTES` this keeps a relative path well inside macOS's 1024-byte PATH_MAX.
const MAX_DIRECTORY_DEPTH: usize = 8;
const MAX_NAME_BYTES: usize = 48;
const MAX_RECORDED_PROBLEMS: usize = 1_000;
/// `compressed-rtf` sizes its output buffer from the header's raw size and slices by it, so both
/// are checked before the crate sees the bytes. LZFu expands at most 8x, so the compressed cap
/// also bounds the output.
const MAX_COMPRESSED_RTF_BYTES: usize = 16 * 1024 * 1024;
const MAX_RTF_RAW_BYTES: u32 = 64 * 1024 * 1024;
const RTF_COMPRESSED: u32 = 0x7546_5A4C;
const RTF_UNCOMPRESSED: u32 = 0x414C_454D;

const MAILBOX_DIRECTORY: &str = "Mailbox";
const NOTES_FILE: &str = "extraction-notes.txt";

const PR_SUBJECT: u16 = 0x0037;
const PR_MESSAGE_CLASS: u16 = 0x001A;
const PR_CLIENT_SUBMIT_TIME: u16 = 0x0039;
const PR_SENT_REPRESENTING_NAME: u16 = 0x0042;
const PR_SENDER_NAME: u16 = 0x0C1A;
const PR_SENDER_ADDRTYPE: u16 = 0x0C1E;
const PR_SENDER_EMAIL_ADDRESS: u16 = 0x0C1F;
const PR_SENDER_SMTP_ADDRESS: u16 = 0x5D01;
const PR_DISPLAY_BCC: u16 = 0x0E02;
const PR_DISPLAY_CC: u16 = 0x0E03;
const PR_DISPLAY_TO: u16 = 0x0E04;
const PR_MESSAGE_DELIVERY_TIME: u16 = 0x0E06;
const PR_BODY: u16 = 0x1000;
const PR_RTF_COMPRESSED: u16 = 0x1009;
const PR_HTML: u16 = 0x1013;
const PR_INTERNET_MESSAGE_ID: u16 = 0x1035;
const PR_INTERNET_CPID: u16 = 0x3FDE;
const PR_MESSAGE_CODEPAGE: u16 = 0x3FFD;
const PR_RECIPIENT_TYPE: u16 = 0x0C15;
const PR_DISPLAY_NAME: u16 = 0x3001;
const PR_ADDRTYPE: u16 = 0x3002;
const PR_EMAIL_ADDRESS: u16 = 0x3003;
const PR_SMTP_ADDRESS: u16 = 0x39FE;
const PR_ATTACH_EXTENSION: u16 = 0x3703;
const PR_ATTACH_FILENAME: u16 = 0x3704;
const PR_ATTACH_LONG_FILENAME: u16 = 0x3707;
const PR_PST_PASSWORD: u16 = 0x67FF;

/// The two on-disk layouts the reader handles, chosen from the header's `wVer`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PstLayout {
    Unicode,
    Ansi,
}

/// Reads the 12-byte header prefix and says whether the reader can open the file. `Err` from the
/// inner result is a fixed reason for an Outlook store this reader does not handle, such as an
/// OST offline cache; the outer error is an unreadable or truncated file.
pub fn inspect_pst_header(path: &Path) -> Result<Result<PstLayout, &'static str>, AppError> {
    let mut header = [0_u8; 12];
    File::open(path)
        .and_then(|mut file| file.read_exact(&mut header))
        .map_err(|error| {
            AppError::DataParsing(format!(
                "PST_HEADER_UNREADABLE: failed to read the header of '{}': {error}",
                path.display()
            ))
        })?;
    if &header[..4] != b"!BDN" {
        return Err(AppError::DataParsing(format!(
            "PST_HEADER_INVALID: '{}' does not start with the Outlook store signature",
            path.display()
        )));
    }
    Ok(match (&header[8..10], u16::from_le_bytes([header[10], header[11]])) {
        (b"SM", 14 | 15) => Ok(PstLayout::Ansi),
        (b"SM", 23) => Ok(PstLayout::Unicode),
        (b"SM", _) => Err("PST reader: this PST format version is not supported"),
        (b"SO", _) => Err(
            "PST reader: OST offline caches are not supported; export the mailbox to a PST from Outlook",
        ),
        _ => Err("PST reader: unrecognized Outlook store type"),
    })
}

/// Expands `pst_path` into the new directory `destination` atomically. The reader runs in a child
/// process; a crash, abort or timeout there is an `Err` for this file, never an empty mailbox.
pub fn extract_pst_container_with_manifest<A: AsRef<Path>, D: AsRef<Path>>(
    pst_path: A,
    destination: D,
) -> Result<Vec<PathBuf>, AppError> {
    extract_in_child(
        pst_path.as_ref(),
        destination.as_ref(),
        ChildBudget::for_source,
        |_| {},
    )
}

fn extract_in_child(
    pst_path: &Path,
    destination: &Path,
    budget_for_source: impl FnOnce(u64) -> ChildBudget,
    configure: impl FnOnce(&mut std::process::Command),
) -> Result<Vec<PathBuf>, AppError> {
    let source = inspect_regular_file(pst_path, "PST", MAX_PST_BYTES)?;
    inspect_pst_header(pst_path)?
        .map_err(|reason| AppError::DataParsing(format!("PST_FORMAT_UNSUPPORTED: {reason}")))?;
    if destination.as_os_str().is_empty() || destination.file_name().is_none() {
        return Err(AppError::DataParsing(
            "PST_DESTINATION_INVALID: destination must identify a new directory".to_string(),
        ));
    }
    if destination.try_exists().map_err(|error| {
        AppError::Internal(format!(
            "PST_DESTINATION_INSPECTION_FAILED: failed to inspect '{}': {error}",
            destination.display()
        ))
    })? {
        return Err(AppError::DataParsing(format!(
            "PST_DESTINATION_EXISTS: '{}' already exists",
            destination.display()
        )));
    }

    let parent = destination.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(|error| {
        AppError::Internal(format!(
            "PST_DESTINATION_CREATE_FAILED: failed to create '{}': {error}",
            parent.display()
        ))
    })?;
    let destination_name = destination
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            AppError::DataParsing(format!(
                "PST_DESTINATION_INVALID: '{}' has no valid final component",
                destination.display()
            ))
        })?;
    let staging = parent.join(format!(".{destination_name}.{}.extracting", Uuid::new_v4()));
    fs::create_dir(&staging).map_err(|error| {
        AppError::Internal(format!(
            "PST_STAGING_CREATE_FAILED: failed to create '{}': {error}",
            staging.display()
        ))
    })?;

    let budget = budget_for_source(source.len());
    let relative_files = match expand_in_child(pst_path, &staging, &budget, configure) {
        Ok(files) => files,
        Err(error) => return Err(cleanup_failed_extraction(&staging, "PST", error)),
    };
    if let Err(error) = fs::rename(&staging, destination) {
        let commit_error = AppError::Internal(format!(
            "PST_COMMIT_FAILED: failed to commit '{}' as '{}': {error}",
            staging.display(),
            destination.display()
        ));
        return Err(cleanup_failed_extraction(&staging, "PST", commit_error));
    }
    Ok(relative_files
        .into_iter()
        .map(|relative_path| destination.join(relative_path))
        .collect())
}

/// The child process's whole job: read the mailbox into the empty staging directory the parent
/// created. Commit and cleanup stay with the parent.
pub(crate) fn expand_into_staging(pst_path: &Path, staging: &Path) -> Result<(), AppError> {
    inspect_regular_file(pst_path, "PST", MAX_PST_BYTES)?;
    let layout = inspect_pst_header(pst_path)?
        .map_err(|reason| AppError::DataParsing(format!("PST_FORMAT_UNSUPPORTED: {reason}")))?;
    let empty_directory = fs::symlink_metadata(staging).is_ok_and(|metadata| metadata.is_dir())
        && fs::read_dir(staging).is_ok_and(|mut entries| entries.next().is_none());
    if !empty_directory {
        return Err(AppError::Internal(format!(
            "PST_STAGING_INVALID: '{}' is not an empty directory",
            staging.display()
        )));
    }
    extract_into(pst_path, layout, staging).map(|_| ())
}

enum PstStore {
    Unicode(Rc<UnicodeStore>),
    Ansi(Rc<AnsiStore>),
}

enum PstMessage {
    Unicode(Rc<UnicodeMessage>),
    Ansi(Rc<AnsiMessage>),
}

impl PstStore {
    fn open(path: &Path, layout: PstLayout) -> io::Result<Self> {
        let reader = Box::new(BufReader::new(File::open(path)?));
        Ok(match layout {
            PstLayout::Unicode => Self::Unicode(UnicodeStore::read(Rc::new(
                UnicodePstFile::read_from(reader)?,
            ))?),
            PstLayout::Ansi => {
                Self::Ansi(AnsiStore::read(Rc::new(AnsiPstFile::read_from(reader)?))?)
            }
        })
    }

    fn store(&self) -> &dyn Store {
        match self {
            Self::Unicode(store) => store.as_ref(),
            Self::Ansi(store) => store.as_ref(),
        }
    }

    fn entry_id(&self, node: u32) -> io::Result<EntryId> {
        self.store().properties().make_entry_id(NodeId::from(node))
    }

    fn open_message(&self, entry_id: &EntryId) -> io::Result<PstMessage> {
        Ok(match self {
            Self::Unicode(store) => {
                PstMessage::Unicode(UnicodeMessage::read(store.clone(), entry_id, None)?)
            }
            Self::Ansi(store) => {
                PstMessage::Ansi(AnsiMessage::read(store.clone(), entry_id, None)?)
            }
        })
    }
}

impl PstMessage {
    fn message(&self) -> &dyn Message {
        match self {
            Self::Unicode(message) => message.as_ref(),
            Self::Ansi(message) => message.as_ref(),
        }
    }

    fn open_attachment(&self, node: u32) -> io::Result<Rc<dyn Attachment>> {
        let node = NodeId::from(node);
        Ok(match self {
            Self::Unicode(message) => UnicodeAttachment::read(message.clone(), node, None)?,
            Self::Ansi(message) => AnsiAttachment::read(message.clone(), node, None)?,
        })
    }
}

/// Everything written into staging, with the budgets every write is charged against.
struct Output<'a> {
    staging: &'a Path,
    files: Vec<PathBuf>,
    entries: usize,
    max_entries: usize,
    total_bytes: u64,
}

impl<'a> Output<'a> {
    fn new(staging: &'a Path, max_entries: usize) -> Self {
        Self {
            staging,
            files: Vec::new(),
            entries: 0,
            max_entries,
            total_bytes: 0,
        }
    }

    fn charge_entry(&mut self) -> Result<(), AppError> {
        self.entries += 1;
        if self.entries > self.max_entries {
            return Err(AppError::DataParsing(format!(
                "PST_ENTRY_COUNT_EXCEEDED: mailbox expands beyond {} files and folders",
                self.max_entries
            )));
        }
        Ok(())
    }

    fn ensure_directory(&mut self, relative: &Path) -> Result<(), AppError> {
        let mut current = self.staging.to_path_buf();
        for component in relative.components() {
            current.push(component);
            match fs::create_dir(&current) {
                Ok(()) => self.charge_entry()?,
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists && current.is_dir() => {}
                Err(error) => {
                    return Err(AppError::Internal(format!(
                        "PST_DIRECTORY_CREATE_FAILED: failed to create '{}': {error}",
                        current.display()
                    )))
                }
            }
        }
        Ok(())
    }

    fn write_file(&mut self, relative: PathBuf, bytes: &[u8]) -> Result<(), AppError> {
        let size = bytes.len() as u64;
        if size > MAX_ENTRY_BYTES {
            return Err(AppError::DataParsing(format!(
                "PST_ENTRY_SIZE_EXCEEDED: '{}' exceeds {MAX_ENTRY_BYTES} bytes",
                relative.display()
            )));
        }
        self.total_bytes = self.total_bytes.saturating_add(size);
        if self.total_bytes > MAX_TOTAL_BYTES {
            return Err(AppError::DataParsing(format!(
                "PST_TOTAL_SIZE_EXCEEDED: expanded data exceeds {MAX_TOTAL_BYTES} bytes"
            )));
        }
        if let Some(parent) = relative.parent() {
            self.ensure_directory(parent)?;
        }
        self.charge_entry()?;
        let path = self.staging.join(&relative);
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|error| {
                AppError::Internal(format!(
                    "PST_ENTRY_CREATE_FAILED: failed to create '{}': {error}",
                    path.display()
                ))
            })?;
        output
            .write_all(bytes)
            .and_then(|()| output.sync_all())
            .map_err(|error| {
                AppError::Internal(format!(
                    "PST_ENTRY_WRITE_FAILED: failed to write '{}': {error}",
                    path.display()
                ))
            })?;
        self.files.push(relative);
        Ok(())
    }
}

/// Problems that cost one entry rather than the whole mailbox. Each becomes a line in the notes
/// file so an unreadable message is recorded, never silently dropped.
#[derive(Default)]
struct Problems {
    lines: Vec<String>,
    count: usize,
}

impl Problems {
    fn record(&mut self, line: String) {
        self.count += 1;
        if self.lines.len() < MAX_RECORDED_PROBLEMS {
            self.lines.push(line);
        }
    }
}

struct PendingFolder {
    folder: Rc<dyn Folder>,
    /// Display names from the top of the mailbox, unsanitized, for the `Folder:` line.
    names: Vec<String>,
    /// Sanitized directory, capped at `MAX_DIRECTORY_DEPTH` components.
    directory: PathBuf,
}

fn extract_into(
    pst_path: &Path,
    layout: PstLayout,
    staging: &Path,
) -> Result<Vec<PathBuf>, AppError> {
    let store = PstStore::open(pst_path, layout).map_err(|error| {
        AppError::DataParsing(format!(
            "PST_OPEN_FAILED: failed to read the mailbox in '{}': {error}",
            pst_path.display()
        ))
    })?;
    let root = store
        .store()
        .properties()
        .ipm_sub_tree_entry_id()
        .and_then(|entry_id| store.store().open_folder(&entry_id))
        .map_err(|error| {
            AppError::DataParsing(format!(
                "PST_ROOT_FOLDER_UNREADABLE: failed to open the top of the mailbox in '{}': {error}",
                pst_path.display()
            ))
        })?;

    let mut output = Output::new(staging, MAX_ENTRY_COUNT);
    let mut problems = Problems::default();
    let mut visited = HashSet::from([u32::from(root.properties().node_id())]);
    let mut folder_count = 0_usize;
    let mut message_rows = 0_usize;
    let mut message_count = 0_usize;
    let mut stack = Vec::new();
    push_sub_folders(
        &store,
        &PendingFolder {
            folder: root,
            names: Vec::new(),
            directory: PathBuf::from(MAILBOX_DIRECTORY),
        },
        &mut stack,
        &mut visited,
        &mut problems,
    );

    while let Some(pending) = stack.pop() {
        folder_count += 1;
        if folder_count > MAX_FOLDER_COUNT {
            return Err(AppError::DataParsing(format!(
                "PST_FOLDER_COUNT_EXCEEDED: mailbox holds more than {MAX_FOLDER_COUNT} folders"
            )));
        }
        let folder_path = pending.names.join("/");
        if let Some(contents) = pending.folder.contents_table() {
            for row in contents.rows_matrix() {
                message_rows += 1;
                if message_rows > MAX_MESSAGE_ROWS {
                    return Err(AppError::DataParsing(format!(
                        "PST_MESSAGE_COUNT_EXCEEDED: mailbox lists more than {MAX_MESSAGE_ROWS} messages"
                    )));
                }
                let node = u32::from(row.id());
                let message = store
                    .entry_id(node)
                    .and_then(|entry_id| store.open_message(&entry_id));
                match message {
                    Ok(message) => {
                        write_message(&message, node, &pending, &folder_path, &mut output)?;
                        message_count += 1;
                    }
                    Err(error) => problems.record(format!(
                        "{}: message {node:08x} could not be read: {error}",
                        display_folder(&folder_path)
                    )),
                }
            }
        }
        if pending.names.len() >= MAX_FOLDER_DEPTH {
            if pending
                .folder
                .hierarchy_table()
                .is_some_and(|table| table.rows_matrix().next().is_some())
            {
                problems.record(format!(
                    "{}: sub-folders not read; folders nest deeper than {MAX_FOLDER_DEPTH} levels",
                    display_folder(&folder_path)
                ));
            }
            continue;
        }
        push_sub_folders(&store, &pending, &mut stack, &mut visited, &mut problems);
    }

    let password_protected = matches!(
        store.store().properties().get(PR_PST_PASSWORD),
        Some(PropertyValue::Integer32(value)) if *value != 0
    );
    if problems.count > 0 || password_protected {
        let mut notes = format!(
            "Outlook PST extraction notes\nMessages extracted: {message_count}\nProblems recorded: {}\n",
            problems.count
        );
        if password_protected {
            notes.push_str(
                "The PST carries an Outlook password. That password is an access check in Outlook, \
                 not encryption, so the contents were read without it.\n",
            );
        }
        for line in &problems.lines {
            notes.push_str("- ");
            notes.push_str(line);
            notes.push('\n');
        }
        if problems.count > problems.lines.len() {
            notes.push_str(&format!(
                "- {} further problem(s) not listed.\n",
                problems.count - problems.lines.len()
            ));
        }
        output.write_file(PathBuf::from(NOTES_FILE), clean_text(&notes).as_bytes())?;
    }
    Ok(output.files)
}

fn push_sub_folders(
    store: &PstStore,
    parent: &PendingFolder,
    stack: &mut Vec<PendingFolder>,
    visited: &mut HashSet<u32>,
    problems: &mut Problems,
) {
    let Some(hierarchy) = parent.folder.hierarchy_table() else {
        return;
    };
    let parent_path = parent.names.join("/");
    for row in hierarchy.rows_matrix() {
        let node = u32::from(row.id());
        if !visited.insert(node) {
            problems.record(format!(
                "{}: folder {node:08x} is listed more than once; read only the first time",
                display_folder(&parent_path)
            ));
            continue;
        }
        let folder = match store
            .entry_id(node)
            .and_then(|entry_id| store.store().open_folder(&entry_id))
        {
            Ok(folder) => folder,
            Err(error) => {
                problems.record(format!(
                    "{}: folder {node:08x} could not be read: {error}",
                    display_folder(&parent_path)
                ));
                continue;
            }
        };
        let name = folder
            .properties()
            .get(PR_DISPLAY_NAME)
            .and_then(|value| property_text(value, None).map(|(text, _)| text))
            .map(|name| clean_line(&name))
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| format!("folder {node:08x}"));
        let mut directory = parent.directory.clone();
        if parent.names.len() < MAX_DIRECTORY_DEPTH {
            let component = sanitized_component(&name, MAX_NAME_BYTES);
            directory.push(if component.is_empty() {
                format!("folder {node:08x}")
            } else {
                component
            });
        }
        let mut names = parent.names.clone();
        names.push(name);
        stack.push(PendingFolder {
            folder,
            names,
            directory,
        });
    }
}

fn display_folder(path: &str) -> &str {
    if path.is_empty() {
        "(top of mailbox)"
    } else {
        path
    }
}

fn write_message(
    message: &PstMessage,
    node: u32,
    folder: &PendingFolder,
    folder_path: &str,
    output: &mut Output<'_>,
) -> Result<(), AppError> {
    let properties = message.message().properties();
    let subject = message_subject(properties);
    let stem = match sanitized_component(subject.as_deref().unwrap_or(""), MAX_NAME_BYTES) {
        name if name.is_empty() => format!("{node:08x}"),
        name => format!("{node:08x} {name}"),
    };
    let attachment_directory = folder.directory.join(format!("{stem}.attachments"));
    let mut attachment_lines = Vec::new();
    if let Some(table) = message.message().attachment_table() {
        for (index, row) in table.rows_matrix().enumerate() {
            let sequence = index + 1;
            let line = match message.open_attachment(u32::from(row.id())) {
                Ok(attachment) => write_attachment(
                    |id| attachment.properties().get(id),
                    attachment.data(),
                    sequence,
                    &attachment_directory,
                    folder_path,
                    output,
                )?,
                Err(error) => {
                    format!("{sequence}. not extracted: the attachment could not be read: {error}")
                }
            };
            attachment_lines.push(line);
        }
    }
    let text = render_message(
        properties,
        message.message().recipient_table(),
        folder_path,
        &attachment_lines,
    );
    output.write_file(
        folder.directory.join(format!("{stem}.txt")),
        text.as_bytes(),
    )
}

/// Writes one attachment and returns the line the parent message lists it under.
fn write_attachment<'a>(
    property: impl Fn(u16) -> Option<&'a PropertyValue>,
    data: Option<&AttachmentData>,
    sequence: usize,
    directory: &Path,
    folder_path: &str,
    output: &mut Output<'_>,
) -> Result<String, AppError> {
    let text_of = |id: u16| {
        property(id)
            .and_then(|value| property_text(value, None).map(|(text, _)| clean_line(&text)))
            .filter(|text| !text.is_empty())
    };
    let display = text_of(PR_ATTACH_LONG_FILENAME)
        .or_else(|| text_of(PR_ATTACH_FILENAME))
        .or_else(|| text_of(PR_DISPLAY_NAME))
        .unwrap_or_else(|| format!("attachment {sequence}"));
    match data {
        Some(AttachmentData::Binary(value)) => {
            let name = attachment_file_name(&display, text_of(PR_ATTACH_EXTENSION), sequence);
            output.write_file(directory.join(&name), value.buffer())?;
            Ok(format!("{sequence}. {display} (extracted as {name})"))
        }
        Some(AttachmentData::Message(embedded)) => {
            let name = attachment_file_name(&display, None, sequence);
            let name = format!("{name}.txt");
            let mut lines = Vec::new();
            if let Some(count) = embedded
                .attachment_table()
                .map(|table| table.rows_matrix().count())
                .filter(|count| *count > 0)
            {
                lines.push(format!(
                    "{count} attachment(s) of this embedded message not extracted: the PST reader \
                     cannot open attachments nested inside an attached message."
                ));
            }
            let text = render_message(
                embedded.properties(),
                embedded.recipient_table(),
                folder_path,
                &lines,
            );
            output.write_file(directory.join(&name), text.as_bytes())?;
            Ok(format!(
                "{sequence}. {display} (attached message, extracted as {name})"
            ))
        }
        None => Ok(format!(
            "{sequence}. {display} not extracted: the PST holds a reference to it, not its contents"
        )),
    }
}

/// The extension is split off before the stem is truncated: the child's format detection reads
/// it, so a long name must not lose it or end at a stray dot.
fn attachment_file_name(display: &str, extension: Option<String>, sequence: usize) -> String {
    let is_extension = |value: &str| {
        (1..=16).contains(&value.len()) && value.chars().all(|c| c.is_ascii_alphanumeric())
    };
    let (stem, extension) = match display.rsplit_once('.') {
        Some((stem, own)) if !stem.trim().is_empty() && is_extension(own) => {
            (stem, Some(own.to_string()))
        }
        _ => (
            display,
            extension
                .map(|value| value.trim().trim_start_matches('.').to_string())
                .filter(|value| is_extension(value)),
        ),
    };
    let mut name = sanitized_component(stem, MAX_NAME_BYTES);
    if name.is_empty() {
        name = format!("attachment {sequence}");
    }
    if let Some(extension) = extension {
        name.push('.');
        name.push_str(&extension);
    }
    format!("{sequence} {name}")
}

fn message_subject(properties: &MessageProperties) -> Option<String> {
    let (subject, _) = property_text(properties.get(PR_SUBJECT)?, message_codepage(properties))?;
    // A subject may open with the MAPI normalized-subject marker: 0x01 then a prefix length.
    let subject = match subject.strip_prefix('\u{1}') {
        Some(rest) => rest.chars().skip(1).collect(),
        None => subject,
    };
    Some(clean_line(&subject)).filter(|subject| !subject.is_empty())
}

fn message_codepage(properties: &MessageProperties) -> Option<i32> {
    match properties.get(PR_MESSAGE_CODEPAGE) {
        Some(PropertyValue::Integer32(codepage)) => Some(*codepage),
        _ => None,
    }
}

/// Builds the child document's text. It always opens with the fixed `Folder:` label: the format
/// detector sniffs short magic prefixes before it looks at the `.txt` name, so a subject on the
/// first line could otherwise pick the child's parser.
fn render_message(
    properties: &MessageProperties,
    recipients: Option<&Rc<dyn TableContext>>,
    folder_path: &str,
    attachment_lines: &[String],
) -> String {
    let codepage = message_codepage(properties);
    let mut notes = Vec::new();
    let text_of = |id: u16, notes: &mut Vec<String>| {
        let (text, note) = property_text(properties.get(id)?, codepage)?;
        if let Some(note) = note {
            notes.push(note);
        }
        Some(clean_line(&text)).filter(|text| !text.is_empty())
    };

    let sender_address = text_of(PR_SENDER_SMTP_ADDRESS, &mut notes).or_else(|| {
        (text_of(PR_SENDER_ADDRTYPE, &mut notes).as_deref() == Some("SMTP"))
            .then(|| text_of(PR_SENDER_EMAIL_ADDRESS, &mut notes))
            .flatten()
    });
    let sender_name = text_of(PR_SENDER_NAME, &mut notes)
        .or_else(|| text_of(PR_SENT_REPRESENTING_NAME, &mut notes));
    let sender =
        (sender_name.is_some() || sender_address.is_some()).then_some(EmailParticipantMetadata {
            name: sender_name,
            email: sender_address,
        });
    let (mut to, mut cc, mut bcc) = recipients
        .map(|table| read_recipients(table.as_ref(), codepage))
        .unwrap_or_default();
    if recipients.is_none() {
        to = display_recipients(text_of(PR_DISPLAY_TO, &mut notes));
        cc = display_recipients(text_of(PR_DISPLAY_CC, &mut notes));
        bcc = display_recipients(text_of(PR_DISPLAY_BCC, &mut notes));
    }
    let sent_at = [PR_MESSAGE_DELIVERY_TIME, PR_CLIENT_SUBMIT_TIME]
        .into_iter()
        .find_map(|id| match properties.get(id) {
            Some(PropertyValue::Time(value)) => filetime_rfc3339(*value),
            _ => None,
        });
    let metadata = EmailMetadata {
        sender,
        to,
        cc,
        bcc,
        subject: message_subject(properties),
        sent_at,
        message_id: text_of(PR_INTERNET_MESSAGE_ID, &mut notes),
    };

    let mut text = format!("Folder: {}\n", display_folder(folder_path));
    let summary = build_metadata_summary(&metadata);
    if !summary.is_empty() {
        text.push_str(&summary);
        text.push('\n');
    }
    if let Some(class) = text_of(PR_MESSAGE_CLASS, &mut notes) {
        text.push_str(&format!("Message class: {class}\n"));
    }
    text.push('\n');
    match select_body(|id| properties.get(id), codepage) {
        Ok((body, note)) => {
            notes.extend(note);
            text.push_str(body.trim());
        }
        Err(reason) => text.push_str(&format!("[Body not extracted: {reason}]")),
    }
    text.push('\n');
    if !attachment_lines.is_empty() {
        text.push_str("\nAttachments:\n");
        for line in attachment_lines {
            text.push_str(line);
            text.push('\n');
        }
    }
    let mut seen = HashSet::new();
    notes.retain(|note| seen.insert(note.clone()));
    if !notes.is_empty() {
        text.push_str("\nExtraction notes:\n");
        for note in notes {
            text.push_str(&note);
            text.push('\n');
        }
    }
    clean_text(&text)
}

/// Plain text first, then HTML reduced to its text, then compressed RTF. A body that is present
/// but cannot be decoded is reported as not extracted; nothing is substituted for it.
fn select_body<'a>(
    property: impl Fn(u16) -> Option<&'a PropertyValue>,
    codepage: Option<i32>,
) -> Result<(String, Option<String>), String> {
    if let Some((body, note)) = property(PR_BODY).and_then(|value| property_text(value, codepage)) {
        if !body.trim().is_empty() {
            return Ok((body, note));
        }
    }
    if let Some(value) = property(PR_HTML) {
        let decoded = match value {
            PropertyValue::Binary(binary) => {
                let html_codepage = match property(PR_INTERNET_CPID) {
                    Some(PropertyValue::Integer32(codepage)) => Some(*codepage),
                    _ => codepage,
                };
                Some(decode_codepage(binary.buffer(), html_codepage))
            }
            other => property_text(other, codepage),
        };
        if let Some((html, note)) = decoded {
            let body = strip_html_markup(&html);
            if !body.trim().is_empty() {
                return Ok((body, note));
            }
        }
    }
    match property(PR_RTF_COMPRESSED) {
        Some(PropertyValue::Binary(binary)) => {
            let rtf = decompress_rtf_body(binary.buffer())?;
            extract_rtf_bytes(&rtf, Path::new("PST message body"))
                .map(|body| (body, None))
                .map_err(|error| format!("the RTF body could not be read: {error}"))
        }
        Some(_) => Err("the RTF body is not stored as binary data".to_string()),
        None => Err("the message has no plain-text, HTML or RTF body".to_string()),
    }
}

fn decompress_rtf_body(data: &[u8]) -> Result<Vec<u8>, String> {
    if data.len() < 16 {
        return Err("the compressed RTF header is truncated".to_string());
    }
    if data.len() > MAX_COMPRESSED_RTF_BYTES {
        return Err(format!(
            "the compressed RTF body exceeds {MAX_COMPRESSED_RTF_BYTES} bytes"
        ));
    }
    let field = |offset: usize| {
        u32::from_le_bytes([
            data[offset],
            data[offset + 1],
            data[offset + 2],
            data[offset + 3],
        ])
    };
    let raw_size = field(4);
    if raw_size > MAX_RTF_RAW_BYTES {
        return Err(format!(
            "the compressed RTF body declares {raw_size} bytes; the limit is {MAX_RTF_RAW_BYTES}"
        ));
    }
    match field(8) {
        RTF_COMPRESSED => {}
        RTF_UNCOMPRESSED if raw_size as usize <= data.len() - 16 => {}
        RTF_UNCOMPRESSED => {
            return Err("the stored RTF body is shorter than it declares".to_string())
        }
        _ => return Err("the RTF body uses an unknown compression type".to_string()),
    }
    let text = compressed_rtf::decompress_rtf(data)
        .map_err(|error| format!("the compressed RTF body could not be decompressed: {error}"))?;
    // The crate widens each byte to a char without decoding, so narrowing back is lossless.
    Ok(text
        .chars()
        .map(|character| character as u32 as u8)
        .collect())
}

type Recipients = (
    Vec<EmailParticipantMetadata>,
    Vec<EmailParticipantMetadata>,
    Vec<EmailParticipantMetadata>,
);

fn read_recipients(table: &dyn TableContext, codepage: Option<i32>) -> Recipients {
    let mut recipients = Recipients::default();
    let context = table.context();
    for row in table.rows_matrix() {
        let Ok(values) = row.columns(context) else {
            continue;
        };
        let mut kind = None;
        let mut name = None;
        let mut address_type = None;
        let mut address = None;
        let mut smtp = None;
        for (column, value) in context.columns().iter().zip(values.iter()) {
            let Some(value) = value else {
                continue;
            };
            let Ok(value) = table.read_column(value, column.prop_type()) else {
                continue;
            };
            let text = || {
                property_text(&value, codepage)
                    .map(|(text, _)| clean_line(&text))
                    .filter(|text| !text.is_empty())
            };
            match column.prop_id() {
                PR_RECIPIENT_TYPE => {
                    if let PropertyValue::Integer32(value) = value {
                        kind = Some(value & 0x0F);
                    }
                }
                PR_DISPLAY_NAME => name = text(),
                PR_ADDRTYPE => address_type = text(),
                PR_EMAIL_ADDRESS => address = text(),
                PR_SMTP_ADDRESS => smtp = text(),
                _ => {}
            }
        }
        let email = smtp.or_else(|| {
            (address_type.as_deref() == Some("SMTP"))
                .then_some(address)
                .flatten()
        });
        if name.is_none() && email.is_none() {
            continue;
        }
        let participant = EmailParticipantMetadata { name, email };
        match kind {
            Some(2) => recipients.1.push(participant),
            Some(3) => recipients.2.push(participant),
            _ => recipients.0.push(participant),
        }
    }
    recipients
}

fn display_recipients(value: Option<String>) -> Vec<EmailParticipantMetadata> {
    value
        .map(|value| {
            value
                .split(';')
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .map(|name| EmailParticipantMetadata {
                    name: Some(name.to_string()),
                    email: None,
                })
                .collect()
        })
        .unwrap_or_default()
}

/// A FILETIME counts 100 ns intervals from 1601-01-01 UTC. Zero means "not set" in MAPI, so it
/// and anything outside chrono's range are absent rather than rendered as 1601.
fn filetime_rfc3339(value: i64) -> Option<String> {
    if value <= 0 {
        return None;
    }
    let seconds = value / 10_000_000 - 11_644_473_600;
    let nanoseconds = u32::try_from((value % 10_000_000) * 100).ok()?;
    DateTime::<Utc>::from_timestamp(seconds, nanoseconds)
        .map(|time| time.to_rfc3339_opts(SecondsFormat::AutoSi, true))
}

/// Decodes a string property. `String8` bytes are in the message codepage; the note says when
/// they were not decoded exactly.
fn property_text(value: &PropertyValue, codepage: Option<i32>) -> Option<(String, Option<String>)> {
    match value {
        PropertyValue::Unicode(value) => {
            let units = value.buffer();
            let end = units
                .iter()
                .rposition(|unit| *unit != 0)
                .map_or(0, |index| index + 1);
            Some(match String::from_utf16(&units[..end]) {
                Ok(text) => (text, None),
                Err(_) => (
                    String::from_utf16_lossy(&units[..end]),
                    Some(
                        "A text property held invalid UTF-16; the invalid units were replaced."
                            .to_string(),
                    ),
                ),
            })
        }
        PropertyValue::String8(value) => {
            let bytes = value.buffer();
            let end = bytes
                .iter()
                .rposition(|byte| *byte != 0)
                .map_or(0, |index| index + 1);
            Some(decode_codepage(&bytes[..end], codepage))
        }
        _ => None,
    }
}

/// Decodes exactly the code pages `eml.rs` decodes exactly: UTF-8, US-ASCII, ISO-8859-1 and
/// windows-1252. Anything else is read as UTF-8 with replacement and says so; guessing a mapping
/// would put invented words into evidence.
fn decode_codepage(bytes: &[u8], codepage: Option<i32>) -> (String, Option<String>) {
    if bytes.is_ascii() {
        return (String::from_utf8_lossy(bytes).into_owned(), None);
    }
    match codepage {
        Some(1252) => (bytes.iter().map(|byte| windows_1252_char(*byte)).collect(), None),
        Some(28591) => (bytes.iter().map(|byte| char::from(*byte)).collect(), None),
        Some(65001) => match std::str::from_utf8(bytes) {
            Ok(text) => (text.to_string(), None),
            Err(_) => (
                String::from_utf8_lossy(bytes).into_owned(),
                Some("Text declared UTF-8 held invalid bytes; they were replaced.".to_string()),
            ),
        },
        Some(codepage) => (
            String::from_utf8_lossy(bytes).into_owned(),
            Some(format!(
                "Text in code page {codepage} is not decoded by this reader; it was read as UTF-8 with replacement."
            )),
        ),
        None => (
            String::from_utf8_lossy(bytes).into_owned(),
            Some("Text with no recorded code page was read as UTF-8 with replacement.".to_string()),
        ),
    }
}

fn clean_line(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect::<String>()
        .trim()
        .to_string()
}

/// Line breaks and tabs survive; other control characters would trip the plain-text reader's
/// binary check on the child document.
fn clean_text(value: &str) -> String {
    value
        .replace("\r\n", "\n")
        .chars()
        .filter(|character| !character.is_control() || matches!(character, '\n' | '\t'))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const TIKA_FIXTURE: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/fixtures/pst/tika-various-body-types.pst"
    );
    const LIBPST_FIXTURE: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/fixtures/pst/libpst-dist-list.pst"
    );

    fn extract(source: &Path) -> (tempfile::TempDir, Result<Vec<PathBuf>, AppError>) {
        let directory = tempfile::tempdir().expect("temp dir");
        let destination = directory.path().join("contents");
        let result = extract_pst_container_with_manifest(source, &destination);
        (directory, result)
    }

    fn read(path: &Path) -> String {
        fs::read_to_string(path).expect("child text")
    }

    fn relative(directory: &tempfile::TempDir, path: &Path) -> String {
        path.strip_prefix(directory.path().join("contents"))
            .expect("inside destination")
            .components()
            .map(|component| component.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/")
    }

    #[test]
    fn messages_enumerate_under_their_folder_with_headers_and_bodies() {
        let (directory, result) = extract(Path::new(TIKA_FIXTURE));
        let files = result.expect("the fixture extracts");
        let mut children = files
            .iter()
            .map(|path| (relative(&directory, path), read(path)))
            .collect::<Vec<_>>();
        children.sort();
        assert_eq!(
            children.len(),
            4,
            "four messages and nothing else: {children:#?}"
        );
        for (path, text) in &children {
            assert!(
                path.starts_with("Mailbox/Inbox/tmp/"),
                "folder path kept: {path}"
            );
            assert!(text.starts_with("Folder: Inbox/tmp\n"), "{text}");
            assert!(
                text.contains("From: Allison, Timothy B. <tallison@mitre.org>\n"),
                "{text}"
            );
            assert!(
                text.contains("To: Allison, Timothy B. <tallison@mitre.org>\n"),
                "{text}"
            );
        }
        let subjects = children
            .iter()
            .filter_map(|(_, text)| text.lines().find_map(|line| line.strip_prefix("Subject: ")))
            .collect::<Vec<_>>();
        assert_eq!(
            subjects
                .iter()
                .filter(|subject| **subject == "original email")
                .count(),
            1
        );
        assert_eq!(
            subjects
                .iter()
                .filter(|subject| **subject == "FW: original email")
                .count(),
            3
        );
        let original = &children
            .iter()
            .find(|(_, text)| text.contains("Subject: original email\n"))
            .expect("the original message")
            .1;
        assert!(
            original.contains("This is the original email (html)"),
            "{original}"
        );
        assert!(
            original.contains("Message-ID: <MWHPR09MB1391E30131B0D193163AA6E0C79C0@MWHPR09MB1391.namprd09.prod.outlook.com>"),
            "{original}"
        );
        assert!(original.contains("Date: 2017-"), "{original}");
    }

    /// Every fixture message carries a plain body, so the HTML and RTF fallbacks are reached here
    /// by hiding it from the real stored properties.
    #[test]
    fn html_and_rtf_bodies_are_read_when_there_is_no_plain_text() {
        let store = PstStore::open(Path::new(TIKA_FIXTURE), PstLayout::Unicode).expect("open");
        let root = store
            .store()
            .properties()
            .ipm_sub_tree_entry_id()
            .and_then(|entry_id| store.store().open_folder(&entry_id))
            .expect("root");
        let mut stack = vec![root];
        let mut bodies = Vec::new();
        while let Some(folder) = stack.pop() {
            if let Some(table) = folder.hierarchy_table() {
                for row in table.rows_matrix() {
                    let entry_id = store.entry_id(u32::from(row.id())).expect("entry id");
                    stack.push(store.store().open_folder(&entry_id).expect("folder"));
                }
            }
            let Some(table) = folder.contents_table() else {
                continue;
            };
            for row in table.rows_matrix() {
                let entry_id = store.entry_id(u32::from(row.id())).expect("entry id");
                let message = store.open_message(&entry_id).expect("message");
                let properties = message.message().properties();
                let without_plain = |id| (id != PR_BODY).then(|| properties.get(id)).flatten();
                let source = if properties.get(PR_HTML).is_some() {
                    "html"
                } else if properties.get(PR_RTF_COMPRESSED).is_some() {
                    "rtf"
                } else {
                    "none"
                };
                bodies.push((
                    source,
                    select_body(without_plain, message_codepage(properties)),
                ));
            }
        }
        let html = bodies
            .iter()
            .filter(|(source, _)| *source == "html")
            .map(|(_, body)| body.as_ref().expect("HTML body").0.clone())
            .collect::<Vec<_>>();
        assert_eq!(html.len(), 2);
        assert!(html
            .iter()
            .any(|body| body.contains("This is the original email (html)")));
        assert!(
            html.iter().all(|body| !body.contains("<html")
                && !body.contains("<p")
                && !body.contains("<div")),
            "markup stripped: {html:?}"
        );
        let rtf = bodies
            .iter()
            .filter(|(source, _)| *source == "rtf")
            .map(|(_, body)| body.as_ref().expect("RTF body").0.clone())
            .collect::<Vec<_>>();
        assert_eq!(rtf.len(), 1);
        assert!(rtf[0].contains("original email"), "{rtf:?}");
        assert!(!rtf[0].contains("\\rtf"), "control words stripped: {rtf:?}");
        let none = bodies
            .iter()
            .filter(|(source, _)| *source == "none")
            .collect::<Vec<_>>();
        assert_eq!(none.len(), 1);
        assert!(
            none[0].1.is_err(),
            "a missing body is reported, not invented"
        );
    }

    /// The dist-list fixture holds an appointment whose two attachments are attached messages,
    /// which outlook-pst 1.2.0 cannot open, and a distribution list it cannot read at all. Both
    /// must cost only their own entry, and both must be written down.
    #[test]
    fn unreadable_messages_and_attachments_are_recorded_not_fatal() {
        let (directory, result) = extract(Path::new(LIBPST_FIXTURE));
        let files = result.expect("one bad entry must not abort the mailbox");
        let by_name = files
            .iter()
            .map(|path| (relative(&directory, path), read(path)))
            .collect::<Vec<_>>();
        let appointment = &by_name
            .iter()
            .find(|(_, text)| text.contains("Subject: Test appointment\n"))
            .expect("the appointment is extracted")
            .1;
        assert!(
            appointment.starts_with("Folder: Calendar\n"),
            "{appointment}"
        );
        assert!(
            appointment.contains("This is a complete test"),
            "{appointment}"
        );
        assert_eq!(
            appointment
                .matches("not extracted: the attachment could not be read")
                .count(),
            2,
            "{appointment}"
        );
        let notes = &by_name
            .iter()
            .find(|(path, _)| path == NOTES_FILE)
            .expect("problems are recorded in the notes file")
            .1;
        assert!(
            notes.starts_with("Outlook PST extraction notes\n"),
            "{notes}"
        );
        assert!(notes.contains("Contacts: message"), "{notes}");
        assert!(notes.contains("could not be read"), "{notes}");
    }

    /// The cap counts directories as well as files, because the cached re-run lists both.
    #[test]
    fn the_entry_cap_counts_directories_and_holds_at_the_limit() {
        let attempt = |limit: usize| {
            let directory = tempfile::tempdir().expect("temp dir");
            let mut output = Output::new(directory.path(), limit);
            output
                .write_file(PathBuf::from("Inbox/one.txt"), b"x")
                .and_then(|()| output.write_file(PathBuf::from("Inbox/two.txt"), b"y"))
        };
        // Inbox plus two files is three entries.
        assert!(
            attempt(2).is_err_and(|error| error.to_string().contains("PST_ENTRY_COUNT_EXCEEDED"))
        );
        assert!(attempt(3).is_ok());
        assert!(attempt(4).is_ok());
    }

    /// What `extraction.rs` sees when the reader dies: an error, no destination, no staging left.
    #[test]
    fn a_crashed_or_hung_reader_commits_nothing_and_leaves_no_staging() {
        for fault in ["abort", "hang"] {
            let directory = tempfile::tempdir().expect("temp dir");
            let destination = directory.path().join("contents");
            let result = extract_in_child(
                Path::new(TIKA_FIXTURE),
                &destination,
                |_| ChildBudget {
                    timeout: std::time::Duration::from_secs(1),
                    max_entries: MAX_ENTRY_COUNT,
                    max_bytes: MAX_TOTAL_BYTES,
                },
                |command| {
                    command.env(crate::parsers::pst_isolation::tests::FAULT_ENV, fault);
                },
            );
            assert!(result.is_err(), "{fault}: {result:?}");
            assert!(!destination.exists(), "{fault}: nothing committed");
            let leftovers = fs::read_dir(directory.path())
                .expect("list")
                .filter_map(Result::ok)
                .count();
            assert_eq!(leftovers, 0, "{fault}: staging removed");
        }
    }

    /// Sparse files, so the real 8 GiB boundary costs no disk.
    #[test]
    fn the_pst_source_cap_admits_the_limit_and_refuses_one_byte_more() {
        let directory = tempfile::tempdir().expect("temp dir");
        for (length, admitted) in [
            (MAX_PST_BYTES - 1, true),
            (MAX_PST_BYTES, true),
            (MAX_PST_BYTES + 1, false),
        ] {
            let source = directory.path().join(format!("{length}.pst"));
            let file = File::create(&source).expect("create");
            file.set_len(length).expect("sparse length");
            let error = extract_pst_container_with_manifest(&source, directory.path().join("out"))
                .expect_err("zeros are not a mailbox")
                .to_string();
            assert_eq!(
                !error.contains("PST_SIZE_INVALID"),
                admitted,
                "{length}: {error}"
            );
            fs::remove_file(&source).expect("remove");
        }
    }

    #[test]
    fn a_truncated_mailbox_fails_cleanly_and_leaves_no_staging() {
        let original = fs::read(TIKA_FIXTURE).expect("fixture");
        for length in [
            12,
            512,
            4_096,
            65_536,
            original.len() / 2,
            original.len() - 1,
        ] {
            let directory = tempfile::tempdir().expect("temp dir");
            let source = directory.path().join("cut.pst");
            fs::write(&source, &original[..length]).expect("write");
            let destination = directory.path().join("contents");
            let result = extract_pst_container_with_manifest(&source, &destination);
            if let Ok(files) = &result {
                // A cut past every structure the reader touches can still be readable; it must
                // then be complete, never a partial tree.
                assert!(!files.is_empty(), "length {length}");
            }
            let leftovers = fs::read_dir(directory.path())
                .expect("list")
                .filter_map(Result::ok)
                .filter(|entry| entry.file_name().to_string_lossy().ends_with(".extracting"))
                .count();
            assert_eq!(leftovers, 0, "staging removed at length {length}");
            assert_eq!(destination.exists(), result.is_ok(), "length {length}");
        }
    }

    #[test]
    fn corrupted_bytes_fail_per_entry_or_whole_file_without_panicking() {
        let original = fs::read(TIKA_FIXTURE).expect("fixture");
        let mut seed = 0x9E37_79B9_7F4A_7C15_u64;
        for _ in 0..64 {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            let mut bytes = original.clone();
            // Leave the 12-byte prefix intact so the reader, not the header check, sees the damage.
            let offset = 12 + (seed as usize) % (bytes.len() - 12);
            bytes[offset] ^= ((seed >> 32) as u8) | 1;
            let directory = tempfile::tempdir().expect("temp dir");
            let source = directory.path().join("flipped.pst");
            fs::write(&source, &bytes).expect("write");
            // The child contains a panic now; it is still a reader defect worth failing on.
            let result =
                extract_pst_container_with_manifest(&source, directory.path().join("contents"));
            assert!(
                !result.is_err_and(|error| error.to_string().contains("PST_CHILD_CRASHED")),
                "seed {seed:x}"
            );
        }
    }

    #[test]
    fn stores_the_reader_cannot_open_are_named_before_it_runs() {
        let original = fs::read(TIKA_FIXTURE).expect("fixture");
        let directory = tempfile::tempdir().expect("temp dir");
        let write = |name: &str, patch: &[(usize, u8)]| {
            let mut bytes = original.clone();
            for (offset, value) in patch {
                bytes[*offset] = *value;
            }
            let path = directory.path().join(name);
            fs::write(&path, bytes).expect("write");
            path
        };
        assert_eq!(
            inspect_pst_header(&write("unicode.pst", &[])).expect("header"),
            Ok(PstLayout::Unicode)
        );
        assert_eq!(
            inspect_pst_header(&write("ansi.pst", &[(10, 14), (11, 0)])).expect("header"),
            Ok(PstLayout::Ansi)
        );
        let ost = inspect_pst_header(&write("offline.ost", &[(9, b'O')])).expect("header");
        assert!(ost.is_err_and(|reason| reason.contains("OST")), "{ost:?}");
        let four_k = inspect_pst_header(&write("4k.pst", &[(10, 36), (11, 0)])).expect("header");
        assert!(four_k.is_err(), "{four_k:?}");
        let extracted = extract_pst_container_with_manifest(
            write("offline2.ost", &[(9, b'O')]),
            directory.path().join("contents"),
        );
        assert!(extracted.is_err_and(|error| error.to_string().contains("PST_FORMAT_UNSUPPORTED")),);
    }

    fn compressed_rtf_with_raw_size(raw_size: u32) -> Vec<u8> {
        let mut data = compressed_rtf::compress_rtf("{\\rtf1\\ansi body text}").expect("compress");
        // The CRC covers only the bytes after the 16-byte header, so the declared size can be
        // rewritten without invalidating it.
        data[4..8].copy_from_slice(&raw_size.to_le_bytes());
        data
    }

    #[test]
    fn compressed_rtf_is_checked_before_the_decompressor_sees_it() {
        for raw_size in [MAX_RTF_RAW_BYTES - 1, MAX_RTF_RAW_BYTES] {
            let rtf = decompress_rtf_body(&compressed_rtf_with_raw_size(raw_size))
                .expect("within the limit");
            assert!(rtf.starts_with(b"{\\rtf1"));
        }
        assert!(decompress_rtf_body(&compressed_rtf_with_raw_size(MAX_RTF_RAW_BYTES + 1)).is_err());
        // Each of these panics inside compressed-rtf 1.0.1 when passed through unchecked.
        let mut short = vec![0_u8; 15];
        short[0..4].copy_from_slice(&11_u32.to_le_bytes());
        short[8..12].copy_from_slice(&RTF_COMPRESSED.to_le_bytes());
        assert!(decompress_rtf_body(&short).is_err());
        let mut uncompressed = vec![0_u8; 16];
        uncompressed[0..4].copy_from_slice(&12_u32.to_le_bytes());
        uncompressed[4..8].copy_from_slice(&1_000_u32.to_le_bytes());
        uncompressed[8..12].copy_from_slice(&RTF_UNCOMPRESSED.to_le_bytes());
        assert!(decompress_rtf_body(&uncompressed).is_err());
    }

    #[test]
    fn eight_bit_text_is_decoded_only_where_the_mapping_is_known() {
        assert_eq!(
            decode_codepage(b"caf\xe9 \x93q\x94", Some(1252)),
            ("café “q”".to_string(), None)
        );
        assert_eq!(
            decode_codepage(b"caf\xe9", Some(28591)),
            ("café".to_string(), None)
        );
        let (text, note) = decode_codepage(b"\xc4\xe3", Some(936));
        assert!(text.contains('\u{FFFD}') && note.is_some_and(|note| note.contains("936")));
        assert!(decode_codepage(b"caf\xe9", None).1.is_some());
    }

    /// Neither fixture holds a by-value attachment, so the bytes are supplied directly.
    #[test]
    fn a_by_value_attachment_is_written_beside_its_message_and_listed_on_it() {
        use outlook_pst::ltp::prop_context::BinaryValue;

        let directory = tempfile::tempdir().expect("temp dir");
        let mut output = Output::new(directory.path(), MAX_ENTRY_COUNT);
        let data = AttachmentData::Binary(BinaryValue::new(b"%PDF-1.7 evidence".to_vec()));
        let attachments = Path::new("Mailbox/Inbox/00200024 Budget.attachments");
        let line = write_attachment(|_| None, Some(&data), 1, attachments, "Inbox", &mut output)
            .expect("written");
        assert_eq!(line, "1. attachment 1 (extracted as 1 attachment 1)");
        assert_eq!(output.files, vec![attachments.join("1 attachment 1")]);
        assert_eq!(
            fs::read(directory.path().join(attachments).join("1 attachment 1")).expect("bytes"),
            b"%PDF-1.7 evidence"
        );
        let referenced =
            write_attachment(|_| None, None, 2, attachments, "Inbox", &mut output).expect("listed");
        assert!(referenced.contains("not extracted"), "{referenced}");
        assert_eq!(output.files.len(), 1, "a reference writes nothing");
    }

    #[test]
    fn a_long_attachment_name_keeps_the_extension_its_format_is_detected_by() {
        let name = attachment_file_name(&format!("{}.pdf", "x".repeat(200)), None, 3);
        assert!(name.starts_with("3 x") && name.ends_with("x.pdf"), "{name}");
        assert!(name.len() <= MAX_NAME_BYTES + "3 .pdf".len(), "{name}");
        assert_eq!(
            attachment_file_name("../../report", Some(".DOCX".to_string()), 1),
            "1 report.DOCX"
        );
    }

    #[test]
    fn an_unset_filetime_is_absent_rather_than_1601() {
        assert_eq!(filetime_rfc3339(0), None);
        assert_eq!(filetime_rfc3339(-1), None);
        assert_eq!(
            filetime_rfc3339(116_444_736_000_000_000).as_deref(),
            Some("1970-01-01T00:00:00Z")
        );
    }
}
