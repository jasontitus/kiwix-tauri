#![forbid(unsafe_code)]
use super::{LinkError, Result, MAX_MESSAGE};
use zeroize::Zeroizing;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Kind { Request = 1, Metadata = 2 }
struct Partial {
    kind: Kind,
    id: u16,
    total: usize,
    bytes: Zeroizing<Vec<u8>>,
}
pub(crate) struct Framing {
    budget: usize,
    tx: u32,
    rx: u32,
    partial: Option<Partial>,
    failed: bool,
}
impl Framing {
    pub(crate) fn new(budget: u16) -> Result<Self> {
        // ATT attribute values are at most 512 bytes, independent of MTU overhead.
        if !(20..=512).contains(&budget) { return Err(LinkError::Framing); }
        Ok(Self { budget: budget as usize, tx: 0, rx: 0, partial: None, failed: false })
    }
    pub(crate) fn clear(&mut self) { self.partial = None; self.failed = true; }
    pub(crate) fn fragment(&mut self, kind: Kind, bytes: &[u8]) -> Result<Vec<Vec<u8>>> {
        let result = self.fragment_inner(kind, bytes);
        if result.is_err() { self.clear(); }
        result
    }
    fn fragment_inner(&mut self, kind: Kind, bytes: &[u8]) -> Result<Vec<Vec<u8>>> {
        if self.failed || self.tx > u16::MAX as u32 || bytes.is_empty() || bytes.len() > MAX_MESSAGE {
            return Err(LinkError::Framing);
        }
        let payload = self.budget - 8;
        let mut packets = Vec::with_capacity(bytes.len().div_ceil(payload));
        for (index, chunk) in bytes.chunks(payload).enumerate() {
            let mut packet = Vec::with_capacity(8 + chunk.len());
            packet.extend_from_slice(&[1, kind as u8]);
            packet.extend_from_slice(&(self.tx as u16).to_be_bytes());
            packet.extend_from_slice(&((index * payload) as u16).to_be_bytes());
            packet.extend_from_slice(&(bytes.len() as u16).to_be_bytes());
            packet.extend_from_slice(chunk);
            packets.push(packet);
        }
        self.tx += 1;
        Ok(packets)
    }
    pub(crate) fn receive(&mut self, expected_kind: Kind, packet: &[u8]) -> Result<Option<Zeroizing<Vec<u8>>>> {
        let result = self.receive_inner(expected_kind, packet);
        if result.is_err() { self.clear(); }
        result
    }
    fn receive_inner(&mut self, expected_kind: Kind, packet: &[u8]) -> Result<Option<Zeroizing<Vec<u8>>>> {
        if self.failed || self.rx > u16::MAX as u32 || packet.len() <= 8 || packet.len() > self.budget
            || packet[0] != 1 || packet[1] != expected_kind as u8
        { return Err(LinkError::Framing); }
        let id = u16::from_be_bytes([packet[2], packet[3]]);
        let offset = u16::from_be_bytes([packet[4], packet[5]]) as usize;
        let total = u16::from_be_bytes([packet[6], packet[7]]) as usize;
        let payload = &packet[8..];
        if id as u32 != self.rx || !(1..=MAX_MESSAGE).contains(&total)
            || offset.checked_add(payload.len()).is_none_or(|end| end > total)
        { return Err(LinkError::Framing); }
        if self.partial.is_none() {
            if offset != 0 { return Err(LinkError::Framing); }
            self.partial = Some(Partial { kind: expected_kind, id, total, bytes: Zeroizing::new(Vec::with_capacity(total)) });
        }
        let partial = self.partial.as_mut().ok_or(LinkError::Framing)?;
        if partial.kind != expected_kind || partial.id != id || partial.total != total || partial.bytes.len() != offset {
            return Err(LinkError::Framing);
        }
        partial.bytes.extend_from_slice(payload);
        if partial.bytes.len() == total {
            self.rx += 1;
            Ok(self.partial.take().map(|partial| partial.bytes))
        } else { Ok(None) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounds_gaps_overlap_ids_and_kind_are_terminal() {
        let mut sender = Framing::new(20).unwrap();
        let packets = sender.fragment(Kind::Request, &[7; 49]).unwrap();
        assert!(packets.iter().all(|p| p.len() <= 20));
        for corrupted in 0..7 {
            let mut receiver = Framing::new(20).unwrap();
            let mut packet = packets[0].clone();
            match corrupted {
                0 => packet[0] = 2,
                1 => packet[1] = 2,
                2 => packet[3] = 1,
                3 => packet[5] = 1,
                4 => { packet[6] = 0x10; packet[7] = 1; },
                5 => packet.truncate(8),
                _ => packet.push(0),
            }
            assert!(receiver.receive(Kind::Request, &packet).is_err());
            assert!(receiver.receive(Kind::Request, &packets[0]).is_err());
        }
        for bad in [&packets[0], &packets[2]] {
            let mut receiver = Framing::new(20).unwrap();
            receiver.receive(Kind::Request, &packets[0]).unwrap();
            assert!(receiver.receive(Kind::Request, bad).is_err());
        }
        let mut receiver = Framing::new(20).unwrap();
        for packet in &packets[..packets.len()-1] { assert!(receiver.receive(Kind::Request, packet).unwrap().is_none()); }
        assert_eq!(&*receiver.receive(Kind::Request, packets.last().unwrap()).unwrap().unwrap(), &[7;49]);
        assert!(receiver.receive(Kind::Request, &packets[0]).is_err());
    }
    #[test]
    fn conflicting_total_overflow_and_exhaustion() {
        let mut sender = Framing::new(20).unwrap();
        let packets = sender.fragment(Kind::Metadata, &[1; 13]).unwrap();
        let mut receiver = Framing::new(20).unwrap();
        receiver.receive(Kind::Metadata, &packets[0]).unwrap();
        let mut second = packets[1].clone();
        second[7] = 14;
        assert!(receiver.receive(Kind::Metadata, &second).is_err());
        let mut overflow = vec![1,2,0,0,0,0,0,1];
        overflow.extend_from_slice(&[1,2]);
        assert!(Framing::new(20).unwrap().receive(Kind::Metadata, &overflow).is_err());
        let mut sender = Framing::new(20).unwrap();
        sender.tx = 65535;
        sender.fragment(Kind::Metadata, &[0]).unwrap();
        assert!(sender.fragment(Kind::Metadata, &[0]).is_err());
        assert!(Framing::new(19).is_err());
        assert!(Framing::new(513).is_err());
    }
}
