use std::io::Cursor;

use exif::{In, Reader, Tag, Value};

use super::*;

/// A TIFF assembled by hand, in either byte order.
struct Builder {
    order: ByteOrder,
    bytes: Vec<u8>,
}

/// One directory entry: tag, type, count and the raw value bytes.
type Row = (u16, u16, u32, Vec<u8>);

impl Builder {
    fn new(order: ByteOrder, magic: u16) -> Self {
        let mut bytes = match order {
            ByteOrder::Little => b"II".to_vec(),
            ByteOrder::Big => b"MM".to_vec(),
        };
        bytes.extend_from_slice(&order.put_u16(magic));
        bytes.extend_from_slice(&[0; 4]);
        Self { order, bytes }
    }

    fn short(&self, tag: u16, value: u16) -> Row {
        (tag, 3, 1, self.order.put_u16(value).to_vec())
    }

    fn long(&self, tag: u16, value: u32) -> Row {
        (tag, TYPE_LONG, 1, self.order.put_u32(value).to_vec())
    }

    fn ascii(&self, tag: u16, text: &str) -> Row {
        let mut value = text.as_bytes().to_vec();
        value.push(0);
        (tag, 2, value.len() as u32, value)
    }

    fn rational(&self, tag: u16, numerator: u32, denominator: u32) -> Row {
        let mut value = self.order.put_u32(numerator).to_vec();
        value.extend_from_slice(&self.order.put_u32(denominator));
        (tag, 5, 1, value)
    }

    fn undefined(&self, tag: u16, value: Vec<u8>) -> Row {
        (tag, 7, value.len() as u32, value)
    }

    /// Writes a directory at the current end and returns its offset.
    fn directory(&mut self, mut rows: Vec<Row>) -> u32 {
        rows.sort_by_key(|row| row.0);
        let mut table = self.order.put_u16(rows.len() as u16).to_vec();
        for (tag, kind, count, value) in rows {
            table.extend_from_slice(&self.order.put_u16(tag));
            table.extend_from_slice(&self.order.put_u16(kind));
            table.extend_from_slice(&self.order.put_u32(count));
            if value.len() <= 4 {
                let mut field = [0u8; 4];
                field[..value.len()].copy_from_slice(&value);
                table.extend_from_slice(&field);
            } else {
                let offset = self.bytes.len() as u32;
                self.bytes.extend_from_slice(&value);
                table.extend_from_slice(&self.order.put_u32(offset));
            }
        }
        table.extend_from_slice(&[0; 4]);
        let offset = self.bytes.len() as u32;
        self.bytes.extend_from_slice(&table);
        offset
    }

    fn finish(mut self, first: u32) -> Vec<u8> {
        let pointer = self.order.put_u32(first);
        self.bytes[4..8].copy_from_slice(&pointer);
        self.bytes
    }
}

/// A file with every kind of directory and a megabyte of things that must
/// be left behind.
fn rich_file(order: ByteOrder, magic: u16) -> Vec<u8> {
    let mut tiff = Builder::new(order, magic);
    let pixels = vec![0x5a; 400_000];
    let strip = tiff.bytes.len() as u32;
    tiff.bytes.extend_from_slice(&pixels);

    let interop = tiff.directory(vec![tiff.ascii(0x0001, "R03")]);
    let exif = tiff.directory(vec![
        tiff.rational(0x829a, 1, 250),
        tiff.short(0x8827, 800),
        tiff.ascii(0x9003, "2024:03:15 07:42:10"),
        tiff.undefined(TAG_MAKER_NOTE, vec![7; 5000]),
        tiff.long(TAG_INTEROP_IFD, interop),
        tiff.ascii(0xa434, "Testkit 35mm F2"),
    ]);
    let gps = tiff.directory(vec![tiff.ascii(0x0001, "N"), tiff.ascii(0x0003, "W")]);
    let first = tiff.directory(vec![
        tiff.long(0x0100, 640),
        tiff.ascii(0x010f, "Photo Testkit"),
        tiff.long(0x0111, strip),
        tiff.short(0x0112, 6),
        tiff.long(0x0117, pixels.len() as u32),
        tiff.undefined(0x8773, vec![9; 300_000]),
        tiff.long(TAG_EXIF_IFD, exif),
        tiff.long(TAG_GPS_IFD, gps),
    ]);
    tiff.finish(first)
}

