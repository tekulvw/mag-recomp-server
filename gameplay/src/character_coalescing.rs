//! Opt-in coalescing of queued Character deltas into one update per recipient.
//!
//! A server update carries, for each object type its mask selects, a list of
//! objects: a presence bit, the 16-bit object index and that object's field
//! presence bits with their data, ended by a zero bit. Server deltas that touch
//! a single Character (bot movement, aim and ammunition, landing damage) are
//! queued as separate updates today, each repeating the 147/148-bit header.
//! When enabled, the drain joins consecutive queued Character-only updates of
//! one recipient into one update: one header, every row, one terminator.
//!
//! Only rows this module can measure exactly are joined: a single Character
//! row whose present fields all have a fixed width the server already encodes
//! (`FIELD_BITS`), followed by the list terminator and zero padding to the
//! byte. Anything else, and rows of the same Character twice, end a run, so
//! the recipient still sees every field in the order it was queued. Rows join
//! only when their headers carry the same timestamp (the same world tick).
use crate::replication::{BitReader, BitWriter, MAX_UPDATE_BYTES, ObjectUpdate, UpdateHeader};
use crate::wire_profile::WireProfile;

const CHARACTER_MASK: u32 = 1 << 3;
const FIELD_COUNT: usize = 22;

/// Data bits of each Character field the server publishes in deltas, by field
/// index; `None` for fields a delta row never carries (they end a run).
const FIELD_BITS: [Option<usize>; FIELD_COUNT] = {
    let mut bits = [None; FIELD_COUNT];
    bits[4] = Some(32); // state: packed state/stance byte and three user bytes
    bits[5] = Some(49); // position: x 16, sign 1, y 16, z 16
    bits[6] = Some(8); // pitch
    bits[7] = Some(10); // yaw
    bits[8] = Some(120); // damage: 8 + 16 + 32 + 32 + 32
    bits[11] = Some(24); // primary item
    bits[12] = Some(24); // secondary item
    bits[13] = Some(1); // primary equipped
    bits
};

/// One queued single-Character update, measured.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct CharacterRow {
    pub object: u16,
    pub timestamp_ms: u32,
    /// Bit offset of the row (its list-presence bit) in the queued wire.
    start: usize,
    /// Row length in bits, from its presence bit through its last field.
    bits: usize,
}

impl CharacterRow {
    /// Measure `wire` as exactly one server Character row. `None` when the
    /// update has another shape or a field this module does not measure.
    pub(crate) fn measure(wire: &[u8], profile: WireProfile) -> Option<Self> {
        let update = ObjectUpdate::decode_with_profile(wire, profile).ok()?;
        let header = update.header;
        if header.object_type_mask != CHARACTER_MASK
            || header.nak_sequence != 0
            || header.client_index != 0
        {
            return None;
        }
        let mut bits = BitReader::with_profile(wire, profile);
        let start = profile.header_bits();
        bits.position = start;
        if bits.read(1).ok()? != 1 {
            return None;
        }
        let object = bits.read(16).ok()? as u16;
        for width in FIELD_BITS {
            if bits.read(1).ok()? == 1 {
                skip(&mut bits, width?).ok()?;
            }
        }
        let end = bits.position;
        // The list ends here, and only zero padding to the byte may follow.
        if bits.read(1).ok()? != 0 {
            return None;
        }
        let padding = wire.len() * 8 - bits.position;
        if padding >= 8 || bits.read(padding).ok()? != 0 {
            return None;
        }
        Some(Self {
            object,
            timestamp_ms: header.timestamp_ms,
            start,
            bits: end - start,
        })
    }

    /// Bytes of one update holding `rows` rows totalling `row_bits` bits.
    pub(crate) fn update_bytes(row_bits: usize, profile: WireProfile) -> usize {
        (profile.header_bits() + row_bits + 1).div_ceil(8)
    }

    pub(crate) fn bits(&self) -> usize {
        self.bits
    }
}

fn skip(bits: &mut BitReader<'_>, mut width: usize) -> Result<(), &'static str> {
    while width > 0 {
        let step = width.min(32);
        bits.read(step)?;
        width -= step;
    }
    Ok(())
}

fn copy(
    source: &mut BitReader<'_>,
    out: &mut BitWriter,
    mut width: usize,
) -> Result<(), &'static str> {
    while width > 0 {
        let step = width.min(32);
        out.write(source.read(step)?, step)?;
        width -= step;
    }
    Ok(())
}

/// Join measured rows (`rows[i]` measured from `wires[i]`) into one update
/// with `sequence`. Callers keep the rows to one timestamp, distinct objects
/// and at most [`MAX_UPDATE_BYTES`]; this checks all three.
pub(crate) fn join(
    wires: &[&[u8]],
    rows: &[CharacterRow],
    sequence: u32,
    profile: WireProfile,
) -> Result<Vec<u8>, &'static str> {
    let first = rows.first().ok_or("empty_character_coalesce")?;
    if wires.len() != rows.len() || sequence == 0 {
        return Err("invalid_character_coalesce");
    }
    let mut seen = std::collections::BTreeSet::new();
    let mut row_bits = 0;
    for row in rows {
        if row.timestamp_ms != first.timestamp_ms || !seen.insert(row.object) {
            return Err("invalid_character_coalesce");
        }
        row_bits += row.bits;
    }
    if CharacterRow::update_bytes(row_bits, profile) > MAX_UPDATE_BYTES {
        return Err("replication_packet_too_long");
    }
    let mut out = BitWriter::with_profile(profile);
    UpdateHeader {
        sequence,
        nak_sequence: 0,
        client_index: 0,
        object_type_mask: CHARACTER_MASK,
        timestamp_ms: first.timestamp_ms,
    }
    .encode(&mut out)?;
    for (wire, row) in wires.iter().zip(rows) {
        let mut source = BitReader::with_profile(wire, profile);
        source.position = row.start;
        copy(&mut source, &mut out, row.bits)?;
    }
    out.write(0, 1)?;
    Ok(out.finish())
}

/// Object indices of a Character-only server update's rows, when every row
/// carries only measured fields.
#[cfg(test)]
pub(crate) fn objects(wire: &[u8], profile: WireProfile) -> Option<Vec<u16>> {
    let header = ObjectUpdate::decode_with_profile(wire, profile)
        .ok()?
        .header;
    if header.object_type_mask != CHARACTER_MASK {
        return None;
    }
    let mut bits = BitReader::with_profile(wire, profile);
    bits.position = profile.header_bits();
    let mut objects = Vec::new();
    while bits.read(1).ok()? == 1 {
        objects.push(bits.read(16).ok()? as u16);
        for width in FIELD_BITS {
            if bits.read(1).ok()? == 1 {
                skip(&mut bits, width?).ok()?;
            }
        }
    }
    (wire.len() * 8 - bits.position < 8).then_some(objects)
}

#[cfg(test)]
#[path = "character_coalescing_tests.rs"]
mod tests;
