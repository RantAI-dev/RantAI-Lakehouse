//! A Parquet file read by pieces from the object store, so the API never holds
//! the file (`DATA-17` T2.2, plan AD2: the API reads only bounded samples).
//!
//! [`RangeReader`] is the `parquet` crate's `ChunkReader` over a stored object:
//! the crate asks for the byte ranges it needs (the last eight bytes, the
//! footer, a column chunk) and each is one ranged `GET`. Nothing here decodes a
//! value; `upload_parquet` does, and bounds what the footer may declare before
//! any range is asked for.
//!
//! # Sync reader, async store
//!
//! The `parquet` reader is synchronous and the store is not, so every read is
//! `Handle::block_on` of the store's future. That is only legal off the async
//! worker threads, which is why [`RangeReader`] is built in an async handler and
//! used inside `spawn_blocking` (`routes::uploads::blocking`).
//!
//! # Bounds
//!
//! - One ranged request is at most [`MAX_FETCH_BYTES`] (the store's own
//!   `MAX_RANGE_BYTES`); a larger ask is an error and the file is unreadable.
//! - A range that is not inside the object is an error, asked of the store
//!   never.
//! - [`BlockRead`] (what `get_read` returns) fetches 1 MiB at a time and only
//!   as the reader consumes it.
//!
//! What the footer may declare (its length, the place of every column chunk,
//! the size of every row group) is checked in `upload_parquet::open`.

use std::io::{Read, Seek, SeekFrom};
use std::sync::Arc;

use axum::body::Bytes;
use parquet::errors::{ParquetError, Result as ParquetResult};
use parquet::file::reader::{ChunkReader, Length};
use tokio::runtime::Handle;

use crate::upload_store::{MAX_RANGE_BYTES, UploadStore, UploadStoreError};

/// The most one ranged request may ask for.
pub const MAX_FETCH_BYTES: u64 = MAX_RANGE_BYTES;

/// What [`BlockRead`] fetches at a time.
const BLOCK_BYTES: u64 = 1024 * 1024;

/// A stored object as a `ChunkReader`. Cheap to clone (the store and the key
/// are shared).
#[derive(Clone)]
pub struct RangeReader {
    store: UploadStore,
    key: Arc<str>,
    size: u64,
    handle: Handle,
}

/// `Debug` prints the size only: the key and the store are not for a log line.
impl std::fmt::Debug for RangeReader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RangeReader")
            .field("size", &self.size)
            .finish_non_exhaustive()
    }
}

impl RangeReader {
    /// Open the object at `key`: one `HEAD` for its size. Call it from an async
    /// handler; read through it only on a blocking thread.
    ///
    /// # Errors
    ///
    /// [`UploadStoreError::Unavailable`] when the object is missing or the
    /// store fails.
    pub async fn open(store: &UploadStore, key: &str) -> Result<Self, UploadStoreError> {
        let size = store
            .object_size(key)
            .await?
            .ok_or(UploadStoreError::Unavailable("not found"))?;
        Ok(Self {
            store: store.clone(),
            key: Arc::from(key),
            size,
            handle: Handle::current(),
        })
    }

    fn fetch(&self, start: u64, len: u64) -> ParquetResult<Bytes> {
        let inside = start.checked_add(len).is_some_and(|end| end <= self.size);
        if !inside || len > MAX_FETCH_BYTES {
            return Err(ParquetError::General(
                "a range outside the file or past the fetch bound".to_owned(),
            ));
        }
        let bytes = self
            .handle
            .block_on(self.store.read_range(&self.key, start, len))
            .map_err(|_err| ParquetError::General("the store could not be read".to_owned()))?;
        if u64::try_from(bytes.len()).ok() != Some(len) {
            return Err(ParquetError::General(
                "the store returned a different length".to_owned(),
            ));
        }
        Ok(bytes)
    }
}

impl Length for RangeReader {
    fn len(&self) -> u64 {
        self.size
    }
}

impl ChunkReader for RangeReader {
    type T = BlockRead;

    fn get_read(&self, start: u64) -> ParquetResult<Self::T> {
        Ok(BlockRead {
            source: self.clone(),
            next: start.min(self.size),
            block: Bytes::new(),
        })
    }

    fn get_bytes(&self, start: u64, length: usize) -> ParquetResult<Bytes> {
        self.fetch(start, u64::try_from(length).unwrap_or(u64::MAX))
    }
}

impl RangeReader {
    /// A sequential `Read` of the whole object, one block at a time (a gzip stream).
    #[must_use]
    pub fn sequential(&self) -> BlockRead {
        BlockRead {
            source: self.clone(),
            next: 0,
            block: Bytes::new(),
        }
    }

    /// A `Read + Seek` over the object, fetching one block at a time around the position
    /// (a zip archive: its central directory is at the end, its files are in the middle).
    #[must_use]
    pub fn seekable(&self) -> RangeSeek {
        RangeSeek {
            source: self.clone(),
            position: 0,
            block_start: 0,
            block: Bytes::new(),
        }
    }
}

/// A `Read + Seek` over a stored object (`DATA-17` slice 4): one ranged request per block of
/// [`BLOCK_BYTES`], the last one kept, nothing else held.
pub struct RangeSeek {
    source: RangeReader,
    position: u64,
    block_start: u64,
    block: Bytes,
}

impl Read for RangeSeek {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if buf.is_empty() || self.position >= self.source.size {
            return Ok(0);
        }
        let held = self.position >= self.block_start
            && self.position < self.block_start + self.block.len() as u64;
        if !held {
            let len = BLOCK_BYTES.min(self.source.size - self.position);
            self.block = self
                .source
                .fetch(self.position, len)
                .map_err(|err| std::io::Error::other(err.to_string()))?;
            self.block_start = self.position;
        }
        let from = usize::try_from(self.position - self.block_start).unwrap_or(0);
        let take = buf.len().min(self.block.len() - from);
        buf[..take].copy_from_slice(&self.block[from..from + take]);
        self.position += take as u64;
        Ok(take)
    }
}

