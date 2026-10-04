// v1.1.0.3 - Validate an ICO and preserve every size in a native Windows resource.
use std::io;

pub const APPLICATION_ICON_RESOURCE: u16 = 101;

fn invalid() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "invalid or incomplete Windows ICO",
    )
}

fn u16_at(raw: &[u8], at: usize) -> io::Result<u16> {
    Ok(u16::from_le_bytes(
        raw.get(at..at + 2).ok_or_else(invalid)?.try_into().unwrap(),
    ))
}

fn u32_at(raw: &[u8], at: usize) -> io::Result<u32> {
    Ok(u32::from_le_bytes(
        raw.get(at..at + 4).ok_or_else(invalid)?.try_into().unwrap(),
    ))
}

fn align(bytes: &mut Vec<u8>) {
    while !bytes.len().is_multiple_of(4) {
        bytes.push(0);
    }
}

fn resource(bytes: &mut Vec<u8>, kind: u16, name: u16, payload: &[u8]) {
    align(bytes);
    bytes.extend((payload.len() as u32).to_le_bytes());
    bytes.extend(32_u32.to_le_bytes());
    for word in [0xffff, kind, 0xffff, name] {
        bytes.extend(word.to_le_bytes());
    }
    bytes.extend(0_u32.to_le_bytes());
    bytes.extend(0x1030_u16.to_le_bytes());
    bytes.extend(0x0409_u16.to_le_bytes());
    bytes.extend(0_u32.to_le_bytes());
    bytes.extend(0_u32.to_le_bytes());
    bytes.extend(payload);
    align(bytes);
}

pub fn icon_resource(ico: &[u8]) -> io::Result<Vec<u8>> {
    if u16_at(ico, 0)? != 0 || u16_at(ico, 2)? != 1 {
        return Err(invalid());
    }
    let count = u16_at(ico, 4)? as usize;
    if count == 0 || count > 64 {
        return Err(invalid());
    }
    let directory_end = 6 + count * 16;
    if ico.len() < directory_end {
        return Err(invalid());
    }
    let mut group = ico[..6].to_vec();
    let mut images = Vec::with_capacity(count);
    let mut spans = Vec::with_capacity(count);
    for index in 0..count {
        let entry = 6 + index * 16;
        let size = u32_at(ico, entry + 8)? as usize;
        let start = u32_at(ico, entry + 12)? as usize;
        let end = start.checked_add(size).ok_or_else(invalid)?;
        if size == 0
            || start < directory_end
            || end > ico.len()
            || spans
                .iter()
                .any(|&(other_start, other_end)| start < other_end && end > other_start)
        {
            return Err(invalid());
        }
        spans.push((start, end));
        group.extend_from_slice(&ico[entry..entry + 12]);
        group.extend(((index + 1) as u16).to_le_bytes());
        images.push(&ico[start..end]);
    }
    let mut output = vec![0_u8; 32];
    output[4..8].copy_from_slice(&32_u32.to_le_bytes());
    output[8..10].copy_from_slice(&0xffff_u16.to_le_bytes());
    output[12..14].copy_from_slice(&0xffff_u16.to_le_bytes());
    for (index, image) in images.iter().enumerate() {
        resource(&mut output, 3, (index + 1) as u16, image);
    }
    resource(&mut output, 14, APPLICATION_ICON_RESOURCE, &group);
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> Vec<u8> {
        let mut bytes = vec![0, 0, 1, 0, 2, 0];
        for (dimension, offset) in [(16_u8, 38_u32), (32, 42)] {
            bytes.extend([dimension, dimension, 0, 0, 1, 0, 32, 0]);
            bytes.extend(4_u32.to_le_bytes());
            bytes.extend(offset.to_le_bytes());
        }
        bytes.extend([1, 2, 3, 4, 5, 6, 7, 8]);
        bytes
    }
    #[test]
    fn resource_preserves_complete_images_and_native_group_identity() {
        let output = icon_resource(&fixture()).unwrap();
        assert_eq!(u32_at(&output, 4).unwrap(), 32);
        assert_eq!(u16_at(&output, 42).unwrap(), 3);
        assert_eq!(&output[64..68], &[1, 2, 3, 4]);
        assert_eq!(u16_at(&output, 114).unwrap(), 14);
        assert_eq!(u16_at(&output, 118).unwrap(), APPLICATION_ICON_RESOURCE);
        assert_eq!(u16_at(&output, 140).unwrap(), 2);
        assert_eq!(output.len() % 4, 0);
    }
    #[test]
    fn missing_overlapping_or_escaped_images_fail_before_embedding() {
        let mut bytes = fixture();
        assert!(icon_resource(&bytes[..bytes.len() - 1]).is_err());
        bytes[34..38].copy_from_slice(&38_u32.to_le_bytes());
        assert!(icon_resource(&bytes).is_err());
        bytes[34..38].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(icon_resource(&bytes).is_err());
        bytes[4..6].copy_from_slice(&0_u16.to_le_bytes());
        assert!(icon_resource(&bytes).is_err());
    }
}
