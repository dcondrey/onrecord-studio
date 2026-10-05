//! Reads named members of a remote zip archive through byte ranges: the end record, the central
//! directory, and each wanted member, never the archive itself. The IRS batch archives this was
//! written for are about 1 GB with a 15 MB directory.

use crate::error::AppError;
use std::future::Future;
use std::io::Read;

const END_RECORD: u32 = 0x0605_4b50;
const ZIP64_LOCATOR: u32 = 0x0706_4b50;
const ZIP64_END_RECORD: u32 = 0x0606_4b50;
const DIRECTORY_ENTRY: u32 = 0x0201_4b50;
const LOCAL_HEADER: u32 = 0x0403_4b50;
/// End record (22) with the longest comment (65,535), the zip64 locator (20) and record (56).
const TAIL_BYTES: u64 = 22 + 65_535 + 20 + 56;
const MAX_DIRECTORY_BYTES: u64 = 64 * 1024 * 1024;
const MAX_MEMBER_BYTES: u64 = 64 * 1024 * 1024;

/// A source that answers byte ranges of one resource of fixed content.
pub trait RangeSource {
    /// Bytes `first..=last`.
    fn span(&self, first: u64, last: u64) -> impl Future<Output = Result<Vec<u8>, AppError>>;
    /// The last `length` bytes (the whole resource when it is shorter) and the resource's size.
    fn suffix(&self, length: u64) -> impl Future<Output = Result<(Vec<u8>, u64), AppError>>;
}

fn malformed(detail: &str) -> AppError {
    AppError::DataParsing(format!(
        "The remote archive is not a readable zip: {detail}"
    ))
}

fn u16_at(bytes: &[u8], at: usize) -> Result<u16, AppError> {
    bytes
        .get(at..at + 2)
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
        .ok_or_else(|| malformed("a record is truncated"))
}

fn u32_at(bytes: &[u8], at: usize) -> Result<u32, AppError> {
    bytes
        .get(at..at + 4)
        .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .ok_or_else(|| malformed("a record is truncated"))
}

fn u64_at(bytes: &[u8], at: usize) -> Result<u64, AppError> {
    bytes
        .get(at..at + 8)
        .map(|b| u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]))
        .ok_or_else(|| malformed("a record is truncated"))
}

struct Entry {
    name: String,
    method: u16,
    crc32: u32,
    compressed: u64,
    uncompressed: u64,
    header_offset: u64,
}

enum Directory {
    /// Offset and size.
    At(u64, u64),
    /// The zip64 end record lies before the bytes read so far.
    Zip64RecordAt(u64),
}

/// Where the central directory lies, from the archive's last bytes.
fn locate_directory(tail: &[u8], tail_first: u64) -> Result<Directory, AppError> {
    let end = (0..=tail.len().saturating_sub(22))
        .rev()
        .find(|&at| {
            u32_at(tail, at).ok() == Some(END_RECORD)
                && u16_at(tail, at + 20)
                    .ok()
                    .is_some_and(|comment| at + 22 + comment as usize == tail.len())
        })
        .ok_or_else(|| malformed("no end-of-directory record"))?;
    let size = u32_at(tail, end + 12)?;
    let offset = u32_at(tail, end + 16)?;
    if u16_at(tail, end + 10)? != u16::MAX && size != u32::MAX && offset != u32::MAX {
        return Ok(Directory::At(u64::from(offset), u64::from(size)));
    }
    let locator = end
        .checked_sub(20)
        .filter(|&at| u32_at(tail, at).ok() == Some(ZIP64_LOCATOR))
        .ok_or_else(|| malformed("no zip64 locator"))?;
    let record_offset = u64_at(tail, locator + 8)?;
    match record_offset
        .checked_sub(tail_first)
        .and_then(|at| usize::try_from(at).ok())
        .filter(|&at| u32_at(tail, at).ok() == Some(ZIP64_END_RECORD))
    {
        Some(at) => Ok(Directory::At(
            u64_at(tail, at + 48)?,
            u64_at(tail, at + 40)?,
        )),
        None => Ok(Directory::Zip64RecordAt(record_offset)),
    }
}