fn block(file: &[u8]) -> Option<Vec<u8>> {
    extract(&mut Cursor::new(file), file.len() as u64)
}

fn text(parsed: &exif::Exif, tag: Tag) -> String {
    match &parsed
        .get_field(tag, In::PRIMARY)
        .expect("field present")
        .value
    {
        Value::Ascii(strings) => String::from_utf8_lossy(&strings[0]).into_owned(),
        other => panic!("{tag} is not text: {other:?}"),
    }
}

#[test]
fn the_block_is_small_and_holds_every_metadata_directory() {
    for order in [ByteOrder::Little, ByteOrder::Big] {
        let file = rich_file(order, 42);
        assert!(file.len() > 700_000);
        let block = block(&file).expect("a TIFF");
        assert!(block.len() < 600, "block is {} bytes", block.len());

        let parsed = Reader::new().read_raw(block).expect("a clean EXIF block");
        assert_eq!(text(&parsed, Tag::Make), "Photo Testkit");
        assert_eq!(text(&parsed, Tag::DateTimeOriginal), "2024:03:15 07:42:10");
        assert_eq!(text(&parsed, Tag::LensModel), "Testkit 35mm F2");
        assert_eq!(text(&parsed, Tag::GPSLatitudeRef), "N");
        assert_eq!(text(&parsed, Tag::GPSLongitudeRef), "W");
        assert_eq!(text(&parsed, Tag::InteroperabilityIndex), "R03");
        let value = |tag| {
            parsed
                .get_field(tag, In::PRIMARY)
                .unwrap()
                .value
                .get_uint(0)
        };
        assert_eq!(value(Tag::Orientation), Some(6));
        assert_eq!(value(Tag::PhotographicSensitivity), Some(800));
        assert_eq!(value(Tag::ImageWidth), Some(640));
        match &parsed
            .get_field(Tag::ExposureTime, In::PRIMARY)
            .unwrap()
            .value
        {
            Value::Rational(v) => assert_eq!((v[0].num, v[0].denom), (1, 250)),
            other => panic!("{other:?}"),
        }
        // The big things stayed behind.
        assert!(parsed.get_field(Tag::MakerNote, In::PRIMARY).is_none());
        assert!(parsed.fields().all(|field| field.tag.number() != 0x8773));
    }
}

#[test]
fn raw_variants_of_the_header_are_accepted_and_normalised() {
    for magic in [0x55, 0x4f52, 0x5352] {
        let block = block(&rich_file(ByteOrder::Little, magic)).expect("a TIFF variant");
        assert_eq!(&block[..4], b"II\x2a\0");
        assert!(Reader::new().read_raw(block).is_ok());
    }
}

#[test]
fn files_that_are_not_tiff_give_nothing() {
    assert!(block(b"").is_none());
    assert!(block(b"II\x2a").is_none());
    assert!(block(b"\xff\xd8\xff\xe0 not a tiff at all").is_none());
    // BigTIFF.
    assert!(block(&rich_file(ByteOrder::Little, 43)).is_none());
    // The first directory lies outside the file, or inside the header.
    let file = rich_file(ByteOrder::Big, 42);
    for bad_offset in [0u32, 4, file.len() as u32, u32::MAX] {
        let mut broken = file.clone();
        broken[4..8].copy_from_slice(&bad_offset.to_be_bytes());
        assert!(block(&broken).is_none(), "offset {bad_offset}");
    }
}

#[test]
fn truncation_at_any_point_is_survived() {
    let file = rich_file(ByteOrder::Little, 42);
    let step = file.len() / 400;
    for length in (0..file.len())
        .step_by(step)
        .chain(file.len() - 300..file.len())
    {
        if let Some(block) = extract(&mut Cursor::new(&file[..length]), length as u64) {
            let mut reader = Reader::new();
            reader.continue_on_error(true);
            let _ = reader.read_raw(block);
        }
    }
}

