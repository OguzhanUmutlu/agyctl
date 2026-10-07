pub fn decode_varint(data: &[u8], mut offset: usize) -> Result<(u64, usize), ()> {
    let mut res: u64 = 0;
    let mut shift = 0;
    loop {
        if offset >= data.len() {
            return Err(());
        }
        let b = data[offset];
        offset += 1;
        res |= ((b & 0x7F) as u64) << shift;
        shift += 7;
        if (b & 0x80) == 0 {
            break;
        }
    }
    Ok((res, offset))
}

pub fn encode_varint(mut val: u64) -> Vec<u8> {
    let mut out = Vec::new();
    while val > 0x7F {
        out.push(((val & 0x7F) as u8) | 0x80);
        val >>= 7;
    }
    out.push((val & 0x7F) as u8);
    out
}

pub fn replace_bytes(data: &[u8], from: &[u8], to: &[u8]) -> Vec<u8> {
    if from.is_empty() {
        return data.to_vec();
    }
    let mut result = Vec::new();
    let mut i = 0;
    while i < data.len() {
        if i + from.len() <= data.len() && &data[i..i + from.len()] == from {
            result.extend_from_slice(to);
            i += from.len();
        } else {
            result.push(data[i]);
            i += 1;
        }
    }
    result
}

pub fn rewrite_proto(data: &[u8], old_prefix: &[u8], new_prefix: &[u8]) -> Vec<u8> {
    if data.is_empty() || !data.windows(old_prefix.len()).any(|w| w == old_prefix) {
        return data.to_vec();
    }

    let mut offset = 0;
    let mut out = Vec::new();

    while offset < data.len() {
        let start_offset = offset;
        let (key, new_offset) = match decode_varint(data, offset) {
            Ok(v) => v,
            Err(_) => {
                out.extend_from_slice(&data[start_offset..]);
                break;
            }
        };
        offset = new_offset;

        let wire_type = key & 7;
        let field_num = key >> 3;

        match wire_type {
            0 => match decode_varint(data, offset) {
                Ok((_, next_offset)) => {
                    offset = next_offset;
                    out.extend_from_slice(&data[start_offset..offset]);
                }
                Err(_) => {
                    out.extend_from_slice(&data[start_offset..]);
                    break;
                }
            },
            2 => {
                let (length, next_offset) = match decode_varint(data, offset) {
                    Ok(v) => v,
                    Err(_) => {
                        out.extend_from_slice(&data[start_offset..]);
                        break;
                    }
                };
                offset = next_offset;
                let length = length as usize;
                if offset + length > data.len() {
                    out.extend_from_slice(&data[start_offset..]);
                    break;
                }

                let val = &data[offset..offset + length];
                offset += length;

                if val.windows(old_prefix.len()).any(|w| w == old_prefix) {
                    let rewritten = rewrite_proto(val, old_prefix, new_prefix);
                    let final_bytes = if rewritten == val {
                        replace_bytes(val, old_prefix, new_prefix)
                    } else {
                        rewritten
                    };
                    out.extend(encode_varint((field_num << 3) | 2));
                    out.extend(encode_varint(final_bytes.len() as u64));
                    out.extend(final_bytes);
                } else {
                    out.extend(encode_varint((field_num << 3) | 2));
                    out.extend(encode_varint(length as u64));
                    out.extend_from_slice(val);
                }
            }
            1 => {
                if offset + 8 > data.len() {
                    out.extend_from_slice(&data[start_offset..]);
                    break;
                }
                out.extend_from_slice(&data[start_offset..offset + 8]);
                offset += 8;
            }
            5 => {
                if offset + 4 > data.len() {
                    out.extend_from_slice(&data[start_offset..]);
                    break;
                }
                out.extend_from_slice(&data[start_offset..offset + 4]);
                offset += 4;
            }
            _ => {
                out.extend_from_slice(&data[start_offset..]);
                break;
            }
        }
    }

    out
}