fn parse_directory(directory: &[u8]) -> Result<Vec<Entry>, AppError> {
    let mut entries = Vec::new();
    let mut at = 0;
    while at < directory.len() {
        if u32_at(directory, at)? != DIRECTORY_ENTRY {
            return Err(malformed("a directory entry has the wrong signature"));
        }
        let name_length = u16_at(directory, at + 28)? as usize;
        let extra_length = u16_at(directory, at + 30)? as usize;
        let comment_length = u16_at(directory, at + 32)? as usize;
        let name_end = at + 46 + name_length;
        let extra = directory
            .get(name_end..name_end + extra_length)
            .ok_or_else(|| malformed("a directory entry is truncated"))?;
        let mut entry = Entry {
            name: String::from_utf8_lossy(&directory[at + 46..name_end]).into_owned(),
            method: u16_at(directory, at + 10)?,
            crc32: u32_at(directory, at + 16)?,
            compressed: u64::from(u32_at(directory, at + 20)?),
            uncompressed: u64::from(u32_at(directory, at + 24)?),
            header_offset: u64::from(u32_at(directory, at + 42)?),
        };
        if u16_at(directory, at + 8)? & 1 == 1 {
            entry.method = u16::MAX;
        }
        // The zip64 extra field carries, in this order, only the values whose 32-bit slot is
        // saturated.
        let mut field = 0;
        while field + 4 <= extra.len() {
            let length = u16_at(extra, field + 2)? as usize;
            if u16_at(extra, field)? == 1 {
                let mut value = field + 4;
                for slot in [
                    &mut entry.uncompressed,
                    &mut entry.compressed,
                    &mut entry.header_offset,
                ] {
                    if *slot == u64::from(u32::MAX) {
                        *slot = u64_at(extra, value)?;
                        value += 8;
                    }
                }
            }
            field += 4 + length;
        }
        entries.push(entry);
        at = name_end + extra_length + comment_length;
    }
    Ok(entries)
}

fn inflate(entry: &Entry, data: &[u8]) -> Result<Vec<u8>, AppError> {
    let read = |mut decoder: Box<dyn Read + '_>| {
        let mut out = Vec::new();
        decoder
            .by_ref()
            .take(entry.uncompressed + 1)
            .read_to_end(&mut out)
            .map(|_| out)
            .map_err(|error| malformed(&format!("member '{}': {error}", entry.name)))
    };
    let bytes = match entry.method {
        0 => data.to_vec(),
        8 => read(Box::new(flate2::read::DeflateDecoder::new(data)))?,
        // Deflate64, which the IRS archives use from 2025.
        9 => read(Box::new(deflate64::Deflate64Decoder::new(data)))?,
        _ => {
            return Err(malformed(&format!(
                "member '{}' is encrypted or uses an unsupported compression method",
                entry.name
            )))
        }
    };
    let mut crc = flate2::Crc::new();
    crc.update(&bytes);
    if bytes.len() as u64 != entry.uncompressed || crc.sum() != entry.crc32 {
        return Err(malformed(&format!(
            "member '{}' does not match the size and checksum its directory entry records",
            entry.name
        )));
    }
    Ok(bytes)
}

/// A member's name as the archive's directory writes it, and its bytes.
pub type Member = (String, Vec<u8>);

