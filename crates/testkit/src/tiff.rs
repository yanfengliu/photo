//! A minimal little-endian TIFF writer: just enough structure for an EXIF
//! block, a baseline RGB TIFF and a DNG.
//!
//! The stream is built front to back. Pixel data and child directories are
//! appended first, so that by the time a directory is written every offset it
//! needs is already known and nothing has to be patched except the header.

use std::collections::BTreeMap;

/// A TIFF field value with its on-disk type.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Value {
    Byte(Vec<u8>),
    /// Written with a terminating NUL, which the count includes.
    Ascii(String),
    Short(Vec<u16>),
    Long(Vec<u32>),
    Rational(Vec<(u32, u32)>),
    SRational(Vec<(i32, i32)>),
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

    fn count(&self) -> usize {
        match self {
            Self::Byte(v) | Self::Undefined(v) => v.len(),
            Self::Ascii(text) => text.len() + 1,
            Self::Short(v) => v.len(),
            Self::Long(v) => v.len(),
            Self::Rational(v) => v.len(),
            Self::SRational(v) => v.len(),
        }
    }

    fn payload(&self) -> Vec<u8> {
        match self {
            Self::Byte(v) | Self::Undefined(v) => v.clone(),
            Self::Ascii(text) => text.bytes().chain([0]).collect(),
            Self::Short(v) => v.iter().flat_map(|x| x.to_le_bytes()).collect(),
            Self::Long(v) => v.iter().flat_map(|x| x.to_le_bytes()).collect(),
            Self::Rational(v) => v
                .iter()
                .flat_map(|(n, d)| n.to_le_bytes().into_iter().chain(d.to_le_bytes()))
                .collect(),
            Self::SRational(v) => v
                .iter()
                .flat_map(|(n, d)| n.to_le_bytes().into_iter().chain(d.to_le_bytes()))
                .collect(),
        }
    }
}

/// One image file directory under construction. Entries are kept in ascending
/// tag order, as the TIFF specification requires.
#[derive(Clone, Debug, Default)]
pub(crate) struct Ifd {
    entries: BTreeMap<u16, Value>,
}

impl Ifd {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Sets `tag`, replacing any earlier value.
    pub(crate) fn set(&mut self, tag: u16, value: Value) -> &mut Self {
        self.entries.insert(tag, value);
        self
    }

    pub(crate) fn short(&mut self, tag: u16, value: u16) -> &mut Self {
        self.set(tag, Value::Short(vec![value]))
    }

    pub(crate) fn long(&mut self, tag: u16, value: u32) -> &mut Self {
        self.set(tag, Value::Long(vec![value]))
    }

    pub(crate) fn ascii(&mut self, tag: u16, text: &str) -> &mut Self {
        self.set(tag, Value::Ascii(text.to_string()))
    }
}

/// A growing TIFF byte stream. Offsets are relative to the first byte of the
/// header, which is how both a TIFF file and an EXIF block count them.
pub(crate) struct TiffWriter {
    bytes: Vec<u8>,
}

impl TiffWriter {
    /// Starts a little-endian stream with a header whose first-directory
    /// offset is filled in by [`Self::finish`].
    pub(crate) fn new() -> Self {
        Self {
            bytes: vec![b'I', b'I', 42, 0, 0, 0, 0, 0],
        }
    }

    /// Appends raw bytes (pixel strips, an embedded JPEG) on a word boundary
    /// and returns their offset.
    pub(crate) fn data(&mut self, payload: &[u8]) -> u32 {
        if self.bytes.len() % 2 == 1 {
            self.bytes.push(0);
        }
        let offset = self.position();
        self.bytes.extend_from_slice(payload);
        offset
    }

    /// Writes a directory (out-of-line values first, then the entry table)
    /// and returns the offset of the table.
    pub(crate) fn ifd(&mut self, ifd: &Ifd) -> u32 {
        let mut table = Vec::with_capacity(2 + ifd.entries.len() * 12 + 4);
        let entry_count = u16::try_from(ifd.entries.len()).expect("too many TIFF entries");
        table.extend_from_slice(&entry_count.to_le_bytes());
        for (&tag, value) in &ifd.entries {
            let payload = value.payload();
            let count = u32::try_from(value.count()).expect("TIFF value too long");
            table.extend_from_slice(&tag.to_le_bytes());
            table.extend_from_slice(&value.type_code().to_le_bytes());
            table.extend_from_slice(&count.to_le_bytes());
            if payload.len() <= 4 {
                let mut inline = [0u8; 4];
                inline[..payload.len()].copy_from_slice(&payload);
                table.extend_from_slice(&inline);
            } else {
                let offset = self.data(&payload);
                table.extend_from_slice(&offset.to_le_bytes());
            }
        }
        // No next directory: every directory here is reached by a pointer tag.
        table.extend_from_slice(&0u32.to_le_bytes());
        self.data(&table)
    }

