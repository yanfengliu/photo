//! TIFF image file directories, little-endian: the structure shared by an
//! EXIF block and a TIFF file.
//!
//! A stream is built front to back. Child directories (the EXIF and GPS
//! directories) are written before the directory that points to them, so
//! every offset is known when it is written and only the header's pointer
//! to the first directory is filled in at the end.

use std::collections::BTreeMap;

/// A field value with its TIFF type.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Value {
    /// `BYTE`.
    Byte(Vec<u8>),
    /// `ASCII`: written as the text's UTF-8 bytes and a terminating NUL;
    /// NULs inside the text are dropped.
    Ascii(String),
    /// `SHORT`.
    Short(Vec<u16>),
    /// `LONG`.
    Long(Vec<u32>),
    /// `RATIONAL`: numerator and denominator.
    Rational(Vec<(u32, u32)>),
    /// `SRATIONAL`.
    SRational(Vec<(i32, i32)>),
    /// `UNDEFINED`.
    Undefined(Vec<u8>),
}

impl Value {
    fn type_code(&self) -> u16 {
        match self {
            Self::Byte(_) => 1,
            Self::Ascii(_) => 2,
            Self::Short(_) => 3,
            Self::Long(_) => 4,
            Self::Rational(_) => 5,
            Self::Undefined(_) => 7,
            Self::SRational(_) => 10,
        }
    }

    /// The value's bytes and its count in units of its type.
    fn encode(&self) -> (Vec<u8>, usize) {
        match self {
            Self::Byte(v) | Self::Undefined(v) => (v.clone(), v.len()),
            Self::Ascii(text) => {
                let bytes: Vec<u8> = text.bytes().filter(|&b| b != 0).chain([0]).collect();
                let count = bytes.len();
                (bytes, count)
            }
            Self::Short(v) => (v.iter().flat_map(|x| x.to_le_bytes()).collect(), v.len()),
            Self::Long(v) => (v.iter().flat_map(|x| x.to_le_bytes()).collect(), v.len()),
            Self::Rational(v) => (
                v.iter()
                    .flat_map(|(n, d)| n.to_le_bytes().into_iter().chain(d.to_le_bytes()))
                    .collect(),
                v.len(),
            ),
            Self::SRational(v) => (
                v.iter()
                    .flat_map(|(n, d)| n.to_le_bytes().into_iter().chain(d.to_le_bytes()))
                    .collect(),
                v.len(),
            ),
        }
    }
}

/// One directory under construction, kept in ascending tag order as TIFF
/// requires.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Ifd {
    entries: BTreeMap<u16, Value>,
}

impl Ifd {
    /// An empty directory.
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Sets `tag`, replacing an earlier value.
    pub(crate) fn set(&mut self, tag: u16, value: Value) {
        self.entries.insert(tag, value);
    }

    /// True when no tag is set.
    pub(crate) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Every entry of `other` added to this directory.
    pub(crate) fn merge(&mut self, other: &Ifd) {
        for (&tag, value) in &other.entries {
            self.entries.insert(tag, value.clone());
        }
    }
}

/// A little-endian TIFF stream under construction. Offsets count from its
/// first byte, as both a TIFF file and an EXIF block do.
#[derive(Clone, Debug)]
pub(crate) struct TiffStream {
    bytes: Vec<u8>,
}

impl TiffStream {
    /// A stream holding only a header whose pointer to the first directory
    /// [`Self::finish`] fills in.
    pub(crate) fn new() -> Self {
        Self {
            bytes: vec![b'I', b'I', 42, 0, 0, 0, 0, 0],
        }
    }

    /// The stream's length so far.
    pub(crate) fn len(&self) -> usize {
        self.bytes.len()
    }

    /// Writes `ifd` (its long values first, then its entry table, on word
    /// boundaries) and returns the table's offset.
    pub(crate) fn directory(&mut self, ifd: &Ifd) -> u32 {
        let mut table = Vec::with_capacity(2 + ifd.entries.len() * 12 + 4);
        table.extend_from_slice(&(ifd.entries.len() as u16).to_le_bytes());
        for (&tag, value) in &ifd.entries {
            let (payload, count) = value.encode();
            table.extend_from_slice(&tag.to_le_bytes());
            table.extend_from_slice(&value.type_code().to_le_bytes());
            table.extend_from_slice(&(count as u32).to_le_bytes());
            if payload.len() <= 4 {
                let mut inline = [0u8; 4];
                inline[..payload.len()].copy_from_slice(&payload);
                table.extend_from_slice(&inline);
            } else {
                let offset = self.append(&payload);
                table.extend_from_slice(&offset.to_le_bytes());
            }
        }
        // Directories here are reached through pointer tags, never chained.
        table.extend_from_slice(&0u32.to_le_bytes());
        self.append(&table)
    }

    /// Points the header at `first` and returns the bytes.
    pub(crate) fn finish(mut self, first: u32) -> Vec<u8> {
        self.bytes[4..8].copy_from_slice(&first.to_le_bytes());
        self.bytes
    }

    fn append(&mut self, payload: &[u8]) -> u32 {
        if self.bytes.len() % 2 == 1 {
            self.bytes.push(0);
        }
        let offset = self.bytes.len() as u32;
        self.bytes.extend_from_slice(payload);
        offset
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_values_are_inline_and_large_ones_follow_on_word_boundaries() {
        let mut ifd = Ifd::new();
        ifd.set(0x0112, Value::Short(vec![1]));
        ifd.set(0x010f, Value::Ascii("Make\0r".into()));
        ifd.set(0x829a, Value::Rational(vec![(1, 250)]));
        let mut stream = TiffStream::new();
        let first = stream.directory(&ifd);
        let bytes = stream.finish(first);
        assert_eq!(&bytes[..4], b"II*\0");
        let at = first as usize;
        assert_eq!(first % 2, 0);
        assert_eq!(u16::from_le_bytes([bytes[at], bytes[at + 1]]), 3);
        // Entries ascend by tag; the ASCII value lost its NUL and gained one
        // at the end.
        let entry = |index: usize| &bytes[at + 2 + index * 12..at + 14 + index * 12];
        assert_eq!(&entry(0)[..2], &0x010fu16.to_le_bytes());
        assert_eq!(u32::from_le_bytes(entry(0)[4..8].try_into().unwrap()), 6);
        let text_at = u32::from_le_bytes(entry(0)[8..12].try_into().unwrap()) as usize;
        assert_eq!(&bytes[text_at..text_at + 6], b"Maker\0");
        assert_eq!(&entry(1)[8..10], &1u16.to_le_bytes());
    }
}