impl Seek for RangeSeek {
    fn seek(&mut self, to: SeekFrom) -> std::io::Result<u64> {
        let target = match to {
            SeekFrom::Start(at) => Some(at),
            SeekFrom::End(delta) => self.source.size.checked_add_signed(delta),
            SeekFrom::Current(delta) => self.position.checked_add_signed(delta),
        };
        let at = target.ok_or_else(|| std::io::Error::other("a seek before the start"))?;
        self.position = at;
        Ok(at)
    }
}

/// A `Read` over the object from an offset, fetching [`BLOCK_BYTES`] at a time
/// as it is consumed.
pub struct BlockRead {
    source: RangeReader,
    next: u64,
    block: Bytes,
}

impl Read for BlockRead {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.block.is_empty() {
            let len = BLOCK_BYTES.min(self.source.size.saturating_sub(self.next));
            if len == 0 || buf.is_empty() {
                return Ok(0);
            }
            self.block = self
                .source
                .fetch(self.next, len)
                .map_err(|err| std::io::Error::other(err.to_string()))?;
            self.next += len;
        }
        let take = buf.len().min(self.block.len());
        buf[..take].copy_from_slice(&self.block[..take]);
        self.block = self.block.slice(take..);
        Ok(take)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use crate::upload_parquet::{convert, inspect};

    const ORDERS: &[u8] = include_bytes!("../../../../ops/fixtures/parquet/orders.parquet");
    const KEY: &str = "uploads/t/up-1.parquet";

    async fn stored(bytes: &'static [u8]) -> (UploadStore, RangeReader) {
        let store = UploadStore::in_memory();
        store.put(KEY, Bytes::from_static(bytes)).await.unwrap();
        let reader = RangeReader::open(&store, KEY).await.unwrap();
        (store, reader)
    }

    /// The same answers as the bytes give, read through the store by ranges.
    #[tokio::test]
    async fn a_stored_file_reads_like_its_bytes() {
        let (_store, reader) = stored(ORDERS).await;
        let whole = Bytes::from_static(ORDERS);
        let ranged = tokio::task::spawn_blocking(move || {
            (
                inspect(&reader).unwrap(),
                convert(&reader, Some(2)).unwrap(),
            )
        })
        .await
        .unwrap();
        assert_eq!(ranged.0, inspect(&whole).unwrap());
        assert_eq!(ranged.1, convert(&whole, Some(2)).unwrap());
    }

    #[tokio::test]
    async fn a_range_outside_the_file_or_past_the_fetch_bound_is_an_error_never_a_request() {
        let (_store, reader) = stored(ORDERS).await;
        let size = reader.len();
        let results = tokio::task::spawn_blocking(move || {
            (
                reader.get_bytes(size, 1).is_err(),
                reader.get_bytes(size - 1, 2).is_err(),
                reader.get_bytes(u64::MAX, 1).is_err(),
                reader.get_bytes(0, 4).is_ok(),
                reader.get_bytes(size - 4, 4).is_ok(),
            )
        })
        .await
        .unwrap();
        assert_eq!(results, (true, true, true, true, true));
    }

    #[tokio::test]
    async fn the_block_reader_serves_the_file_from_an_offset_to_its_end() {
        let (_store, reader) = stored(ORDERS).await;
        let got = tokio::task::spawn_blocking(move || {
            let mut read = reader.get_read(10).unwrap();
            let mut out = Vec::new();
            read.read_to_end(&mut out).unwrap();
            let mut past = reader.get_read(u64::MAX).unwrap();
            let mut nothing = Vec::new();
            past.read_to_end(&mut nothing).unwrap();
            (out, nothing)
        })
        .await
        .unwrap();
        assert_eq!(got.0, ORDERS[10..]);
        assert!(got.1.is_empty());
    }

    #[tokio::test]
    async fn the_seekable_reader_reads_any_range_by_blocks_and_never_past_the_end() {
        let (_store, reader) = stored(ORDERS).await;
        let size = usize::try_from(reader.len()).unwrap();
        let got = tokio::task::spawn_blocking(move || {
            let mut seek = reader.seekable();
            let mut tail = [0u8; 8];
            seek.seek(SeekFrom::End(-8)).unwrap();
            seek.read_exact(&mut tail).unwrap();
            seek.seek(SeekFrom::Start(3)).unwrap();
            let mut middle = [0u8; 5];
            seek.read_exact(&mut middle).unwrap();
            let mut rest = Vec::new();
            seek.seek(SeekFrom::Current(-5)).unwrap();
            seek.read_to_end(&mut rest).unwrap();
            let mut sequential = Vec::new();
            reader.sequential().read_to_end(&mut sequential).unwrap();
            (
                tail,
                middle,
                rest,
                sequential,
                seek.seek(SeekFrom::Current(-1_000_000)).is_err(),
            )
        })
        .await
        .unwrap();
        assert_eq!(got.0, ORDERS[size - 8..]);
        assert_eq!(got.1, ORDERS[3..8]);
        assert_eq!(got.2, ORDERS[3..]);
        assert_eq!(got.3, ORDERS);
        assert!(got.4, "a seek before the start is an error");
    }

    #[tokio::test]
    async fn a_missing_object_is_not_found() {
        let store = UploadStore::in_memory();
        let err = RangeReader::open(&store, KEY).await.unwrap_err();
        assert_eq!(err, UploadStoreError::Unavailable("not found"));
    }
}