    /// Points the header at `first_ifd` and returns the finished stream.
    pub(crate) fn finish(mut self, first_ifd: u32) -> Vec<u8> {
        self.bytes[4..8].copy_from_slice(&first_ifd.to_le_bytes());
        self.bytes
    }

    fn position(&self) -> u32 {
        u32::try_from(self.bytes.len()).expect("fixture TIFF exceeds 4 GiB")
    }
}

#[cfg(test)]
pub(crate) mod read {
    //! A tiny reader used only to check what the writer produced.

    use std::collections::BTreeMap;

    pub(crate) struct Entry {
        pub(crate) kind: u16,
        pub(crate) count: u32,
        /// The value bytes, wherever they were stored.
        pub(crate) bytes: Vec<u8>,
    }

    impl Entry {
        pub(crate) fn u32_at(&self, index: usize) -> u32 {
            match self.kind {
                3 => u32::from(u16::from_le_bytes([
                    self.bytes[index * 2],
                    self.bytes[index * 2 + 1],
                ])),
                4 => u32::from_le_bytes(self.bytes[index * 4..index * 4 + 4].try_into().unwrap()),
                1 | 7 => u32::from(self.bytes[index]),
                other => panic!("entry type {other} is not an unsigned integer"),
            }
        }

        pub(crate) fn text(&self) -> String {
            String::from_utf8_lossy(&self.bytes)
                .trim_end_matches('\0')
                .to_string()
        }
    }

    pub(crate) fn first_ifd(tiff: &[u8]) -> u32 {
        assert_eq!(&tiff[..4], b"II\x2a\0", "little-endian TIFF header");
        u32::from_le_bytes(tiff[4..8].try_into().unwrap())
    }

    pub(crate) fn ifd(tiff: &[u8], offset: u32) -> BTreeMap<u16, Entry> {
        let at = offset as usize;
        let count = u16::from_le_bytes([tiff[at], tiff[at + 1]]) as usize;
        let mut entries = BTreeMap::new();
        let mut previous_tag = None;
        for index in 0..count {
            let entry = &tiff[at + 2 + index * 12..at + 14 + index * 12];
            let tag = u16::from_le_bytes([entry[0], entry[1]]);
            assert!(previous_tag < Some(tag), "tags must ascend");
            previous_tag = Some(tag);
            let kind = u16::from_le_bytes([entry[2], entry[3]]);
            let value_count = u32::from_le_bytes(entry[4..8].try_into().unwrap());
            let unit = match kind {
                1 | 2 | 7 => 1,
                3 => 2,
                4 => 4,
                5 | 10 => 8,
                other => panic!("unexpected TIFF type {other}"),
            };
            let size = unit * value_count as usize;
            let bytes = if size <= 4 {
                entry[8..8 + size].to_vec()
            } else {
                let start = u32::from_le_bytes(entry[8..12].try_into().unwrap()) as usize;
                tiff[start..start + size].to_vec()
            };
            entries.insert(
                tag,
                Entry {
                    kind,
                    count: value_count,
                    bytes,
                },
            );
        }
        let next = &tiff[at + 2 + count * 12..at + 6 + count * 12];
        assert_eq!(next, [0, 0, 0, 0], "directories are not chained");
        entries
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_values_are_stored_inline_and_long_ones_out_of_line() {
        let mut writer = TiffWriter::new();
        let mut ifd = Ifd::new();
        ifd.short(0x0112, 6)
            .ascii(0x010f, "Photo Testkit")
            .set(0x829a, Value::Rational(vec![(1, 250)]));
        let offset = writer.ifd(&ifd);
        let tiff = writer.finish(offset);

        let entries = read::ifd(&tiff, read::first_ifd(&tiff));
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[&0x0112].u32_at(0), 6);
        assert_eq!(entries[&0x010f].text(), "Photo Testkit");
        assert_eq!(entries[&0x010f].count, 14, "count includes the NUL");
        assert_eq!(entries[&0x829a].bytes, [1, 0, 0, 0, 250, 0, 0, 0]);
    }

    #[test]
    fn data_blocks_start_on_word_boundaries() {
        let mut writer = TiffWriter::new();
        assert_eq!(writer.data(&[1, 2, 3]), 8);
        assert_eq!(writer.data(&[4]), 12);
        let mut ifd = Ifd::new();
        ifd.long(0x0111, 8);
        let offset = writer.ifd(&ifd);
        assert_eq!(offset % 2, 0);
    }
}
