//! Bounded duplex frames. Each frame is one tag, a four-byte big-endian length,
//! then exact payload bytes. Control records carry the admitted request ID.
use std::io::Read;

pub const MAX_FRAME: usize = 64 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Kind {
    Input = 1,
    InputClosed = 2,
    Stdout = 3,
    Stderr = 4,
    Started = 5,
    Outcome = 6,
    Finalize = 7,
}
impl Kind {
    pub fn decode(tag: u8, length: usize) -> anyhow::Result<Self> {
        let kind = match tag {
            1 => Self::Input,
            2 => Self::InputClosed,
            3 => Self::Stdout,
            4 => Self::Stderr,
            5 => Self::Started,
            6 => Self::Outcome,
            7 => Self::Finalize,
            _ => anyhow::bail!("unknown guest stream frame"),
        };
        if length > MAX_FRAME || (length == 0) != (kind == Self::InputClosed) {
            anyhow::bail!("invalid guest stream frame length");
        }
        Ok(kind)
    }
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Started {
    pub version: u32,
    pub id: String,
}

pub fn encode(kind: Kind, bytes: &[u8]) -> anyhow::Result<Vec<u8>> {
    Kind::decode(kind as u8, bytes.len())?;
    let mut frame = Vec::with_capacity(5 + bytes.len());
    frame.push(kind as u8);
    frame.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
    frame.extend_from_slice(bytes);
    Ok(frame)
}

/// Retain partial reads across EAGAIN without buffering a second frame. A
/// hostile length is rejected before allocating its payload.
#[derive(Default)]
pub struct Decoder {
    header: [u8; 5],
    header_used: usize,
    body: Vec<u8>,
    body_used: usize,
}
impl Decoder {
    pub fn read(&mut self, reader: &mut impl Read) -> anyhow::Result<Option<(Kind, Vec<u8>)>> {
        while self.header_used < 5 {
            match reader.read(&mut self.header[self.header_used..]) {
                Ok(0) => anyhow::bail!("guest stream closed before a complete frame"),
                Ok(n) => self.header_used += n,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => return Ok(None),
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e.into()),
            }
        }
        let length = u32::from_be_bytes(self.header[1..].try_into()?) as usize;
        let kind = Kind::decode(self.header[0], length)?;
        self.body.resize(length, 0);
        while self.body_used < length {
            match reader.read(&mut self.body[self.body_used..]) {
                Ok(0) => anyhow::bail!("guest stream payload was truncated"),
                Ok(n) => self.body_used += n,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => return Ok(None),
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e.into()),
            }
        }
        self.header_used = 0;
        self.body_used = 0;
        Ok(Some((kind, std::mem::take(&mut self.body))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn partial_binary_frames_preserve_data_and_backpressure() {
        struct Fragmented<'a> {
            bytes: &'a [u8],
            wait: bool,
        }
        impl Read for Fragmented<'_> {
            fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
                self.wait = !self.wait;
                if self.wait {
                    return Err(std::io::ErrorKind::WouldBlock.into());
                }
                self.bytes.read(&mut out[..1])
            }
        }
        let data = b" \0\xff\nexact ";
        let mut bytes = encode(Kind::Input, data).unwrap();
        bytes.extend(encode(Kind::InputClosed, b"").unwrap());
        let mut reader = Fragmented {
            bytes: &bytes,
            wait: false,
        };
        let mut decoder = Decoder::default();
        for expected in [
            (Kind::Input, data.as_slice()),
            (Kind::InputClosed, b"".as_slice()),
        ] {
            let frame = loop {
                if let Some(frame) = decoder.read(&mut reader).unwrap() {
                    break frame;
                }
            };
            assert_eq!(frame, (expected.0, expected.1.to_vec()));
        }
        assert!(decoder.read(&mut reader).is_ok()); // one final EAGAIN
        assert!(decoder.read(&mut reader).is_err());
    }
    #[test]
    fn malformed_lengths_tags_and_truncation_are_rejected() {
        for bytes in [
            vec![1, 255, 255, 255, 255],
            vec![8, 0, 0, 0, 1],
            vec![2, 0, 0, 0, 1],
            vec![3, 0, 0, 0, 0],
            vec![1, 0, 0, 0, 2, 42],
        ] {
            assert!(Decoder::default().read(&mut bytes.as_slice()).is_err());
        }
    }
}
