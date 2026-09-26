//! Network NBT (unnamed root); depth, allocation and element counts are bounded.
use crate::{Error, Result, codec::Reader};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq)]
pub enum Nbt {
    End,
    Byte(i8),
    Short(i16),
    Int(i32),
    Long(i64),
    Float(f32),
    Double(f64),
    Bytes(Vec<u8>),
    String(String),
    List(Vec<Nbt>),
    Compound(BTreeMap<String, Nbt>),
    Ints(Vec<i32>),
    Longs(Vec<i64>),
}
impl Nbt {
    pub fn integer(&self) -> Option<i32> {
        match self {
            Self::Int(v) => Some(*v),
            Self::Byte(v) => Some(*v as i32),
            Self::Short(v) => Some(*v as i32),
            _ => None,
        }
    }
    pub fn get(&self, key: &str) -> Option<&Nbt> {
        match self {
            Self::Compound(v) => v.get(key),
            _ => None,
        }
    }
    /// Plain-text view, retaining translation keys when localization is unavailable.
    pub fn text(&self) -> String {
        match self {
            Self::String(s) => s.clone(),
            Self::List(list) => list.iter().map(Self::text).collect(),
            Self::Compound(fields) => {
                let mut text = fields
                    .get("text")
                    .or_else(|| fields.get("translate"))
                    .map(Self::text)
                    .unwrap_or_default();
                if let Some(extra) = fields.get("extra") {
                    text.push_str(&extra.text());
                }
                if let Some(args) = fields.get("with") {
                    text.push(' ');
                    text.push_str(&args.text());
                }
                text
            }
            _ => String::new(),
        }
    }
}
pub fn read(reader: &mut Reader<'_>) -> Result<Nbt> {
    let tag = reader.u8()?;
    payload(reader, tag, 0, &mut 262_144)
}
fn count(reader: &mut Reader<'_>, budget: &mut usize) -> Result<usize> {
    let count = reader.i32()?;
    if count < 0 || count as usize > *budget {
        return Err(Error::Limit("NBT collection"));
    }
    *budget -= count as usize;
    Ok(count as usize)
}
fn string(reader: &mut Reader<'_>) -> Result<String> {
    let n = reader.u16()? as usize;
    let bytes = reader.take(n)?;
    // NBT uses Java modified UTF-8. Decode it through UTF-16, including surrogate pairs.
    let mut units = Vec::with_capacity(n);
    let mut i = 0;
    while i < n {
        let b = bytes[i];
        if b & 0x80 == 0 {
            units.push(b as u16);
            i += 1;
        } else if b & 0xe0 == 0xc0 && i + 1 < n && bytes[i + 1] & 0xc0 == 0x80 {
            units.push((((b & 31) as u16) << 6) | ((bytes[i + 1] & 63) as u16));
            i += 2;
        } else if b & 0xf0 == 0xe0
            && i + 2 < n
            && bytes[i + 1] & 0xc0 == 0x80
            && bytes[i + 2] & 0xc0 == 0x80
        {
            units.push(
                (((b & 15) as u16) << 12)
                    | (((bytes[i + 1] & 63) as u16) << 6)
                    | ((bytes[i + 2] & 63) as u16),
            );
            i += 3;
        } else {
            return Err(Error::Malformed("invalid NBT modified UTF-8"));
        }
    }
    String::from_utf16(&units).map_err(|_| Error::Malformed("invalid NBT surrogate pair"))
}
fn payload(reader: &mut Reader<'_>, tag: u8, depth: usize, budget: &mut usize) -> Result<Nbt> {
    if depth > 64 || *budget == 0 {
        return Err(Error::Limit("NBT depth or nodes"));
    }
    *budget -= 1;
    Ok(match tag {
        0 => Nbt::End,
        1 => Nbt::Byte(reader.u8()? as i8),
        2 => Nbt::Short(reader.i16()?),
        3 => Nbt::Int(reader.i32()?),
        4 => Nbt::Long(reader.i64()?),
        5 => Nbt::Float(reader.f32()?),
        6 => Nbt::Double(reader.f64()?),
        7 => {
            let n = count(reader, budget)?;
            Nbt::Bytes(reader.take(n)?.to_vec())
        }
        8 => Nbt::String(string(reader)?),
        9 => {
            let element = reader.u8()?;
            let n = count(reader, budget)?;
            if element == 0 && n > 0 {
                return Err(Error::Malformed("nonempty NBT End list"));
            }
            let mut list = Vec::with_capacity(n);
            for _ in 0..n {
                list.push(payload(reader, element, depth + 1, budget)?);
            }
            Nbt::List(list)
        }
        10 => {
            let mut map = BTreeMap::new();
            loop {
                let child = reader.u8()?;
                if child == 0 {
                    break;
                }
                let key = string(reader)?;
                let value = payload(reader, child, depth + 1, budget)?;
                map.insert(key, value);
            }
            Nbt::Compound(map)
        }
        11 => {
            let n = count(reader, budget)?;
            let mut values = Vec::with_capacity(n);
            for _ in 0..n {
                values.push(reader.i32()?);
            }
            Nbt::Ints(values)
        }
        12 => {
            let n = count(reader, budget)?;
            let mut values = Vec::with_capacity(n);
            for _ in 0..n {
                values.push(reader.i64()?);
            }
            Nbt::Longs(values)
        }
        _ => return Err(Error::Malformed("unknown NBT type")),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn network_compound() {
        let b = [
            10, 3, 0, 5, b'm', b'i', b'n', b'_', b'y', 255, 255, 255, 192, 0,
        ];
        let n = read(&mut Reader::new(&b)).unwrap();
        assert_eq!(n.get("min_y").unwrap().integer(), Some(-64));
    }
    #[test]
    fn rejects_depth_and_negative_count() {
        assert!(read(&mut Reader::new(&[7, 255, 255, 255, 255])).is_err());
        let mut b = vec![10];
        for _ in 0..70 {
            b.extend_from_slice(&[10, 0, 0]);
        }
        assert!(read(&mut Reader::new(&b)).is_err());
    }
    #[test]
    fn modified_utf8_emoji() {
        let b = [8, 0, 6, 0xed, 0xa0, 0xbd, 0xed, 0xb8, 0x80];
        assert_eq!(read(&mut Reader::new(&b)).unwrap().text(), "😀");
    }
}
