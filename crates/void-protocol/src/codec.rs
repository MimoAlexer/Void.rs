//! Bounded Java wire primitives. No Minecraft networking library is used.
use std::io::{Read, Write};

use crate::{Error, Result};

pub const MAX_FRAME: usize = 2_097_151;
pub const MAX_DECOMPRESSED: usize = 8 * 1024 * 1024;

pub fn write_varint(value: i32, out: &mut Vec<u8>) {
    let mut value = value as u32;
    loop {
        let byte = (value & 0x7f) as u8;
        value >>= 7;
        out.push(if value == 0 { byte } else { byte | 0x80 });
        if value == 0 {
            break;
        }
    }
}

pub fn peek_varint(data: &[u8]) -> Result<Option<(i32, usize)>> {
    let mut value = 0u32;
    for (index, byte) in data.iter().copied().take(5).enumerate() {
        if index == 4 && byte & 0xf0 != 0 {
            return Err(Error::Malformed("VarInt exceeds 32 bits"));
        }
        value |= ((byte & 0x7f) as u32) << (index * 7);
        if byte & 0x80 == 0 {
            return Ok(Some((value as i32, index + 1)));
        }
    }
    if data.len() >= 5 {
        Err(Error::Malformed("VarInt exceeds five bytes"))
    } else {
        Ok(None)
    }
}

#[derive(Clone)]
pub struct Reader<'a> {
    data: &'a [u8],
    offset: usize,
}
impl<'a> Reader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, offset: 0 }
    }
    pub fn remaining(&self) -> usize {
        self.data.len() - self.offset
    }
    pub fn rest(&self) -> &'a [u8] {
        &self.data[self.offset..]
    }
    pub fn take(&mut self, count: usize) -> Result<&'a [u8]> {
        if count > self.remaining() {
            return Err(Error::Malformed("truncated packet"));
        }
        let bytes = &self.data[self.offset..self.offset + count];
        self.offset += count;
        Ok(bytes)
    }
    pub fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }
    pub fn bool(&mut self) -> Result<bool> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(Error::Malformed("invalid boolean")),
        }
    }
    pub fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_be_bytes(self.take(2)?.try_into().unwrap()))
    }
    pub fn i16(&mut self) -> Result<i16> {
        Ok(i16::from_be_bytes(self.take(2)?.try_into().unwrap()))
    }
    pub fn i32(&mut self) -> Result<i32> {
        Ok(i32::from_be_bytes(self.take(4)?.try_into().unwrap()))
    }
    pub fn i64(&mut self) -> Result<i64> {
        Ok(i64::from_be_bytes(self.take(8)?.try_into().unwrap()))
    }
    pub fn f32(&mut self) -> Result<f32> {
        Ok(f32::from_bits(self.i32()? as u32))
    }
    pub fn f64(&mut self) -> Result<f64> {
        Ok(f64::from_bits(self.i64()? as u64))
    }
    pub fn varint(&mut self) -> Result<i32> {
        let (value, count) =
            peek_varint(self.rest())?.ok_or(Error::Malformed("truncated VarInt"))?;
        self.offset += count;
        Ok(value)
    }
    pub fn varlong(&mut self) -> Result<i64> {
        let mut value = 0u64;
        for shift in 0..10 {
            let byte = self.u8()?;
            if shift == 9 && byte & 0xfe != 0 {
                return Err(Error::Malformed("VarLong exceeds 64 bits"));
            }
            value |= ((byte & 0x7f) as u64) << (shift * 7);
            if byte & 0x80 == 0 {
                return Ok(value as i64);
            }
        }
        Err(Error::Malformed("VarLong exceeds ten bytes"))
    }
    pub fn count(&mut self, max: usize) -> Result<usize> {
        let value = self.varint()?;
        if value < 0 || value as usize > max {
            return Err(Error::Limit("collection count"));
        }
        Ok(value as usize)
    }
    pub fn string(&mut self, max_chars: usize) -> Result<String> {
        let count = self.count(max_chars.saturating_mul(3))?;
        let value = std::str::from_utf8(self.take(count)?)
            .map_err(|_| Error::Malformed("invalid UTF-8"))?;
        if value.encode_utf16().count() > max_chars {
            return Err(Error::Limit("string length"));
        }
        Ok(value.to_owned())
    }
    pub fn uuid(&mut self) -> Result<uuid::Uuid> {
        Ok(uuid::Uuid::from_bytes(self.take(16)?.try_into().unwrap()))
    }
    /// Mojang's LpVec3 compact velocity (26.2): zero marker or six packed bytes
    /// followed by an optional scale continuation VarInt.
    pub fn lp_vec3(&mut self) -> Result<[f64; 3]> {
        let first = self.u8()? as u64;
        if first == 0 {
            return Ok([0.0; 3]);
        }
        let second = self.u8()? as u64;
        let upper = self.i32()? as u32 as u64;
        let packed = (upper << 16) | (second << 8) | first;
        let mut scale = first & 3;
        if first & 4 != 0 {
            scale |= (self.varint()? as u32 as u64) << 2;
        }
        Ok([3, 18, 33].map(|shift| {
            ((((packed >> shift) & 32767).min(32766) as f64) * 2.0 / 32766.0 - 1.0) * scale as f64
        }))
    }
    pub fn finish(self) -> Result<()> {
        if self.remaining() == 0 {
            Ok(())
        } else {
            Err(Error::Malformed("trailing packet bytes"))
        }
    }
}

