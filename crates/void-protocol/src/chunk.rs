use crate::{Error, Result, codec::Reader};

#[derive(Debug, Clone)]
pub struct ChunkSection {
    pub y: i32,
    pub non_air_count: u16,
    pub fluid_count: u16,
    /// Index = (local_y * 16 + local_z) * 16 + local_x; vanilla global state IDs.
    pub block_states: Vec<u32>,
    pub biomes: Vec<u32>,
}
#[derive(Debug, Clone)]
pub struct ChunkData {
    pub x: i32,
    pub z: i32,
    pub min_y: i32,
    pub sections: Vec<ChunkSection>,
}

pub fn read_chunk(reader: &mut Reader<'_>, min_y: i32, height: u32) -> Result<ChunkData> {
    if height == 0 || height > 4096 || !height.is_multiple_of(16) {
        return Err(Error::Limit("dimension height"));
    }
    let x = reader.i32()?;
    let z = reader.i32()?;
    for _ in 0..reader.count(16)? {
        reader.varint()?;
        let len = reader.count(4096)?;
        reader.take(len * 8)?;
    }
    let len = reader.count(crate::codec::MAX_DECOMPRESSED)?;
    let mut data = Reader::new(reader.take(len)?);
    let mut sections = Vec::with_capacity(height as usize / 16);
    for index in 0..height as usize / 16 {
        let non_air_count = data.u16()?;
        let fluid_count = data.u16()?;
        if non_air_count > 4096 || fluid_count > 4096 {
            return Err(Error::Malformed("section counts exceed volume"));
        }
        let block_states = palette(&mut data, 4096, 4, 8)?;
        let biomes = palette(&mut data, 64, 1, 3)?;
        sections.push(ChunkSection {
            y: min_y / 16 + index as i32,
            non_air_count,
            fluid_count,
            block_states,
            biomes,
        });
    }
    data.finish()?;
    // Validate all remaining wire fields even while the debug renderer does not apply
    // block-entity models or lighting. A truncated tail is not accepted as a valid chunk.
    for _ in 0..reader.count(65_536)? {
        reader.u8()?;
        reader.i16()?;
        reader.varint()?;
        crate::nbt::read(reader)?;
    }
    let max_sections = height as usize / 16 + 2;
    let mut masks = Vec::with_capacity(4);
    for _ in 0..4 {
        let count = reader.count(max_sections.div_ceil(64))?;
        let mut mask = Vec::with_capacity(count);
        for _ in 0..count {
            mask.push(reader.i64()? as u64);
        }
        masks.push(mask);
    }
    for mask in masks.iter().take(2) {
        let count = reader.count(max_sections)?;
        if count
            != mask
                .iter()
                .map(|word| word.count_ones() as usize)
                .sum::<usize>()
        {
            return Err(Error::Malformed(
                "light-array count does not match section mask",
            ));
        }
        for _ in 0..count {
            if reader.count(2048)? != 2048 {
                return Err(Error::Malformed("invalid light array size"));
            }
            reader.take(2048)?;
        }
    }
    reader.clone().finish()?;
    Ok(ChunkData {
        x,
        z,
        min_y,
        sections,
    })
}

fn palette(
    reader: &mut Reader<'_>,
    entries: usize,
    min_bits: u8,
    max_local_bits: u8,
) -> Result<Vec<u32>> {
    let bits = reader.u8()?;
    if bits == 0 {
        let state = reader.varint()?;
        if state < 0 {
            return Err(Error::Malformed("negative palette ID"));
        }
        return Ok(vec![state as u32; entries]);
    }
    if bits > 32 || bits < min_bits {
        return Err(Error::Malformed("invalid palette bit count"));
    }
    let mut local = Vec::new();
    if bits <= max_local_bits {
        let count = reader.count(1usize << bits)?;
        if count == 0 {
            return Err(Error::Malformed("empty palette"));
        }
        for _ in 0..count {
            let value = reader.varint()?;
            if value < 0 {
                return Err(Error::Malformed("negative palette ID"));
            }
            local.push(value as u32);
        }
    }
    // Since 1.21.5, 26.2 uses the fixed-length long array, WITHOUT a VarInt length.
    // Verified in Mojang FriendlyByteBuf.readFixedSizeLongArray / PalettedContainer.read.
    let per_long = 64 / bits as usize;
    let longs = entries.div_ceil(per_long);
    let mask = (1u64 << bits) - 1;
    let mut result = Vec::with_capacity(entries);
    for _ in 0..longs {
        let packed = reader.i64()? as u64;
        for offset in 0..per_long {
            if result.len() == entries {
                break;
            }
            let id = ((packed >> (offset * bits as usize)) & mask) as u32;
            result.push(if local.is_empty() {
                id
            } else {
                *local
                    .get(id as usize)
                    .ok_or(Error::Malformed("palette index outside palette"))?
            });
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn single_value_has_no_array_length() {
        let mut r = Reader::new(&[0, 42, 99]);
        assert_eq!(palette(&mut r, 4096, 4, 8).unwrap(), vec![42; 4096]);
        assert_eq!(r.u8().unwrap(), 99);
    }
    #[test]
    fn padded_local_palette() {
        let mut bytes = vec![5, 2, 0, 7];
        for _ in 0..4096usize.div_ceil(12) {
            bytes.extend_from_slice(&0x0084_2108_4210_8421u64.to_be_bytes());
        }
        let values = palette(&mut Reader::new(&bytes), 4096, 4, 8).unwrap();
        assert_eq!(values.len(), 4096);
        assert!(values.iter().all(|v| *v == 7));
    }
    #[test]
    fn rejects_bad_palette_index() {
        let mut bytes = vec![4, 1, 0];
        bytes.extend_from_slice(&u64::MAX.to_be_bytes());
        assert!(palette(&mut Reader::new(&bytes), 4096, 4, 8).is_err());
    }
}