#[test]
fn a_value_pointing_outside_the_file_is_dropped_alone() {
    let mut tiff = Builder::new(ByteOrder::Little, 42);
    let first = tiff.directory(vec![
        tiff.ascii(0x010f, "Photo Testkit"),
        tiff.ascii(0x0110, "A model name longer than four bytes"),
        tiff.short(0x0112, 3),
    ]);
    let mut file = tiff.finish(first);
    // The model is the second entry; send its value offset past the end.
    let offset_field = first as usize + 2 + 12 + 8;
    file[offset_field..offset_field + 4].copy_from_slice(&0x7fff_fff0u32.to_le_bytes());

    let parsed = Reader::new().read_raw(block(&file).unwrap()).unwrap();
    assert_eq!(text(&parsed, Tag::Make), "Photo Testkit");
    assert!(parsed.get_field(Tag::Model, In::PRIMARY).is_none());
    let orientation = parsed.get_field(Tag::Orientation, In::PRIMARY).unwrap();
    assert_eq!(orientation.value.get_uint(0), Some(3));
}

#[test]
fn a_pointer_loop_ends() {
    // The EXIF pointer leads back to the first directory itself.
    let mut tiff = Builder::new(ByteOrder::Little, 42);
    let first = tiff.bytes.len() as u32 + "Photo Testkit\0".len() as u32;
    let written = tiff.directory(vec![
        tiff.ascii(0x010f, "Photo Testkit"),
        tiff.long(TAG_EXIF_IFD, first),
    ]);
    assert_eq!(written, first);
    let block = block(&tiff.finish(first)).expect("still a TIFF");
    let mut reader = Reader::new();
    reader.continue_on_error(true);
    let parsed = reader
        .read_raw(block)
        .or_else(|error| error.distill_partial_result(|_| ()))
        .unwrap();
    assert_eq!(text(&parsed, Tag::Make), "Photo Testkit");
}

#[test]
fn absurd_directories_are_refused() {
    let mut empty = Builder::new(ByteOrder::Little, 42);
    let first = empty.directory(Vec::new());
    assert!(block(&empty.finish(first)).is_none(), "no entries");

    let mut crowded = Builder::new(ByteOrder::Little, 42);
    let rows = (0..=MAX_ENTRIES as u16)
        .map(|tag| crowded.short(tag, 1))
        .collect();
    let first = crowded.directory(rows);
    assert!(block(&crowded.finish(first)).is_none(), "too many entries");
}

#[test]
fn the_total_budget_caps_what_a_hostile_file_can_make_us_copy() {
    // Forty values just under the per-value limit: more than the budget.
    let mut tiff = Builder::new(ByteOrder::Little, 42);
    let rows = (0..40u16)
        .map(|index| tiff.undefined(0x9000 + index, vec![1; MAX_VALUE_BYTES as usize]))
        .collect();
    let first = tiff.directory(rows);
    let block = block(&tiff.finish(first)).unwrap();
    assert!(
        block.len() as u64 <= BLOCK_BUDGET + 1024,
        "block is {} bytes",
        block.len()
    );
    assert!(block.len() as u64 >= BLOCK_BUDGET - MAX_VALUE_BYTES);
}

#[test]
fn the_block_agrees_with_a_full_parse_of_a_real_dng() {
    let dir = tempfile::tempdir().unwrap();
    let path = photo_testkit::write_dng(dir.path(), "full.dng", 64, 48);
    let whole = Reader::new()
        .read_raw(std::fs::read(&path).unwrap())
        .unwrap();
    let sparse = Reader::new()
        .read_raw(metadata_block(&path).unwrap())
        .unwrap();
    // Every field of the sparse block is in the file with the same value, and
    // nothing from the three metadata directories went missing.
    for field in sparse.fields() {
        let reference = whole
            .get_field(field.tag, field.ifd_num)
            .expect("field exists in the file");
        let (got, want) = (
            field.display_value().to_string(),
            reference.display_value().to_string(),
        );
        assert_eq!(got, want, "{}", field.tag);
    }
    assert_eq!(sparse.fields().len(), whole.fields().len());
    assert!(
        sparse.fields().len() >= 40,
        "only {} fields",
        sparse.fields().len()
    );
    assert_eq!(text(&sparse, Tag::Model), "Synthetic One");
}