#[derive(Default)]
pub struct Writer(pub Vec<u8>);
impl Writer {
    pub fn packet(id: i32) -> Self {
        let mut w = Self::default();
        w.varint(id);
        w
    }
    pub fn varint(&mut self, v: i32) {
        write_varint(v, &mut self.0);
    }
    pub fn string(&mut self, v: &str) {
        self.varint(v.len() as i32);
        self.0.extend_from_slice(v.as_bytes());
    }
    pub fn u8(&mut self, v: u8) {
        self.0.push(v);
    }
    pub fn bool(&mut self, v: bool) {
        self.u8(u8::from(v));
    }
    pub fn u16(&mut self, v: u16) {
        self.0.extend_from_slice(&v.to_be_bytes());
    }
    pub fn i32(&mut self, v: i32) {
        self.0.extend_from_slice(&v.to_be_bytes());
    }
    pub fn i64(&mut self, v: i64) {
        self.0.extend_from_slice(&v.to_be_bytes());
    }
    pub fn f32(&mut self, v: f32) {
        self.0.extend_from_slice(&v.to_be_bytes());
    }
    pub fn f64(&mut self, v: f64) {
        self.0.extend_from_slice(&v.to_be_bytes());
    }
    pub fn bytes(&mut self, v: &[u8]) {
        self.0.extend_from_slice(v);
    }
}

pub fn encode_frame(packet: &[u8], threshold: Option<usize>) -> Result<Vec<u8>> {
    if packet.len() > MAX_DECOMPRESSED {
        return Err(Error::Limit("packet size"));
    }
    let mut body = Vec::new();
    match threshold {
        Some(limit) if packet.len() >= limit => {
            write_varint(packet.len() as i32, &mut body);
            let mut encoder =
                flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
            encoder.write_all(packet)?;
            body.extend_from_slice(&encoder.finish()?);
        }
        Some(_) => {
            body.push(0);
            body.extend_from_slice(packet);
        }
        None => body.extend_from_slice(packet),
    }
    if body.len() > MAX_FRAME {
        return Err(Error::Limit("encoded frame size"));
    }
    let mut frame = Vec::with_capacity(body.len() + 3);
    write_varint(body.len() as i32, &mut frame);
    frame.extend_from_slice(&body);
    Ok(frame)
}

/// Incremental decoder keeps partial data across socket read timeouts.
#[derive(Default)]
pub struct FrameDecoder {
    buffer: Vec<u8>,
    pub threshold: Option<usize>,
}
impl FrameDecoder {
    pub fn buffered_len(&self) -> usize {
        self.buffer.len()
    }
    pub fn push(&mut self, bytes: &[u8]) -> Result<()> {
        if self.buffer.len() + bytes.len() > MAX_FRAME + 65536 {
            return Err(Error::Limit("receive buffer"));
        }
        self.buffer.extend_from_slice(bytes);
        Ok(())
    }
    pub fn next_packet(&mut self) -> Result<Option<Vec<u8>>> {
        let Some((size, prefix)) = peek_varint(&self.buffer)? else {
            return Ok(None);
        };
        if size <= 0 || size as usize > MAX_FRAME || prefix > 3 {
            return Err(Error::Limit("frame length"));
        }
        let size = size as usize;
        if self.buffer.len() < prefix + size {
            return Ok(None);
        }
        let body = &self.buffer[prefix..prefix + size];
        let packet = if let Some(threshold) = self.threshold {
            let mut reader = Reader::new(body);
            let inflated_size = reader.count(MAX_DECOMPRESSED)?;
            if inflated_size == 0 {
                if reader.remaining() >= threshold {
                    return Err(Error::Malformed(
                        "uncompressed packet above compression threshold",
                    ));
                }
                reader.rest().to_vec()
            } else {
                if inflated_size < threshold {
                    return Err(Error::Malformed(
                        "compressed packet below compression threshold",
                    ));
                }
                let mut decoder = flate2::read::ZlibDecoder::new(reader.rest());
                let mut output = Vec::with_capacity(inflated_size);
                (&mut decoder)
                    .take(inflated_size as u64 + 1)
                    .read_to_end(&mut output)?;
                if output.len() != inflated_size || decoder.total_in() != reader.remaining() as u64
                {
                    return Err(Error::Malformed("invalid compressed packet length"));
                }
                output
            }
        } else {
            body.to_vec()
        };
        self.buffer.drain(..prefix + size);
        Ok(Some(packet))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn varints_and_fragments() {
        for n in [0, 127, 128, 255, i32::MAX, -1, i32::MIN] {
            let mut bytes = Vec::new();
            write_varint(n, &mut bytes);
            for length in 0..bytes.len() {
                assert_eq!(peek_varint(&bytes[..length]).unwrap(), None);
            }
            assert_eq!(Reader::new(&bytes).varint().unwrap(), n);
        }
        assert!(peek_varint(&[255; 5]).is_err());
    }
    #[test]
    fn compressed_and_fragmented_stream() {
        for threshold in [None, Some(0), Some(256)] {
            let packet = vec![12; 4096];
            let frame = encode_frame(&packet, threshold).unwrap();
            let mut decoder = FrameDecoder {
                threshold,
                ..Default::default()
            };
            for byte in &frame[..frame.len() - 1] {
                decoder.push(&[*byte]).unwrap();
                assert!(decoder.next_packet().unwrap().is_none());
            }
            decoder.push(&frame[frame.len() - 1..]).unwrap();
            assert_eq!(decoder.next_packet().unwrap().unwrap(), packet);
            assert!(decoder.next_packet().unwrap().is_none());
        }
    }
    #[test]
    fn decompression_limits_and_truncation() {
        let mut decoder = FrameDecoder::default();
        decoder.push(&[0xff, 0xff, 0xff, 0x7f]).unwrap();
        assert!(decoder.next_packet().is_err());
        assert!(Reader::new(&[0, 0]).i32().is_err());
        assert!(Reader::new(&[2]).bool().is_err());
    }
}