/// The member with each wanted file name, in the order asked, `None` for one the directory
/// lacks. A file name matches in any directory and without regard to ASCII case, because the
/// archives this reads place the same kind of member under a directory in some years and at the
/// top level in others. Every returned member passed its CRC-32.
pub async fn read_members<S: RangeSource>(
    source: &S,
    wanted: &[String],
) -> Result<Vec<Option<Member>>, AppError> {
    let (tail, total) = source.suffix(TAIL_BYTES).await?;
    let tail_first = total
        .checked_sub(tail.len() as u64)
        .ok_or_else(|| malformed("the source returned more bytes than the archive holds"))?;
    let (offset, size) = match locate_directory(&tail, tail_first)? {
        Directory::At(offset, size) => (offset, size),
        Directory::Zip64RecordAt(record_offset) => {
            let record = source.span(record_offset, record_offset + 55).await?;
            if u32_at(&record, 0)? != ZIP64_END_RECORD {
                return Err(malformed("no zip64 end-of-directory record"));
            }
            (u64_at(&record, 48)?, u64_at(&record, 40)?)
        }
    };
    let directory_end = offset
        .checked_add(size)
        .filter(|&end| end <= total)
        .ok_or_else(|| malformed("the directory lies outside the archive"))?;
    if size > MAX_DIRECTORY_BYTES {
        return Err(malformed("the directory exceeds the 64 MB limit"));
    }
    let directory = if offset >= tail_first {
        tail[(offset - tail_first) as usize..(directory_end - tail_first) as usize].to_vec()
    } else if size == 0 {
        Vec::new()
    } else {
        source.span(offset, directory_end - 1).await?
    };
    let entries = parse_directory(&directory)?;
    let mut members = Vec::with_capacity(wanted.len());
    for name in wanted {
        let Some(entry) = entries.iter().find(|e| {
            e.name
                .rsplit('/')
                .next()
                .is_some_and(|file| file.eq_ignore_ascii_case(name))
        }) else {
            members.push(None);
            continue;
        };
        if entry.compressed > MAX_MEMBER_BYTES || entry.uncompressed > MAX_MEMBER_BYTES {
            return Err(malformed(&format!(
                "member '{}' exceeds the 64 MB limit",
                entry.name
            )));
        }
        // A local header's name and extra lengths may differ from the directory's.
        let header = source
            .span(entry.header_offset, entry.header_offset + 29)
            .await?;
        if u32_at(&header, 0)? != LOCAL_HEADER {
            return Err(malformed("a member's local header has the wrong signature"));
        }
        let data_first = entry.header_offset
            + 30
            + u64::from(u16_at(&header, 26)?)
            + u64::from(u16_at(&header, 28)?);
        let data = if entry.compressed == 0 {
            Vec::new()
        } else {
            if data_first + entry.compressed > offset {
                return Err(malformed("a member overlaps the directory"));
            }
            source
                .span(data_first, data_first + entry.compressed - 1)
                .await?
        };
        members.push(Some((entry.name.clone(), inflate(entry, &data)?)));
    }
    Ok(members)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::sync::Mutex;

    struct Memory {
        bytes: Vec<u8>,
        served: Mutex<u64>,
    }

    impl RangeSource for Memory {
        async fn span(&self, first: u64, last: u64) -> Result<Vec<u8>, AppError> {
            *self.served.lock().unwrap() += last - first + 1;
            Ok(self.bytes[first as usize..=last as usize].to_vec())
        }
        async fn suffix(&self, length: u64) -> Result<(Vec<u8>, u64), AppError> {
            let total = self.bytes.len() as u64;
            let first = total.saturating_sub(length);
            *self.served.lock().unwrap() += total - first;
            Ok((self.bytes[first as usize..].to_vec(), total))
        }
    }

    fn archive(members: usize, filler: usize) -> Vec<u8> {
        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let deflated = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        let stored = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        for index in 0..members {
            writer
                .start_file(format!("BATCH_01A/{index}_public.xml"), deflated)
                .unwrap();
            writer
                .write_all(format!("<Return>{index}</Return>").as_bytes())
                .unwrap();
        }
        // Incompressible bulk before the wanted member, so reading it whole would be visible.
        writer.start_file("BATCH_01A/filler.bin", stored).unwrap();
        let noise = (0..filler)
            .map(|i| (i as u32).wrapping_mul(2_654_435_761).to_le_bytes()[3])
            .collect::<Vec<_>>();
        writer.write_all(&noise).unwrap();
        writer
            .start_file("BATCH_01A/last_public.xml", deflated)
            .unwrap();
        writer.write_all(b"<Return>last</Return>").unwrap();
        writer.finish().unwrap().into_inner()
    }

    #[tokio::test]
    async fn reads_named_members_without_reading_the_archive() {
        let source = Memory {
            bytes: archive(3, 400_000),
            served: Mutex::new(0),
        };
        let members = read_members(
            &source,
            &[
                "LAST_public.xml".to_string(),
                "absent_public.xml".to_string(),
                "1_public.xml".to_string(),
            ],
        )
        .await
        .unwrap();
        assert_eq!(
            members[0],
            Some((
                "BATCH_01A/last_public.xml".to_string(),
                b"<Return>last</Return>".to_vec()
            ))
        );
        assert_eq!(members[1], None);
        assert_eq!(members[2].as_ref().unwrap().1, b"<Return>1</Return>");
        assert!(*source.served.lock().unwrap() < 100_000);
    }

    #[tokio::test]
    async fn reads_an_archive_whose_member_count_needs_the_zip64_record() {
        let source = Memory {
            bytes: archive(70_000, 0),
            served: Mutex::new(0),
        };
        let members = read_members(&source, &["69999_public.xml".to_string()])
            .await
            .unwrap();
        assert_eq!(members[0].as_ref().unwrap().1, b"<Return>69999</Return>");
    }

    #[tokio::test]
    async fn refuses_a_member_whose_bytes_do_not_match_its_recorded_checksum() {
        let mut bytes = archive(0, 1_000);
        let at = bytes
            .windows(4)
            .position(|w| w == [0x50, 0x4b, 3, 4])
            .unwrap();
        let name = u16_at(&bytes, at + 26).unwrap() as usize;
        let extra = u16_at(&bytes, at + 28).unwrap() as usize;
        bytes[at + 30 + name + extra] ^= 0xff;
        let source = Memory {
            bytes,
            served: Mutex::new(0),
        };
        assert!(read_members(&source, &["filler.bin".to_string()])
            .await
            .is_err());
    }
}
