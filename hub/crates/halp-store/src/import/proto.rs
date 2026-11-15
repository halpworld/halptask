//! A small, bounded protobuf reader for the TUI's `TreeProto` / `ItemProto`
//! (see `proto/v1/storage.proto` in the TUI). Hand-written rather than
//! generated so the input bounds (size, depth, item count) are enforced while
//! decoding, before anything is allocated for an over-limit file.

use super::LegacyNode;
use crate::error::{Result, StoreError};

pub(super) struct Limits {
    pub max_depth: usize,
    pub max_items: usize,
}

struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

fn bad(msg: &str) -> StoreError {
    StoreError::Import(format!("not a valid TUI protobuf file: {msg}"))
}

impl<'a> Reader<'a> {
    fn new(buf: &'a [u8]) -> Self {
        Reader { buf, pos: 0 }
    }

    fn done(&self) -> bool {
        self.pos >= self.buf.len()
    }

    fn varint(&mut self) -> Result<u64> {
        let mut v: u64 = 0;
        for shift in (0..70).step_by(7) {
            let b = *self
                .buf
                .get(self.pos)
                .ok_or_else(|| bad("truncated varint"))?;
            self.pos += 1;
            if shift == 63 && b > 1 {
                return Err(bad("varint overflow"));
            }
            v |= u64::from(b & 0x7f) << shift;
            if b & 0x80 == 0 {
                return Ok(v);
            }
        }
        Err(bad("varint too long"))
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self
            .pos
            .checked_add(n)
            .ok_or_else(|| bad("length overflow"))?;
        let s = self
            .buf
            .get(self.pos..end)
            .ok_or_else(|| bad("truncated field"))?;
        self.pos = end;
        Ok(s)
    }

    fn bytes(&mut self) -> Result<&'a [u8]> {
        let n = usize::try_from(self.varint()?).map_err(|_| bad("length overflow"))?;
        self.take(n)
    }

    fn key(&mut self) -> Result<(u32, u8)> {
        let k = self.varint()?;
        let field = u32::try_from(k >> 3).map_err(|_| bad("field number overflow"))?;
        if field == 0 {
            return Err(bad("field number 0"));
        }
        Ok((field, (k & 7) as u8))
    }

    fn skip(&mut self, wire: u8) -> Result<()> {
        match wire {
            0 => self.varint().map(|_| ()),
            1 => self.take(8).map(|_| ()),
            2 => self.bytes().map(|_| ()),
            5 => self.take(4).map(|_| ()),
            _ => Err(bad("unsupported wire type")),
        }
    }
}

fn expect(wire: u8, want: u8) -> Result<()> {
    if wire == want {
        Ok(())
    } else {
        Err(bad("unexpected wire type"))
    }
}

fn string(bytes: &[u8], warnings: &mut Vec<String>) -> String {
    match std::str::from_utf8(bytes) {
        Ok(s) => s.to_string(),
        Err(_) => {
            if warnings.len() < super::MAX_WARNINGS {
                warnings.push("replaced invalid UTF-8 in a text field".into());
            }
            String::from_utf8_lossy(bytes).into_owned()
        }
    }
}

pub(super) struct Decoded {
    pub schema_version: u32,
    pub roots: Vec<LegacyNode>,
}

pub(super) fn decode_tree(
    buf: &[u8],
    limits: &Limits,
    warnings: &mut Vec<String>,
) -> Result<Decoded> {
    let mut r = Reader::new(buf);
    let mut count = 0usize;
    let mut out = Decoded {
        schema_version: 0,
        roots: Vec::new(),
    };
    while !r.done() {
        let (field, wire) = r.key()?;
        match field {
            1 => {
                expect(wire, 0)?;
                out.schema_version = u32::try_from(r.varint()?).unwrap_or(u32::MAX);
            }
            2 => {
                expect(wire, 2)?;
                let b = r.bytes()?;
                out.roots
                    .push(decode_item(b, 1, &mut count, limits, warnings)?);
            }
            _ => r.skip(wire)?,
        }
    }
    Ok(out)
}

