// v2.22.27 - Bound allocations while reading files and untrusted response bodies.
use std::fs::File;
use std::io::{self, Read};
use std::path::Path;

pub const STATE_TEXT_LIMIT: usize = 64 * 1024 * 1024;
pub const CONFIG_TEXT_LIMIT: usize = 1024 * 1024;

pub fn read_limited(reader: impl Read, limit: usize) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    reader
        .take((limit as u64).saturating_add(1))
        .read_to_end(&mut bytes)?;
    if bytes.len() > limit {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("input exceeds {limit} byte limit"),
        ));
    }
    Ok(bytes)
}

pub fn read_text_file(path: impl AsRef<Path>, limit: usize) -> io::Result<String> {
    let file = File::open(path)?;
    // Check the opened handle, not a separate path lookup. The bounded read
    // also handles a file growing after metadata was sampled.
    if file.metadata()?.len() > limit as u64 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "text file exceeds configured byte limit",
        ));
    }
    String::from_utf8(read_limited(file, limit)?)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn boundary_is_inclusive_and_stream_growth_is_bounded() {
        assert_eq!(read_limited(&b"abcd"[..], 4).unwrap(), b"abcd");
        assert_eq!(
            read_limited(&b"abcde"[..], 4).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        assert!(read_limited(io::repeat(0), 4096).is_err());
        assert_eq!(read_limited(io::empty(), 0).unwrap().len(), 0);
    }
}
