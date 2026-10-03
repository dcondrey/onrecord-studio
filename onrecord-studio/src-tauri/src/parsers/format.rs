use std::fs::File;
use std::io::Read;
use std::path::Path;

use crate::error::AppError;

/// Header bytes read for signature matching, bounding memory per call to this constant regardless
/// of file size.
///
/// INVARIANT(parsers): Must stay at or above 262. The tar check reads `signature[257..262]`, and a
/// smaller buffer would not shrink detection loudly: tar files would silently fall through to the
/// extension fallback and still classify correctly whenever the name happens to end in `.tar`.
/// `signature_buffer_covers_the_tar_magic_offset` pins the resulting behavior.
const SIGNATURE_BYTES: usize = 1_024;

/// The tar signature check reads `signature[257..262]`, so shrinking the buffer below 262 must
/// fail the build rather than silently degrade tar detection to a filename guess.
const _: () = assert!(SIGNATURE_BYTES >= 262);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DocumentFormat {
    Pdf,
    PlainText,
    RichText,
    Spreadsheet,
    OutlookMessage,
    InternetMessage,
    EmailArchive,
    Mailbox,
    WebArchive,
    OutlookStore,
    MailStore,
    Image,
    Zip,
    Tar,
    TarGzip,
    Gzip,
    Bzip2,
    TarBzip2,
    Xz,
    TarXz,
    Zstd,
    TarZstd,
    Cab,
    Cpio,
    UnixCompress,
    Rar,
    SevenZip,
    DiskOrPackageImage,
    WordPackage,
    PresentationPackage,
    OpenDocumentText,
    OpenDocumentPresentation,
    IWork,
    Ebook,
    LegacyWord,
    LegacyPresentation,
    Video,
    Audio,
    Sqlite,
    DuckDb,
    SqlDump,
    Database,
    Access,
    Columnar,
    MachineLearningModel,
    Shapefile,
    Dbf,
    Cad,
    ForensicContainer,
    CryptographicEnvelope,
    Encrypted,
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExtractionRoute {
    Pdf,
    PlainText,
    RichText,
    Spreadsheet,
    OutlookMessage,
    ImageOcr,
    PackageText,
    Sqlite,
    DuckDb,
    SqlDump,
    Container(&'static str),
    ToolRequired(&'static str),
    KeyRequired,
    /// A photograph or other image OCR read no text from. It is kept as evidence to view; it
    /// has nothing to search.
    ImageWithoutText,
    Unknown,
}

impl DocumentFormat {
    pub const fn extraction_route(self) -> ExtractionRoute {
        match self {
            Self::Pdf => ExtractionRoute::Pdf,
            Self::PlainText => ExtractionRoute::PlainText,
            Self::RichText => ExtractionRoute::RichText,
            Self::Spreadsheet => ExtractionRoute::Spreadsheet,
            Self::OutlookMessage => ExtractionRoute::OutlookMessage,
            Self::Image => ExtractionRoute::ImageOcr,
            Self::WordPackage
            | Self::PresentationPackage
            | Self::OpenDocumentText
            | Self::OpenDocumentPresentation
            | Self::Ebook => ExtractionRoute::PackageText,
            Self::Zip | Self::EmailArchive => ExtractionRoute::Container("zip"),
            Self::Tar => ExtractionRoute::Container("tar"),
            Self::TarGzip => ExtractionRoute::Container("tar-gzip"),
            Self::Gzip => ExtractionRoute::Container("gzip"),
            Self::Bzip2 => ExtractionRoute::Container("bzip2"),
            Self::TarBzip2 => ExtractionRoute::Container("tar-bzip2"),
            Self::Xz => ExtractionRoute::Container("xz"),
            Self::TarXz => ExtractionRoute::Container("tar-xz"),
            Self::Zstd => ExtractionRoute::Container("zstd"),
            Self::TarZstd => ExtractionRoute::Container("tar-zstd"),
            Self::Cab => ExtractionRoute::Container("cab"),
            Self::Cpio => ExtractionRoute::Container("cpio"),
            Self::UnixCompress => ExtractionRoute::Container("unix-compress"),
            Self::Rar => ExtractionRoute::Container("rar"),
            Self::SevenZip => ExtractionRoute::Container("7zip"),
            Self::InternetMessage => ExtractionRoute::ToolRequired("MIME message parser"),
            Self::Mailbox => ExtractionRoute::ToolRequired("mbox parser"),
            Self::WebArchive => ExtractionRoute::ToolRequired("WARC parser"),
            Self::OutlookStore => ExtractionRoute::Container("pst"),
            Self::MailStore => ExtractionRoute::ToolRequired("NSF parser"),
            Self::IWork => ExtractionRoute::ToolRequired("iWork IWA parser"),
            Self::LegacyWord | Self::LegacyPresentation => {
                ExtractionRoute::ToolRequired("sandboxed office converter")
            }
            Self::Video | Self::Audio => {
                ExtractionRoute::ToolRequired("media transcription pipeline")
            }
            Self::Sqlite => ExtractionRoute::Sqlite,
            Self::DuckDb => ExtractionRoute::DuckDb,
            Self::SqlDump => ExtractionRoute::SqlDump,
            Self::Database => {
                ExtractionRoute::ToolRequired("manual database format identification and export")
            }
            Self::Access => {
                ExtractionRoute::ToolRequired("manual Access export to SQLite, CSV, or SQL")
            }
            Self::Columnar => ExtractionRoute::ToolRequired("columnar data extractor"),
            Self::MachineLearningModel => {
                ExtractionRoute::ToolRequired("safe model metadata extractor")
            }
            Self::Shapefile | Self::Dbf => ExtractionRoute::ToolRequired("geospatial extractor"),
            Self::Cad => ExtractionRoute::ToolRequired("CAD extractor"),
            Self::ForensicContainer => {
                ExtractionRoute::ToolRequired("forensic container extractor")
            }
            Self::DiskOrPackageImage => {
                ExtractionRoute::ToolRequired("sandboxed package image extractor")
            }
            Self::CryptographicEnvelope => {
                ExtractionRoute::ToolRequired("cryptographic envelope verifier")
            }
            Self::Encrypted => ExtractionRoute::KeyRequired,
            Self::Unknown => ExtractionRoute::Unknown,
        }
    }
}

pub fn detect_document_format(
    path: &Path,
    declared_file_name: Option<&str>,
    content_type: Option<&str>,
) -> Result<DocumentFormat, AppError> {
    let extension = declared_file_name
        .and_then(compound_extension)
        .or_else(|| {
            path.file_name()
                .and_then(|name| name.to_str())
                .and_then(compound_extension)
        })
        .unwrap_or_default();
    let extension_format = classify_extension(&extension);
    let content_type_format = classify_content_type(content_type.unwrap_or_default());
    let declared = if extension_format == DocumentFormat::Unknown {
        content_type_format
    } else {
        extension_format
    };
    let signature = read_signature(path)?;

    // SECURITY(security): Strong signatures take precedence over untrusted names/content types.
    // `declared` derives from the portal-supplied file name and Content-Type, so it is attacker
    // controlled. It feeds the decision in two distinct places, and only the first is a narrowing:
    //
    //   1. Container disambiguation (the gzip/zip/CFB arms below). A signature has already fixed
    //      the family; `declared` only chooses a member within it, so the blast radius is bounded
    //      to that family.
    //   2. The unsignatured fallback at the end of this function. When nothing matched, `declared`
    //      is returned as-is and is the *whole* decision. That is the only route to roughly twenty
    //      variants (PlainText, Mailbox, Video, Database, Encrypted, ...), which have no magic
    //      bytes to check against.
    //
    // LIMIT(security): Case 2 means a renamed file picks its own extraction route. That is
    // tolerable because every route is expected to survive hostile input on its own, not because
    // the name was validated. Do not add a route here that assumes a trustworthy format.
    if signature.starts_with(b"%PDF-") {
        return Ok(DocumentFormat::Pdf);
    }
    if signature.starts_with(b"{\\rtf") {
        return Ok(DocumentFormat::RichText);
    }
    if signature.starts_with(b"WARC/1.") {
        return Ok(DocumentFormat::WebArchive);
    }
    if signature.starts_with(b"!BDN") {
        return Ok(DocumentFormat::OutlookStore);
    }
    if signature.starts_with(b"-----BEGIN PGP")
        || signature.starts_with(b"-----BEGIN OPEN")
        || signature.starts_with(b"ssh-rsa ")
        || signature.starts_with(b"ssh-ed25519 ")
    {
        return Ok(DocumentFormat::CryptographicEnvelope);
    }
    if signature.starts_with(b"\x89PNG\r\n\x1a\n")
        || signature.starts_with(&[0xff, 0xd8, 0xff])
        || signature.starts_with(b"II*\0")
        || signature.starts_with(b"MM\0*")
    {
        return Ok(DocumentFormat::Image);
    }
    if signature.starts_with(b"SQLite format 3\0") {
        return Ok(DocumentFormat::Sqlite);
    }
    if signature.get(8..12) == Some(&b"DUCK"[..]) {
        return Ok(DocumentFormat::DuckDb);
    }
    if signature.starts_with(b"7z\xbc\xaf'\x1c") {
        return Ok(DocumentFormat::SevenZip);
    }
    if signature.starts_with(&[0x1f, 0x8b]) {
        return Ok(match declared {
            DocumentFormat::WebArchive => DocumentFormat::WebArchive,
            DocumentFormat::TarGzip => DocumentFormat::TarGzip,
            DocumentFormat::Gzip => DocumentFormat::Gzip,
            _ => DocumentFormat::Gzip,
        });
    }
    if signature.starts_with(b"BZh") {
        return Ok(match declared {
            DocumentFormat::TarBzip2 => DocumentFormat::TarBzip2,
            _ => DocumentFormat::Bzip2,
        });
    }
    if signature.starts_with(&[0xfd, b'7', b'z', b'X', b'Z', 0x00]) {
        return Ok(match declared {
            DocumentFormat::TarXz => DocumentFormat::TarXz,
            _ => DocumentFormat::Xz,
        });
    }
    if signature.starts_with(&[0x28, 0xb5, 0x2f, 0xfd])
        || (signature.starts_with(&[0x50, 0x2a, 0x4d])
            && signature
                .get(3)
                .is_some_and(|byte| (0x18..=0x1f).contains(byte)))
    {
        return Ok(match declared {
            DocumentFormat::TarZstd => DocumentFormat::TarZstd,
            _ => DocumentFormat::Zstd,
        });
    }
    if signature.starts_with(b"MSCF") {
        return Ok(DocumentFormat::Cab);
    }
    if signature.starts_with(b"Rar!\x1a\x07") {
        return Ok(DocumentFormat::Rar);
    }
    if signature.starts_with(&[0x1f, 0x9d]) {
        return Ok(DocumentFormat::UnixCompress);
    }
    if signature.starts_with(b"070701")
        || signature.starts_with(b"070702")
        || signature.starts_with(b"070707")
    {
        return Ok(DocumentFormat::Cpio);
    }
    if signature.starts_with(b"xar!") {
        return Ok(DocumentFormat::DiskOrPackageImage);
    }
    if signature.get(257..262) == Some(&b"ustar"[..]) {
        return Ok(DocumentFormat::Tar);
    }
    if signature.starts_with(&[0xd0, 0xcf, 0x11, 0xe0, 0xa1, 0xb1, 0x1a, 0xe1]) {
        return Ok(match declared {
            DocumentFormat::Spreadsheet
            | DocumentFormat::OutlookMessage
            | DocumentFormat::LegacyWord
            | DocumentFormat::LegacyPresentation
            | DocumentFormat::Access => declared,
            _ => DocumentFormat::Unknown,
        });
    }
    if signature.starts_with(b"PAR1")
        || signature.starts_with(b"Obj\x01")
        || signature.starts_with(b"ORC")
    {
        return Ok(DocumentFormat::Columnar);
    }
    if signature.starts_with(b"PK\x03\x04")
        || signature.starts_with(b"PK\x05\x06")
        || signature.starts_with(b"PK\x07\x08")
    {
        return Ok(match declared {
            DocumentFormat::Spreadsheet
            | DocumentFormat::WordPackage
            | DocumentFormat::PresentationPackage
            | DocumentFormat::OpenDocumentText
            | DocumentFormat::OpenDocumentPresentation
            | DocumentFormat::IWork
            | DocumentFormat::Ebook
            | DocumentFormat::EmailArchive
            | DocumentFormat::WebArchive
            | DocumentFormat::ForensicContainer => declared,
            _ => DocumentFormat::Zip,
        });
    }
    if declared != DocumentFormat::Unknown {
        return Ok(declared);
    }

    Ok(DocumentFormat::Unknown)
}

fn read_signature(path: &Path) -> Result<Vec<u8>, AppError> {
    let metadata = std::fs::symlink_metadata(path).map_err(|error| {
        AppError::Internal(format!(
            "FORMAT_INSPECTION_FAILED: failed to inspect '{}': {error}",
            path.display()
        ))
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(AppError::DataParsing(format!(
            "FORMAT_SOURCE_INVALID: '{}' must be a regular, non-symlink file",
            path.display()
        )));
    }
    let mut file = File::open(path).map_err(|error| {
        AppError::Internal(format!(
            "FORMAT_READ_FAILED: failed to open '{}': {error}",
            path.display()
        ))
    })?;
    let mut signature = vec![0_u8; SIGNATURE_BYTES];
    let mut offset = 0_usize;
    while offset < signature.len() {
        let read = file.read(&mut signature[offset..]).map_err(|error| {
            AppError::Internal(format!(
                "FORMAT_READ_FAILED: failed to read '{}': {error}",
                path.display()
            ))
        })?;
        if read == 0 {
            break;
        }
        offset += read;
    }
    signature.truncate(offset);
    Ok(signature)
}

fn compound_extension(file_name: &str) -> Option<String> {
    let file_name = file_name.trim().to_ascii_lowercase();
    for extension in ["tar.bz2", "tar.zst", "tar.gz", "tar.xz", "eml.zip"] {
        if file_name.ends_with(&format!(".{extension}")) {
            return Some(extension.to_string());
        }
    }
    if file_name.ends_with(".warc.gz") || file_name.ends_with(".wrac.gz") {
        return Some("warc.gz".to_string());
    }
    Path::new(&file_name)
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_string)
}

fn classify_extension(extension: &str) -> DocumentFormat {
    // LIMIT(parsers): `svg` still has no magic-byte arm. Sniffing `<?xml` would claim every XML
    // dialect, so an SVG renamed to `.bin` and served without a Content-Type stays `Unknown`. Name
    // and Content-Type cover the portal cases; the forensic overlay audit is tracked in
    // `parsers/svg.rs`, which is where the plan lives -- not here.
    match extension {
        "pdf" => DocumentFormat::Pdf,
        "txt" | "log" | "csv" | "tsv" | "json" | "jsonl" | "ndjson" | "xml" | "yaml" | "yml"
        | "ini" | "conf" | "config" | "geojson" | "kml" | "html" | "htm" | "xhtml" | "css"
        | "sass" | "markdown" | "md" | "inf" | "reg" | "sh" | "bat" | "cmd" | "ps1" | "patch"
        | "diff" | "fods" | "sylk" | "slk" | "vcf" | "vcard" | "dxf" | "prj" | "pbtxt"
        | "prototxt" | "bib" | "ris" | "enw" | "end" | "tex" => DocumentFormat::PlainText,
        // SVG is XML, so it rides the PlainText route; `queue/extraction.rs` dispatches a PlainText document
        // named `.svg` (or typed `image/svg+xml`) to `parsers::svg::extract_svg_text` rather than
        // to the raw-text reader, which would otherwise index path data as if it were prose.
        "svg" => DocumentFormat::PlainText,
        "rtf" => DocumentFormat::RichText,
        "xls" | "xlsx" | "xlsb" | "xlsm" | "ods" => DocumentFormat::Spreadsheet,
        "msg" => DocumentFormat::OutlookMessage,
        "eml" | "emlx" => DocumentFormat::InternetMessage,
        "eml.zip" => DocumentFormat::EmailArchive,
        "mbox" => DocumentFormat::Mailbox,
        "warc" | "wrac" | "warc.gz" | "wacz" => DocumentFormat::WebArchive,
        "pst" | "ost" => DocumentFormat::OutlookStore,
        "nsf" => DocumentFormat::MailStore,
        "jpg" | "jpeg" | "png" | "tiff" | "tif" | "bmp" | "webp" => DocumentFormat::Image,
        "zip" | "kmz" => DocumentFormat::Zip,
        "tar" => DocumentFormat::Tar,
        "tar.gz" | "tgz" => DocumentFormat::TarGzip,
        "gz" => DocumentFormat::Gzip,
        "bz2" => DocumentFormat::Bzip2,
        "tar.bz2" | "tbz" | "tbz2" => DocumentFormat::TarBzip2,
        "xz" => DocumentFormat::Xz,
        "tar.xz" | "txz" => DocumentFormat::TarXz,
        "zst" => DocumentFormat::Zstd,
        "tar.zst" | "tzst" => DocumentFormat::TarZstd,
        "cab" => DocumentFormat::Cab,
        "cpio" => DocumentFormat::Cpio,
        "z" => DocumentFormat::UnixCompress,
        "rar" => DocumentFormat::Rar,
        "7z" => DocumentFormat::SevenZip,
        "dmg" | "pkg" => DocumentFormat::DiskOrPackageImage,
        "docx" => DocumentFormat::WordPackage,
        "pptx" => DocumentFormat::PresentationPackage,
        "odt" => DocumentFormat::OpenDocumentText,
        "odp" => DocumentFormat::OpenDocumentPresentation,
        "pages" | "numbers" | "key" => DocumentFormat::IWork,
        "epub" => DocumentFormat::Ebook,
        "doc" => DocumentFormat::LegacyWord,
        "ppt" => DocumentFormat::LegacyPresentation,
        "mp4" | "mov" | "avi" | "mkv" | "wmv" | "3gp" => DocumentFormat::Video,
        "mp3" | "wav" | "m4a" | "aac" | "wma" => DocumentFormat::Audio,
        "sqlite" | "sqlite3" => DocumentFormat::Sqlite,
        "duckdb" => DocumentFormat::DuckDb,
        "sql" => DocumentFormat::SqlDump,
        "db" => DocumentFormat::Database,
        "accdb" | "mdb" => DocumentFormat::Access,
        "parquet" | "avro" | "orc" => DocumentFormat::Columnar,
        "onnx" | "safetensors" | "pt" | "pth" | "pb" | "caffemodel" | "pkl" | "pickle" => {
            DocumentFormat::MachineLearningModel
        }
        "shp" | "shape" | "shx" => DocumentFormat::Shapefile,
        "dbf" => DocumentFormat::Dbf,
        "dwg" => DocumentFormat::Cad,
        "ufdr" | "ufd" => DocumentFormat::ForensicContainer,
        "p7m" | "p7s" | "pgp" | "ssh" => DocumentFormat::CryptographicEnvelope,
        "enc" | "crypto" => DocumentFormat::Encrypted,
        _ => DocumentFormat::Unknown,
    }
}

fn classify_content_type(content_type: &str) -> DocumentFormat {
    let content_type = content_type
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    match content_type.as_str() {
        "application/pdf" => DocumentFormat::Pdf,
        "text/plain"
        | "text/csv"
        | "text/tab-separated-values"
        | "text/html"
        | "text/xml"
        | "text/yaml"
        | "application/json"
        | "application/geo+json"
        | "application/xml"
        | "application/yaml"
        | "image/svg+xml"
        | "application/x-yaml" => DocumentFormat::PlainText,
        "application/rtf" | "text/rtf" => DocumentFormat::RichText,
        "application/vnd.ms-excel"
        | "application/vnd.ms-excel.sheet.binary.macroenabled.12"
        | "application/vnd.ms-excel.sheet.macroenabled.12"
        | "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
        | "application/vnd.oasis.opendocument.spreadsheet" => DocumentFormat::Spreadsheet,
        "application/vnd.ms-outlook" => DocumentFormat::OutlookMessage,
        "message/rfc822" => DocumentFormat::InternetMessage,
        "application/mbox" => DocumentFormat::Mailbox,
        "application/warc" => DocumentFormat::WebArchive,
        "image/bmp" | "image/jpeg" | "image/png" | "image/tiff" | "image/webp" => {
            DocumentFormat::Image
        }
        "application/zip" => DocumentFormat::Zip,
        "application/x-tar" => DocumentFormat::Tar,
        "application/gzip" | "application/x-gzip" => DocumentFormat::Gzip,
        "application/x-bzip2" => DocumentFormat::Bzip2,
        "application/x-xz" => DocumentFormat::Xz,
        "application/zstd" => DocumentFormat::Zstd,
        "application/vnd.rar" | "application/x-rar-compressed" => DocumentFormat::Rar,
        "application/vnd.ms-cab-compressed" => DocumentFormat::Cab,
        "application/x-cpio" => DocumentFormat::Cpio,
        "application/x-7z-compressed" => DocumentFormat::SevenZip,
        "application/epub+zip" => DocumentFormat::Ebook,
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document" => {
            DocumentFormat::WordPackage
        }
        "application/vnd.openxmlformats-officedocument.presentationml.presentation" => {
            DocumentFormat::PresentationPackage
        }
        "application/vnd.oasis.opendocument.text" => DocumentFormat::OpenDocumentText,
        "application/vnd.oasis.opendocument.presentation" => {
            DocumentFormat::OpenDocumentPresentation
        }
        "application/vnd.sqlite3" | "application/x-sqlite3" => DocumentFormat::Sqlite,
        "application/vnd.duckdb" | "application/x-duckdb" => DocumentFormat::DuckDb,
        "application/sql" | "text/sql" => DocumentFormat::SqlDump,
        value if value.starts_with("video/") => DocumentFormat::Video,
        value if value.starts_with("audio/") => DocumentFormat::Audio,
        _ => DocumentFormat::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// Writes `bytes` to a temp file and classifies it. `declared` stands in for the portal's
    /// file name and `content_type` for its Content-Type header, both attacker controlled.
    fn detect(
        bytes: &[u8],
        path_name: &str,
        declared: Option<&str>,
        content_type: Option<&str>,
    ) -> DocumentFormat {
        let directory = tempfile::tempdir().expect("temp dir");
        let path = directory.path().join(path_name);
        let mut file = File::create(&path).expect("create fixture");
        file.write_all(bytes).expect("write fixture");
        file.sync_all().expect("flush fixture");
        detect_document_format(&path, declared, content_type).expect("detection should succeed")
    }

    fn zip(declared: &str) -> DocumentFormat {
        detect(
            b"PK\x03\x04rest-of-archive",
            "fixture.bin",
            Some(declared),
            None,
        )
    }

    fn gzip(declared: &str) -> DocumentFormat {
        detect(
            b"\x1f\x8b\x08\x00\x00\x00\x00\x00",
            "fixture.bin",
            Some(declared),
            None,
        )
    }

    // SECURITY(security): A strong signature must win over the portal-supplied name and
    // Content-Type. If any of these flip, a hostile portal can pick the extraction route by
    // lying about the file name.
    #[test]
    fn magic_bytes_outrank_an_untrusted_name_and_content_type() {
        assert_eq!(
            detect(
                b"%PDF-1.7\n",
                "fixture.bin",
                Some("payload.txt"),
                Some("text/plain")
            ),
            DocumentFormat::Pdf,
        );
        assert_eq!(
            detect(b"{\\rtf1\\ansi", "fixture.bin", Some("payload.doc"), None),
            DocumentFormat::RichText,
        );
        assert_eq!(
            detect(b"WARC/1.1\r\n", "fixture.bin", Some("payload.txt"), None),
            DocumentFormat::WebArchive,
        );
        assert_eq!(
            detect(
                b"!BDN\0\0\0\0SM\x17\0",
                "fixture.bin",
                Some("mailbox.zip"),
                Some("application/zip"),
            ),
            DocumentFormat::OutlookStore,
        );
        assert_eq!(
            detect(
                b"\x89PNG\r\n\x1a\n",
                "fixture.bin",
                Some("payload.pdf"),
                Some("application/pdf"),
            ),
            DocumentFormat::Image,
        );
        assert_eq!(
            detect(
                b"SQLite format 3\0",
                "fixture.bin",
                Some("payload.db"),
                None
            ),
            DocumentFormat::Sqlite,
        );
        let mut duckdb_header = [0_u8; 16];
        duckdb_header[8..12].copy_from_slice(b"DUCK");
        assert_eq!(
            detect(
                &duckdb_header,
                "fixture.bin",
                Some("payload.sqlite"),
                Some("application/vnd.sqlite3")
            ),
            DocumentFormat::DuckDb,
        );
    }

    // Container families: the signature fixes the family, the untrusted name picks the member.
    // This is the narrowing case, and it is bounded to the family the signature already proved.
    #[test]
    fn a_container_signature_bounds_what_the_declared_name_can_select() {
        assert_eq!(zip("report.docx"), DocumentFormat::WordPackage);
        assert_eq!(zip("report.xlsx"), DocumentFormat::Spreadsheet);
        assert_eq!(zip("report.pptx"), DocumentFormat::PresentationPackage);
        assert_eq!(zip("report.odt"), DocumentFormat::OpenDocumentText);
        assert_eq!(zip("report.epub"), DocumentFormat::Ebook);
        assert_eq!(zip("report.ufdr"), DocumentFormat::ForensicContainer);

        // A name outside the passthrough list cannot escape the ZIP family.
        assert_eq!(zip("report.bin"), DocumentFormat::Zip);
        assert_eq!(zip("report.pdf"), DocumentFormat::Zip);
        // LIMIT(parsers): no central-directory or [Content_Types].xml inspection happens, so a
        // renamed .docx classifies as a plain Zip and loses the package text route.
        assert_eq!(zip("report.kmz"), DocumentFormat::Zip);
    }

    // LIMIT(parsers): tar-inside-a-compressor is decided purely by the declared name. Nothing
    // decompresses to look, so a renamed .tar.gz is indistinguishable from a plain .gz.
    #[test]
    fn tar_in_compressor_is_selected_by_name_because_nothing_decompresses_to_check() {
        assert_eq!(gzip("archive.tar.gz"), DocumentFormat::TarGzip);
        assert_eq!(gzip("archive.tgz"), DocumentFormat::TarGzip);
        assert_eq!(gzip("archive.gz"), DocumentFormat::Gzip);
        // A `.warc.gz` is a dedicated compound extension, not a tar.
        assert_eq!(gzip("crawl.warc.gz"), DocumentFormat::WebArchive);
        // A declared plain `.tar` inside a gzip stream falls to the wildcard arm, not TarGzip.
        assert_eq!(gzip("archive.tar"), DocumentFormat::Gzip);
        // No Content-Type maps to a tar-in-compressor variant, so a header-only classification
        // can never reach TarGzip.
        assert_eq!(
            detect(
                b"\x1f\x8b\x08\x00\x00\x00\x00\x00",
                "fixture.bin",
                None,
                Some("application/gzip"),
            ),
            DocumentFormat::Gzip,
        );
    }

    #[test]
    fn the_declared_extension_outranks_the_declared_content_type() {
        assert_eq!(
            detect(
                b"plain words",
                "fixture.bin",
                Some("notes.csv"),
                Some("application/pdf")
            ),
            DocumentFormat::PlainText,
        );
        // Content-Type is consulted only when the extension is unrecognized.
        assert_eq!(
            detect(
                b"plain words",
                "fixture.bin",
                Some("notes.bogus"),
                Some("application/pdf")
            ),
            DocumentFormat::Pdf,
        );
        assert_eq!(
            detect(
                b"plain words",
                "fixture.bin",
                None,
                Some("text/plain; charset=utf-8")
            ),
            DocumentFormat::PlainText,
        );
        assert_eq!(
            detect(b"plain words", "fixture.bin", None, Some("APPLICATION/PDF")),
            DocumentFormat::Pdf,
        );
    }

    // INVARIANT(parsers): a declared name that HAS an extension, even an unknown one, suppresses
    // the path's own extension entirely. Non-obvious, and it is why the case above resolves via
    // Content-Type rather than via the path.
    #[test]
    fn a_declared_name_with_any_extension_suppresses_the_path_extension() {
        assert_eq!(
            detect(b"plain words", "real.pdf", Some("payload.bogus"), None),
            DocumentFormat::Unknown,
        );
        // With no extension at all in the declared name, the path is consulted.
        assert_eq!(
            detect(b"plain words", "real.pdf", Some("receipt"), None),
            DocumentFormat::Pdf,
        );
    }

    // Short ASCII prefixes are real magic numbers here, so ordinary prose that happens to start
    // with one is misclassified. Pinned so the surprise is deliberate rather than discovered.
    #[test]
    fn short_ascii_signatures_outrank_a_plain_text_extension() {
        assert_eq!(
            detect(
                b"ORCHESTRA MEETING NOTES\n",
                "fixture.bin",
                Some("notes.txt"),
                None
            ),
            DocumentFormat::Columnar,
        );
        assert_eq!(
            detect(
                b"070707 audit log line",
                "fixture.bin",
                Some("notes.txt"),
                None
            ),
            DocumentFormat::Cpio,
        );
    }

    // INVARIANT(parsers): the tar check reads signature[257..262], so SIGNATURE_BYTES must stay
    // at or above 262. This test fails loudly if someone shrinks the buffer.
    #[test]
    fn signature_buffer_covers_the_tar_magic_offset() {
        // The 262-byte floor itself is enforced at compile time next to the constant; this test
        // covers the behavior that floor exists to protect.
        let mut header = vec![0_u8; 512];
        header[257..262].copy_from_slice(b"ustar");
        assert_eq!(
            detect(&header, "fixture.bin", Some("archive.bin"), None),
            DocumentFormat::Tar,
        );
        // A tar truncated below the magic offset resolves by name instead, via the fallback.
        assert_eq!(
            detect(&header[..128], "fixture.bin", Some("archive.tar"), None),
            DocumentFormat::Tar,
        );
    }

    // SVG resolves through the PlainText route by name and by Content-Type; `parsers::svg` then
    // supplies the extractor. The third case pins the remaining hole: with no magic-byte arm, a
    // renamed SVG carrying no Content-Type is still Unknown.
    #[test]
    fn svg_resolves_to_the_plain_text_route_by_name_and_by_content_type() {
        let svg = b"<?xml version=\"1.0\"?><svg xmlns=\"http://www.w3.org/2000/svg\"/>";
        assert_eq!(
            detect(svg, "fixture.bin", Some("diagram.svg"), None),
            DocumentFormat::PlainText,
        );
        assert_eq!(
            detect(svg, "fixture.bin", None, Some("image/svg+xml")),
            DocumentFormat::PlainText,
        );
        assert_eq!(
            detect(svg, "fixture.bin", Some("diagram.bin"), None),
            DocumentFormat::Unknown,
        );
    }

    #[test]
    fn unrecognized_input_resolves_to_unknown() {
        assert_eq!(
            detect(
                b"plain words",
                "fixture.bin",
                Some("payload.unknownext"),
                Some("application/x-nonsense")
            ),
            DocumentFormat::Unknown,
        );
    }

    // SECURITY(security): detection refuses symlinks and non-regular files before opening them,
    // so a symlinked download cannot redirect a parser at an arbitrary path.
    #[test]
    #[cfg(unix)]
    fn detection_refuses_symlinks_and_directories() {
        let directory = tempfile::tempdir().expect("temp dir");
        let target = directory.path().join("real.pdf");
        std::fs::write(&target, b"%PDF-1.7\n").expect("write target");
        let link = directory.path().join("link.pdf");
        std::os::unix::fs::symlink(&target, &link).expect("symlink");

        let error = detect_document_format(&link, Some("link.pdf"), None)
            .expect_err("a symlink must be refused");
        assert!(
            format!("{error}").contains("FORMAT_SOURCE_INVALID"),
            "expected FORMAT_SOURCE_INVALID, got: {error}",
        );

        let error = detect_document_format(directory.path(), Some("dir.pdf"), None)
            .expect_err("a directory must be refused");
        assert!(
            format!("{error}").contains("FORMAT_SOURCE_INVALID"),
            "got: {error}"
        );
    }

    // INVARIANT(parsers): every format must have a real extraction route. Only DocumentFormat
    //::Unknown may map to ExtractionRoute::Unknown, so a newly added variant cannot silently
    // become unextractable.
    #[test]
    fn every_format_except_unknown_has_an_extraction_route() {
        const ALL: &[DocumentFormat] = &[
            DocumentFormat::Pdf,
            DocumentFormat::PlainText,
            DocumentFormat::RichText,
            DocumentFormat::Spreadsheet,
            DocumentFormat::OutlookMessage,
            DocumentFormat::InternetMessage,
            DocumentFormat::EmailArchive,
            DocumentFormat::Mailbox,
            DocumentFormat::WebArchive,
            DocumentFormat::OutlookStore,
            DocumentFormat::MailStore,
            DocumentFormat::Image,
            DocumentFormat::Zip,
            DocumentFormat::Tar,
            DocumentFormat::TarGzip,
            DocumentFormat::Gzip,
            DocumentFormat::Bzip2,
            DocumentFormat::TarBzip2,
            DocumentFormat::Xz,
            DocumentFormat::TarXz,
            DocumentFormat::Zstd,
            DocumentFormat::TarZstd,
            DocumentFormat::Cab,
            DocumentFormat::Cpio,
            DocumentFormat::UnixCompress,
            DocumentFormat::Rar,
            DocumentFormat::SevenZip,
            DocumentFormat::DiskOrPackageImage,
            DocumentFormat::WordPackage,
            DocumentFormat::PresentationPackage,
            DocumentFormat::OpenDocumentText,
            DocumentFormat::OpenDocumentPresentation,
            DocumentFormat::IWork,
            DocumentFormat::Ebook,
            DocumentFormat::LegacyWord,
            DocumentFormat::LegacyPresentation,
            DocumentFormat::Video,
            DocumentFormat::Audio,
            DocumentFormat::Sqlite,
            DocumentFormat::DuckDb,
            DocumentFormat::SqlDump,
            DocumentFormat::Database,
            DocumentFormat::Access,
            DocumentFormat::Columnar,
            DocumentFormat::MachineLearningModel,
            DocumentFormat::Shapefile,
            DocumentFormat::Dbf,
            DocumentFormat::Cad,
            DocumentFormat::ForensicContainer,
            DocumentFormat::CryptographicEnvelope,
            DocumentFormat::Encrypted,
        ];
        for format in ALL {
            assert_ne!(
                format.extraction_route(),
                ExtractionRoute::Unknown,
                "{format:?} has no extraction route",
            );
        }
        assert_eq!(
            DocumentFormat::Unknown.extraction_route(),
            ExtractionRoute::Unknown,
        );
    }
}