fn decode_item(
    buf: &[u8],
    depth: usize,
    count: &mut usize,
    limits: &Limits,
    warnings: &mut Vec<String>,
) -> Result<LegacyNode> {
    if depth > limits.max_depth {
        return Err(StoreError::Import(format!(
            "the outline nests deeper than {} levels",
            limits.max_depth
        )));
    }
    *count += 1;
    if *count > limits.max_items {
        return Err(StoreError::Import(format!(
            "the file holds more than {} items",
            limits.max_items
        )));
    }
    let mut n = LegacyNode::default();
    let mut r = Reader::new(buf);
    while !r.done() {
        let (field, wire) = r.key()?;
        match field {
            1 => {
                expect(wire, 2)?;
                n.legacy_id = string(r.bytes()?, warnings);
            }
            2 => {
                expect(wire, 2)?;
                n.text = string(r.bytes()?, warnings);
            }
            3 => {
                expect(wire, 0)?;
                n.is_task = r.varint()? != 0;
            }
            4 => {
                expect(wire, 0)?;
                n.status = r.varint()?;
            }
            5 => {
                expect(wire, 0)?;
                n.folded = r.varint()? != 0;
            }
            6 => {
                expect(wire, 2)?;
                let t = string(r.bytes()?, warnings);
                // Bounded at decode; `convert` warns about the excess.
                if n.tags.len() <= halp_domain::limits::MAX_TAGS_PER_ITEM {
                    n.tags.push(t);
                }
            }
            7 => {
                expect(wire, 2)?;
                let b = r.bytes()?;
                n.children
                    .push(decode_item(b, depth + 1, count, limits, warnings)?);
            }
            8 => {
                expect(wire, 0)?;
                n.created_at_ns = r.varint()? as i64;
            }
            9 => {
                expect(wire, 0)?;
                n.updated_at_ns = r.varint()? as i64;
            }
            10 => {
                expect(wire, 2)?;
                n.node_id = string(r.bytes()?, warnings);
            }
            11 => {
                expect(wire, 0)?;
                n.deleted = r.varint()? != 0;
            }
            12 => {
                expect(wire, 0)?;
                n.version = r.varint()?;
            }
            13 => {
                expect(wire, 2)?;
                n.note = string(r.bytes()?, warnings);
            }
            14 => {
                expect(wire, 0)?;
                n.focused = r.varint()? != 0;
            }
            15 => {
                expect(wire, 0)?;
                n.due_unix = r.varint()? as i64;
            }
            16 => {
                expect(wire, 2)?;
                n.due_text = string(r.bytes()?, warnings);
            }
            _ => r.skip(wire)?,
        }
    }
    Ok(n)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn lim() -> Limits {
        Limits {
            max_depth: 64,
            max_items: 1000,
        }
    }

    #[test]
    fn varint_edges() {
        let mut r = Reader::new(&[0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x01]);
        assert_eq!(r.varint().unwrap(), u64::MAX);
        let mut r = Reader::new(&[0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x02]);
        assert!(r.varint().is_err());
        let mut r = Reader::new(&[0x80]);
        assert!(r.varint().is_err());
    }

    #[test]
    fn rejects_huge_lengths_and_bad_wire() {
        let mut w = Vec::new();
        // field 2, wire 2, length 2^40
        w.extend([0x12, 0x80, 0x80, 0x80, 0x80, 0x80, 0x20]);
        assert!(decode_tree(&w, &lim(), &mut vec![]).is_err());
        // field 2 with varint wire type
        assert!(decode_tree(&[0x10, 0x01], &lim(), &mut vec![]).is_err());
        // wire type 3 (group) unsupported
        assert!(decode_tree(&[0x1b], &lim(), &mut vec![]).is_err());
        // unknown fields of every wire type are skipped
        let ok = [
            0x18, 0x05, 0x21, 1, 2, 3, 4, 5, 6, 7, 8, 0x2a, 0x01, 0x00, 0x35, 1, 2, 3, 4,
        ];
        assert_eq!(
            decode_tree(&ok, &lim(), &mut vec![]).unwrap().roots.len(),
            0
        );
    }

    #[test]
    fn depth_and_count_limits() {
        // nest an item 70 levels deep: item{children: item{...}}
        let mut inner: Vec<u8> = vec![0x0a, 0x01, b'x'];
        for _ in 0..70 {
            let mut outer = vec![0x3a];
            push_varint(&mut outer, inner.len() as u64);
            outer.extend(&inner);
            inner = outer;
        }
        let mut tree = vec![0x12];
        push_varint(&mut tree, inner.len() as u64);
        tree.extend(&inner);
        let err = decode_tree(&tree, &lim(), &mut vec![]).err().unwrap();
        assert!(err.to_string().contains("deeper"));
        let small = Limits {
            max_depth: 100,
            max_items: 10,
        };
        assert!(
            decode_tree(&tree, &small, &mut vec![])
                .err()
                .unwrap()
                .to_string()
                .contains("more than")
        );
    }

    fn push_varint(out: &mut Vec<u8>, mut v: u64) {
        loop {
            let b = (v & 0x7f) as u8;
            v >>= 7;
            if v == 0 {
                out.push(b);
                return;
            }
            out.push(b | 0x80);
        }
    }
}
