//! A Parquet file read from the pieces the child was given (`SEC-22` P1).
//!
//! The child that reads an uploaded file is handed only the byte ranges it
//! needs: the tail of the file, which holds the footer, and — for a preview
//! — the first row group. The file itself may be two gibibytes long, and the
//! child never sees more than those two ranges.
//!
//! [`PartsReader`] is the `parquet` crate's `ChunkReader` over such a file.
//! It answers the ranges it was given and REFUSES every other one: the child
//! was not given those bytes, so it does not have them, and a reader that
//! answered them from somewhere else would be reading a file that is not
//! there. The size it reports is the size of the WHOLE file, because that is
//! what the footer declares and what the row group's offsets are counted
//! from.
//!
//! The two ranges may overlap: a file smaller than the tail is its own tail,
//! and its first row group is inside it. The bytes are the same bytes either
//! way, so the reader answers with the first range that holds what was asked
//! for.

use std::io::{self, Read};

use axum::body::Bytes;
use parquet::errors::{ParquetError, Result as ParquetResult};
use parquet::file::reader::{ChunkReader, Length};

/// One piece of a file: where it starts and how long it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Part {
    /// Where the piece starts in the file.
    pub offset: u64,
    /// How many bytes it has.
    pub length: u64,
}

impl Part {
    /// The piece that ends the file: the last `length` bytes of a file of
    /// `size` bytes. A file shorter than `length` is its own tail.
    #[must_use]
    pub fn tail(size: u64, length: u64) -> Self {
        let length = length.min(size);
        Self {
            offset: size - length,
            length,
        }
    }
}

/// A Parquet file read from the pieces the child was given.
#[derive(Clone)]
pub struct PartsReader {
    /// The size of the WHOLE file: what the footer declares and what every
    /// offset in it is counted from.
    size: u64,
    /// The pieces, in the order they were given.
    parts: Vec<(u64, Bytes)>,
}

impl std::fmt::Debug for PartsReader {
    /// Prints the size and the number of pieces, never the bytes: the bytes
    /// are a file somebody uploaded.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PartsReader")
            .field("size", &self.size)
            .field("parts", &self.parts.len())
            .finish_non_exhaustive()
    }
}

impl PartsReader {
    /// A reader over a file of `size` bytes, of which the pieces are the ones
    /// the child was given.
    #[must_use]
    pub fn new(size: u64, parts: Vec<(u64, Bytes)>) -> Self {
        Self { size, parts }
    }

    /// The piece that holds `start`, if any.
    fn piece_at(&self, start: u64) -> Option<(u64, &Bytes)> {
        self.parts.iter().find_map(|(offset, bytes)| {
            if start >= *offset && start < offset.saturating_add(bytes.len() as u64) {
                Some((*offset, bytes))
            } else {
                None
            }
        })
    }
}

impl Length for PartsReader {
    fn len(&self) -> u64 {
        self.size
    }
}

impl ChunkReader for PartsReader {
    type T = PartRead;

    fn get_read(&self, start: u64) -> ParquetResult<Self::T> {
        let Some((offset, bytes)) = self.piece_at(start) else {
            return Err(ParquetError::General(
                "a range this reader was not given".to_owned(),
            ));
        };
        let from = usize::try_from(start - offset).unwrap_or(usize::MAX);
        Ok(PartRead {
            bytes: bytes.clone(),
            position: from,
        })
    }

    fn get_bytes(&self, start: u64, length: usize) -> ParquetResult<Bytes> {
        let end = start
            .checked_add(u64::try_from(length).unwrap_or(u64::MAX))
            .ok_or_else(|| ParquetError::General("a range past the end of the file".to_owned()))?;
        for (offset, bytes) in &self.parts {
            let piece_end = offset.saturating_add(bytes.len() as u64);
            if start >= *offset && end <= piece_end {
                let from = usize::try_from(start - offset).unwrap_or(usize::MAX);
                let to = from.saturating_add(length).min(bytes.len());
                return Ok(bytes.slice(from..to));
            }
        }
        Err(ParquetError::General(
            "a range this reader was not given".to_owned(),
        ))
    }
}

/// A read over one piece of a file, from a point in it.
///
/// The read ends at the end of the piece: the bytes after it were not given
/// to the child, so there is nothing to read there.
pub struct PartRead {
    bytes: Bytes,
    position: usize,
}

impl Read for PartRead {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() || self.position >= self.bytes.len() {
            return Ok(0);
        }
        let take = buf.len().min(self.bytes.len() - self.position);
        buf[..take].copy_from_slice(&self.bytes[self.position..self.position + take]);
        self.position += take;
        Ok(take)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    /// A file of 100 bytes: "0123456789" repeated.
    fn whole() -> Bytes {
        Bytes::from("0123456789".repeat(10).into_bytes())
    }

    /// A reader over the last 20 bytes of the file.
    fn tail_reader() -> PartsReader {
        let bytes = whole();
        let size = bytes.len() as u64;
        let part = Part::tail(size, 20);
        let piece = bytes.slice(usize::try_from(part.offset).unwrap()..);
        PartsReader::new(size, vec![(part.offset, piece)])
    }

    #[test]
    fn the_reader_reports_the_size_of_the_whole_file() {
        assert_eq!(tail_reader().len(), 100);
    }

    #[test]
    fn a_range_the_reader_was_given_is_answered() {
        let reader = tail_reader();
        let bytes = reader.get_bytes(80, 20).unwrap();
        assert_eq!(bytes.len(), 20);
        assert_eq!(bytes[0], b'0');
        assert_eq!(bytes[19], b'9');
    }

    #[test]
    fn a_range_the_reader_was_not_given_is_refused_and_never_answered() {
        let reader = tail_reader();
        assert!(reader.get_bytes(0, 4).is_err(), "the head was not given");
        assert!(
            reader.get_bytes(40, 20).is_err(),
            "the middle was not given"
        );
        assert!(
            reader.get_bytes(90, 20).is_err(),
            "the range runs past the end"
        );
        assert!(reader.get_read(0).is_err(), "the head was not given");
    }

    #[test]
    fn a_read_serves_the_piece_from_the_point_it_starts_at() {
        let reader = tail_reader();
        let mut read = reader.get_read(85).unwrap();
        let mut out = Vec::new();
        read.read_to_end(&mut out).unwrap();
        assert_eq!(out.len(), 15, "the read ends with the piece");
        assert_eq!(out[0], b'5');
    }

    #[test]
    fn the_tail_of_a_file_shorter_than_the_tail_is_the_whole_file() {
        let part = Part::tail(10, 20);
        assert_eq!(part.offset, 0);
        assert_eq!(part.length, 10);
    }
}
