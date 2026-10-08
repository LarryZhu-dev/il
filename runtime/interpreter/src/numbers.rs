use il_graph::Opcode;

pub(crate) fn bounds(signed: bool, bits: u8) -> (i128, i128) {
    if signed { (-(1i128 << (bits - 1)), (1i128 << (bits - 1)) - 1) }
    else { (0, (1i128 << bits) - 1) }
}

pub(crate) fn checked(value: i128, signed: bool, bits: u8) -> Result<i128, &'static str> {
    let (minimum, maximum) = bounds(signed, bits);
    if value < minimum || value > maximum { Err("E_INTEGER_OVERFLOW") } else { Ok(value) }
}

pub(crate) fn calculate(opcode: Opcode, left: i128, right: i128, signed: bool, bits: u8) -> Result<i128, &'static str> {
    use Opcode::*;
    let (minimum, _) = bounds(signed, bits);
    let value = match opcode {
        Add => left.checked_add(right),
        Sub => left.checked_sub(right),
        Mul => left.checked_mul(right),
        Div | Rem => {
            if right == 0 { return Err("E_DIVIDE_BY_ZERO"); }
            if signed && left == minimum && right == -1 { return Err("E_INTEGER_OVERFLOW"); }
            Some(if opcode == Div { left / right } else { left % right })
        }
        Shl | Shr => {
            if right < 0 || right >= i128::from(bits) { return Err("E_INVALID_SHIFT"); }
            Some(if opcode == Shl { left << (right as u32) } else { left >> (right as u32) })
        }
        BitAnd => Some(left & right),
        BitOr => Some(left | right),
        BitXor => Some(left ^ right),
        _ => return Err("E_UNSUPPORTED_FEATURE"),
    }.ok_or("E_INTEGER_OVERFLOW")?;
    checked(value, signed, bits)
}

pub(crate) fn complement(value: i128, signed: bool, bits: u8) -> i128 {
    if signed { !value } else { !value & ((1i128 << bits) - 1) }
}
