//! Metadata from TIFF-structured files without reading their pixels.
//!
//! In a TIFF the EXIF tags are ordinary directory entries scattered through
//! the file, so the obvious way to parse them is to load the whole file,
//! which for a 16-bit scan or a layered export is hundreds of megabytes or
//! more. Instead this walks the directories with seeks, copies just the
//! first, EXIF, GPS and interoperability directories with their small
//! values, and writes them out as a tiny standalone TIFF. That block is what
//! an EXIF parser is given.

use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::Path;

/// Directories with more entries than this are taken as damage.
const MAX_ENTRIES: usize = 512;
/// Values larger than this are left behind: colour profiles, XMP packets,
/// maker notes and strip tables carry nothing the metadata needs.
const MAX_VALUE_BYTES: u64 = 64 * 1024;
/// Most out-of-line value bytes one block may carry in total.
const BLOCK_BUDGET: u64 = 1024 * 1024;

const TAG_EXIF_IFD: u16 = 0x8769;
const TAG_GPS_IFD: u16 = 0x8825;
const TAG_INTEROP_IFD: u16 = 0xa005;
const TAG_MAKER_NOTE: u16 = 0x927c;
const TYPE_LONG: u16 = 4;
const TYPE_IFD: u16 = 13;

#[derive(Clone, Copy)]
enum ByteOrder {
    Little,
    Big,
}

impl ByteOrder {
    fn u16(self, bytes: &[u8]) -> u16 {
        let pair = [bytes[0], bytes[1]];
        match self {
            Self::Little => u16::from_le_bytes(pair),
            Self::Big => u16::from_be_bytes(pair),
        }
    }

    fn u32(self, bytes: &[u8]) -> u32 {
        let quad = [bytes[0], bytes[1], bytes[2], bytes[3]];
        match self {
            Self::Little => u32::from_le_bytes(quad),
            Self::Big => u32::from_be_bytes(quad),
        }
    }

    fn put_u16(self, value: u16) -> [u8; 2] {
        match self {
            Self::Little => value.to_le_bytes(),
            Self::Big => value.to_be_bytes(),
        }
    }

    fn put_u32(self, value: u32) -> [u8; 4] {
        match self {
            Self::Little => value.to_le_bytes(),
            Self::Big => value.to_be_bytes(),
        }
    }
}

/// Where a directory sits in the tree, which decides the pointer tags it
/// may hold.
#[derive(Clone, Copy)]
enum Role {
    First,
    Exif,
    Leaf,
}

impl Role {
    /// The role of the directory `tag` points to, if it is a pointer here.
    fn child(self, tag: u16) -> Option<Role> {
        match (self, tag) {
            (Self::First, TAG_EXIF_IFD) => Some(Self::Exif),
            (Self::First, TAG_GPS_IFD) | (Self::Exif, TAG_INTEROP_IFD) => Some(Self::Leaf),
            _ => None,
        }
    }
}

struct Entry {
    tag: u16,
    kind: u16,
    count: u32,
    /// The value bytes in file order, wherever the file kept them.
    value: Vec<u8>,
}

#[derive(Default)]
struct Directory {
    entries: Vec<Entry>,
    children: Vec<(u16, Directory)>,
}

/// The metadata directories of the TIFF-structured file at `path` as a small
/// standalone TIFF, or `None` when the file is not one or is too damaged to
/// walk. Reads a few kilobytes however large the file is.
///
/// Besides plain TIFF this accepts the Olympus and Panasonic raw variants,
/// which differ only in the header's magic number; the block written always
/// carries the standard one.
pub(crate) fn metadata_block(path: &Path) -> Option<Vec<u8>> {
    let file = File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    extract(&mut BufReader::new(file), len)
}

fn extract<R: Read + Seek>(reader: &mut R, len: u64) -> Option<Vec<u8>> {
    let mut header = [0u8; 8];
    reader.read_exact(&mut header).ok()?;
    let order = match &header[..2] {
        b"II" => ByteOrder::Little,
        b"MM" => ByteOrder::Big,
        _ => return None,
    };
    // 42 is TIFF; 0x55 is Panasonic RW2; "RO" and "RS" are Olympus ORF.
    if !matches!(order.u16(&header[2..4]), 42 | 0x55 | 0x4f52 | 0x5352) {
        return None;
    }
    let mut walk = Walk {
        reader,
        len,
        order,
        budget: BLOCK_BUDGET,
    };
    let first = walk.directory(u64::from(order.u32(&header[4..8])), Role::First)?;

    let mut block = match order {
        ByteOrder::Little => b"II".to_vec(),
        ByteOrder::Big => b"MM".to_vec(),
    };
    block.extend_from_slice(&order.put_u16(42));
    block.extend_from_slice(&[0; 4]);
    let first_offset = write_directory(order, &first, &mut block);
    block[4..8].copy_from_slice(&order.put_u32(first_offset));
    Some(block)
}

