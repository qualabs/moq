use bytes::{Buf, Bytes, BytesMut, BufMut};
use mp4_atom::{Any, Atom, DecodeAtom, DecodeMaybe, Header, Mdat};

pub enum AtomEvent {
    Atom(Any, usize), // Atom and its total size
    Mdat(Bytes, usize), // Payload and header size
}

pub struct AtomReader {
    buffer: BytesMut,
}

impl AtomReader {
    pub fn new() -> Self {
        Self {
            buffer: BytesMut::new(),
        }
    }

    pub fn push<B: Buf>(&mut self, buf: &mut B) {
        if buf.has_remaining() {
            self.buffer.put(buf);
        }
    }

    pub fn next(&mut self) -> anyhow::Result<Option<AtomEvent>> {
        loop {
            if self.buffer.is_empty() {
                return Ok(None);
            }

            // Create a cursor over the current buffer to peek the header
            let mut cursor = std::io::Cursor::new(&self.buffer[..]);

            // Try to read the atom header
            let header = match Header::decode_maybe(&mut cursor)? {
                Some(header) => header,
                None => return Ok(None), // Need more data for header
            };

            let header_size = cursor.position() as usize;

            // mp4-atom Header.size is the payload size (excluding header)
            let payload_size = match header.size {
                Some(s) => s,
                None => {
                    return Err(anyhow::anyhow!("indefinite atom size not supported in stream"));
                }
            };

            let total_size = header_size + payload_size;

            if self.buffer.len() < total_size {
                return Ok(None); // Need more data
            }

            // We have the full atom.
            let mut atom_bytes = self.buffer.split_to(total_size).freeze();

            // Advance past header to get payload
            atom_bytes.advance(header_size);
            let payload = atom_bytes;

            if header.kind == Mdat::KIND {
                return Ok(Some(AtomEvent::Mdat(payload, header_size)));
            } else {
                let mut payload_cursor = std::io::Cursor::new(payload);
                let atom = Any::decode_atom(&header, &mut payload_cursor)?;
                return Ok(Some(AtomEvent::Atom(atom, total_size)));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::BufMut;
    use mp4_atom::{FourCC, Moof};

    #[test]
    fn test_reader_partial_header() {
        let mut reader = AtomReader::new();
        let mut buf = BytesMut::new();
        buf.put_u32(16); // Size
        reader.push(&mut buf);
        assert!(reader.next().unwrap().is_none());

        let mut buf = BytesMut::new();
        buf.put_slice(b"moof"); // Kind
        reader.push(&mut buf);
        // Header complete (8 bytes), but body missing (8 bytes, as size is 16 = 8 header + 8 body)
        assert!(reader.next().unwrap().is_none());
    }

    #[test]
    fn test_reader_partial_body() {
        let mut reader = AtomReader::new();
        let mut buf = BytesMut::new();
        // Header: size 16 (8+8), kind "free"
        buf.put_u32(16);
        buf.put_slice(b"free");
        buf.put_u32(0xDEADBEEF); // 4 bytes of body
        reader.push(&mut buf);

        assert!(reader.next().unwrap().is_none());

        let mut buf = BytesMut::new();
        buf.put_u32(0xCAFEBABE); // remaining 4 bytes
        reader.push(&mut buf);

        let event = reader.next().unwrap().expect("should return atom");
        match event {
            AtomEvent::Atom(Any::Free(_), size) => {
                assert_eq!(size, 16);
            },
            _ => panic!("Expected Free atom"),
        }
    }

    #[test]
    fn test_reader_mdat_zero_copy() {
        let mut reader = AtomReader::new();
        let mut buf = BytesMut::new();
        // Header: size 20 (8+12), kind "mdat"
        buf.put_u32(20);
        buf.put_slice(b"mdat");
        let payload = b"hello world!";
        buf.put_slice(payload);
        reader.push(&mut buf);

        let event = reader.next().unwrap().expect("should return mdat");
        match event {
            AtomEvent::Mdat(data, header_size) => {
                assert_eq!(header_size, 8);
                assert_eq!(data, &payload[..]);
            },
            _ => panic!("Expected Mdat"),
        }
    }

    #[test]
    fn test_reader_multiple_atoms() {
        let mut reader = AtomReader::new();
        let mut buf = BytesMut::new();

        // Atom 1: Free (8 header + 0 body = 8 bytes)
        buf.put_u32(8);
        buf.put_slice(b"free");

        // Atom 2: Free (8 header + 4 body = 12 bytes)
        buf.put_u32(12);
        buf.put_slice(b"free");
        buf.put_slice(b"1234");

        reader.push(&mut buf);

        // First atom
        let event = reader.next().unwrap().expect("should return first atom");
        match event {
            AtomEvent::Atom(Any::Free(_), 8) => {},
            _ => panic!("Expected first Free atom"),
        }

        // Second atom
        let event = reader.next().unwrap().expect("should return second atom");
        match event {
            AtomEvent::Atom(Any::Free(_), 12) => {},
            _ => panic!("Expected second Free atom"),
        }

        assert!(reader.next().unwrap().is_none());
    }
}