struct Walk<'a, R> {
    reader: &'a mut R,
    len: u64,
    order: ByteOrder,
    budget: u64,
}

impl<R: Read + Seek> Walk<'_, R> {
    fn directory(&mut self, offset: u64, role: Role) -> Option<Directory> {
        if offset < 8 || offset >= self.len {
            return None;
        }
        self.reader.seek(SeekFrom::Start(offset)).ok()?;
        let mut count = [0u8; 2];
        self.reader.read_exact(&mut count).ok()?;
        let count = usize::from(self.order.u16(&count));
        if count == 0 || count > MAX_ENTRIES {
            return None;
        }
        let mut table = vec![0u8; count * 12];
        self.reader.read_exact(&mut table).ok()?;

        let mut directory = Directory::default();
        for raw in table.chunks_exact(12) {
            let tag = self.order.u16(&raw[0..2]);
            let kind = self.order.u16(&raw[2..4]);
            let count = self.order.u32(&raw[4..8]);
            if let Some(child_role) = role.child(tag) {
                if matches!(kind, TYPE_LONG | TYPE_IFD) && count == 1 {
                    let target = u64::from(self.order.u32(&raw[8..12]));
                    if let Some(child) = self.directory(target, child_role) {
                        directory.children.push((tag, child));
                    }
                }
            } else if tag != TAG_MAKER_NOTE
                && let Some(value) = self.value(kind, count, &raw[8..12])
            {
                directory.entries.push(Entry {
                    tag,
                    kind,
                    count,
                    value,
                });
            }
        }
        Some(directory)
    }

    /// The bytes of one entry's value, or `None` when it should be left out:
    /// an unknown type, too large, over budget or pointing outside the file.
    fn value(&mut self, kind: u16, count: u32, field: &[u8]) -> Option<Vec<u8>> {
        let size = unit_size(kind)? * u64::from(count);
        if size <= 4 {
            return Some(field[..size as usize].to_vec());
        }
        let offset = u64::from(self.order.u32(field));
        if size > MAX_VALUE_BYTES || size > self.budget || offset.checked_add(size)? > self.len {
            return None;
        }
        let mut value = vec![0u8; size as usize];
        self.reader.seek(SeekFrom::Start(offset)).ok()?;
        self.reader.read_exact(&mut value).ok()?;
        self.budget -= size;
        Some(value)
    }
}

/// Bytes per value for the twelve classic TIFF types.
fn unit_size(kind: u16) -> Option<u64> {
    match kind {
        1 | 2 | 6 | 7 => Some(1),
        3 | 8 => Some(2),
        4 | 9 | 11 => Some(4),
        5 | 10 | 12 => Some(8),
        _ => None,
    }
}

/// Appends `directory` (children first, then its out-of-line values, then
/// its entry table) and returns the offset of the table.
fn write_directory(order: ByteOrder, directory: &Directory, block: &mut Vec<u8>) -> u32 {
    let mut rows: Vec<(u16, u16, u32, Vec<u8>)> = directory
        .children
        .iter()
        .map(|(tag, child)| {
            let offset = write_directory(order, child, block);
            (*tag, TYPE_LONG, 1, order.put_u32(offset).to_vec())
        })
        .collect();
    rows.extend(
        directory
            .entries
            .iter()
            .map(|entry| (entry.tag, entry.kind, entry.count, entry.value.clone())),
    );
    rows.sort_by_key(|row| row.0);

    let mut table = Vec::with_capacity(2 + rows.len() * 12 + 4);
    table.extend_from_slice(&order.put_u16(rows.len() as u16));
    for (tag, kind, count, value) in rows {
        table.extend_from_slice(&order.put_u16(tag));
        table.extend_from_slice(&order.put_u16(kind));
        table.extend_from_slice(&order.put_u32(count));
        if value.len() <= 4 {
            let mut field = [0u8; 4];
            field[..value.len()].copy_from_slice(&value);
            table.extend_from_slice(&field);
        } else {
            table.extend_from_slice(&order.put_u32(append(block, &value)));
        }
    }
    // No next directory.
    table.extend_from_slice(&[0; 4]);
    append(block, &table)
}

/// Appends `bytes` on a word boundary and returns where they start.
fn append(block: &mut Vec<u8>, bytes: &[u8]) -> u32 {
    if block.len() % 2 == 1 {
        block.push(0);
    }
    // The budget keeps a block far below 4 GiB.
    let offset = block.len() as u32;
    block.extend_from_slice(bytes);
    offset
}

#[cfg(test)]
mod tests;
