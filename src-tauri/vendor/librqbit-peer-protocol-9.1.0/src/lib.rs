// Modified by Kiwix Tauri. Check buffered frame headers and layout.

#![forbid(unsafe_code)]

// BitTorrent peer protocol implementation: parsing, serialization etc.
//
// Can be used outside of librqbit.

mod double_buf;
pub mod extended;

use buffers::{ByteBuf, ByteBufOwned};
use byteorder::{BE, ByteOrder};
use bytes::Bytes;
use clone_to_owned::CloneToOwned;
use extended::PeerExtendedMessageIds;
use librqbit_core::{constants::CHUNK_SIZE, hash_id::Id20, lengths::ChunkInfo};
use serde_derive::{Deserialize, Serialize};

pub use crate::double_buf::DoubleBufHelper;

use self::extended::ExtendedMessage;

const INTEGER_LEN: usize = 4;
const MSGID_LEN: usize = 1;
const PREAMBLE_LEN: usize = INTEGER_LEN + MSGID_LEN;
const PIECE_MESSAGE_PREAMBLE_LEN: usize = PREAMBLE_LEN + INTEGER_LEN * 2;
pub const PIECE_MESSAGE_DEFAULT_LEN: usize = PIECE_MESSAGE_PREAMBLE_LEN + CHUNK_SIZE as usize;

// extended message ut_metadata request is the largest known message.
const MAX_MSG_LEN_LEN_JUST_IN_CASE_EXTRA: usize = 64;
pub const MAX_MSG_LEN: usize = PREAMBLE_LEN
    + 1
    + b"d8:msg_typei1e5:piecei42e10:total_sizei16384ee".len()
    + CHUNK_SIZE as usize
    + MAX_MSG_LEN_LEN_JUST_IN_CASE_EXTRA;

const PSTR_BT1: &str = "BitTorrent protocol";

type MsgId = u8;

const MSGID_CHOKE: MsgId = 0;
const MSGID_UNCHOKE: MsgId = 1;
const MSGID_INTERESTED: MsgId = 2;
const MSGID_NOT_INTERESTED: MsgId = 3;
const MSGID_HAVE: MsgId = 4;
const MSGID_BITFIELD: MsgId = 5;
const MSGID_REQUEST: MsgId = 6;
const MSGID_PIECE: MsgId = 7;
const MSGID_CANCEL: MsgId = 8;
const MSGID_EXTENDED: MsgId = 20;

pub const EXTENDED_UT_METADATA_KEY: &[u8] = b"ut_metadata";
pub const MY_EXTENDED_UT_METADATA: u8 = 3;

pub const EXTENDED_UT_PEX_KEY: &[u8] = b"ut_pex";
pub const MY_EXTENDED_UT_PEX: u8 = 1;

#[derive(Clone, Copy)]
pub struct MsgIdDebug(MsgId);
impl MsgIdDebug {
    const fn name(&self) -> Option<&'static str> {
        let n = match self.0 {
            MSGID_CHOKE => "choke",
            MSGID_UNCHOKE => "unchoke",
            MSGID_INTERESTED => "interested",
            MSGID_NOT_INTERESTED => "not_interested",
            MSGID_HAVE => "have",
            MSGID_BITFIELD => "bitfield",
            MSGID_REQUEST => "request",
            MSGID_PIECE => "piece",
            MSGID_CANCEL => "cancel",
            MSGID_EXTENDED => "extended",
            _ => return None,
        };
        Some(n)
    }
}
impl core::fmt::Debug for MsgIdDebug {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.name() {
            Some(name) => f.write_str(name),
            None => write!(f, "<unknown msg_id {}>", self.0),
        }
    }
}

#[derive(thiserror::Error, Debug)]
pub enum MessageDeserializeError {
    #[error("not enough data (msgid={1:?}): expected at least {0} more bytes")]
    NotEnoughData(usize, Option<MsgIdDebug>),
    #[error("need a contiguous input to deserialize")]
    NeedContiguous,
    #[error("unsupported message id {0}")]
    UnsupportedMessageId(u8),
    #[error(transparent)]
    Bencode(#[from] bencode::DeserializeError),
    #[error("message length {0} cannot fit in this address space")]
    FrameLengthOverflow(u32),
    #[error("incorrect message length msg_id={msg_id:?}, expected={expected}, received={received}")]
    IncorrectMsgLen {
        received: u32,
        expected: u32,
        msg_id: MsgIdDebug,
    },
    #[error("ut_metadata:data received {received_len} >= total_size is {total_size}")]
    UtMetadataBufLargerThanTotalSize { total_size: u32, received_len: u32 },
    #[error("ut_metadata:data length must be <= {CHUNK_SIZE} but received {0} bytes")]
    UtMetadataTooLarge(u32),
    #[error("ut_metadata: trailing bytes when decoding")]
    UtMetadataTrailingBytes,
    #[error("ut_metadata: missing total_size")]
    UtMetadataMissingTotalSize,
    #[error("ut_metadata: unrecognized message type: {0}")]
    UtMetadataTypeUnknown(u32),
    #[error("ut_metadata: received piece {received_piece} > total pieces {total_pieces}")]
    UtMetadataPieceOutOfBounds {
        total_pieces: u32,
        received_piece: u32,
    },
    #[error("ut_metadata: expected size {expected_size} != received size {received_size}")]
    UtMetadataSizeMismatch {
        expected_size: u32,
        received_size: u32,
    },
    #[error("pstr doesn't match {PSTR_BT1:?}")]
    HandshakePstrWrongContent,
    #[error("pstr should be 19 bytes long but got {0}")]
    HandshakePstrWrongLength(u8),
}

pub fn serialize_piece_preamble(chunk: &ChunkInfo, mut buf: &mut [u8]) -> usize {
    let len_prefix = MSGID_LEN as u32 + INTEGER_LEN as u32 * 2 + chunk.size;
    BE::write_u32(&mut buf[0..4], len_prefix);
    buf[4] = MSGID_PIECE;

    buf = &mut buf[5..];
    BE::write_u32(&mut buf[0..4], chunk.piece_index.get());
    BE::write_u32(&mut buf[4..8], chunk.offset);

    PIECE_MESSAGE_PREAMBLE_LEN
}

pub struct Piece<B> {
    pub index: u32,
    pub begin: u32,
    block_0: B,
    block_1: B,
}

impl<B: AsRef<[u8]>> std::fmt::Debug for Piece<B> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Piece")
            .field("index", &self.index)
            .field("begin", &self.begin)
            .field("len", &self.len())
            .field("len_0", &self.block_0.as_ref().len())
            .field("len_1", &self.block_1.as_ref().len())
            .finish_non_exhaustive()
    }
}

impl CloneToOwned for Piece<ByteBuf<'_>> {
    type Target = Piece<ByteBufOwned>;

    fn clone_to_owned(&self, within_buffer: Option<&Bytes>) -> Self::Target {
        Piece {
            index: self.index,
            begin: self.begin,
            block_0: self.block_0.clone_to_owned(within_buffer),
            block_1: self.block_1.clone_to_owned(within_buffer),
        }
    }
}

impl<B: AsRef<[u8]>> Piece<B> {
    #[allow(clippy::len_without_is_empty)]
    pub fn len(&self) -> usize {
        self.block_0.as_ref().len() + self.block_1.as_ref().len()
    }

    pub fn serialize_unchecked_len(&self, mut buf: &mut [u8]) -> usize {
        buf[0..4].copy_from_slice(&self.index.to_be_bytes());
        buf[4..8].copy_from_slice(&self.begin.to_be_bytes());
        buf = &mut buf[8..];

        let b0 = self.block_0.as_ref();
        let b1 = self.block_1.as_ref();

        buf[..b0.len()].copy_from_slice(b0);
        buf = &mut buf[b0.len()..];
        buf[..b1.len()].copy_from_slice(b1);
        8 + b0.len() + b1.len()
    }
}

impl Piece<ByteBufOwned> {
    pub fn as_borrowed(&self) -> Piece<ByteBuf<'_>> {
        Piece {
            index: self.index,
            begin: self.begin,
            block_0: self.block_0.as_ref().into(),
            block_1: self.block_1.as_ref().into(),
        }
    }
}

impl<'a> Piece<ByteBuf<'a>> {
    pub fn data(&self) -> (&'a [u8], &'a [u8]) {
        (self.block_0.0, self.block_1.0)
    }

    pub fn from_data(index: u32, begin: u32, block: &'a [u8]) -> Self {
        Piece {
            index,
            begin,
            block_0: ByteBuf(block),
            block_1: ByteBuf(&[]),
        }
    }
}

#[derive(Debug)]
pub enum Message<'a> {
    Request(Request),
    Cancel(Request),
    Bitfield(ByteBuf<'a>),
    KeepAlive,
    Have(u32),
    Choke,
    Unchoke,
    Interested,
    NotInterested,
    Piece(Piece<ByteBuf<'a>>),
    Extended(ExtendedMessage<ByteBuf<'a>>),
}

#[derive(thiserror::Error, Debug)]
pub enum SerializeError {
    #[error("not enough space in buffer")]
    NoSpaceInBuffer,
    #[error("message is too large for the frame length")]
    FrameLengthOverflow,
    #[error(transparent)]
    Bencode(#[from] bencode::SerializeError),
    #[error("need peer's handshake to serialize ut_metadata, or peer does't support ut_metadata")]
    NeedUtMetadata,
    #[error("need peer's handshake to serialize ut_pex, or peer does't support ut_pex")]
    NeedPex,
}

impl From<std::io::Error> for SerializeError {
    fn from(_: std::io::Error) -> Self {
        Self::NoSpaceInBuffer
    }
}

fn reserve_frame(
    out: &mut [u8],
    msg_id: MsgId,
    payload_len: usize,
) -> Result<&mut [u8], SerializeError> {
    let wire_len = u32::try_from(payload_len)
        .ok()
        .and_then(|len| len.checked_add(MSGID_LEN as u32))
        .ok_or(SerializeError::FrameLengthOverflow)?;
    let total_len = payload_len
        .checked_add(PREAMBLE_LEN)
        .ok_or(SerializeError::FrameLengthOverflow)?;
    let (header, payload) = out
        .get_mut(..total_len)
        .ok_or(SerializeError::NoSpaceInBuffer)?
        .split_at_mut(PREAMBLE_LEN);
    header[..INTEGER_LEN].copy_from_slice(&wire_len.to_be_bytes());
    header[INTEGER_LEN] = msg_id;
    Ok(payload)
}

fn serialize_request(
    out: &mut [u8],
    msg_id: MsgId,
    request: &Request,
) -> Result<usize, SerializeError> {
    const PAYLOAD_LEN: usize = INTEGER_LEN * 3;
    let payload = reserve_frame(out, msg_id, PAYLOAD_LEN)?;
    request.serialize_unchecked_len(payload);
    Ok(PREAMBLE_LEN + PAYLOAD_LEN)
}

fn serialize_control(out: &mut [u8], msg_id: MsgId) -> Result<usize, SerializeError> {
    reserve_frame(out, msg_id, 0)?;
    Ok(PREAMBLE_LEN)
}

impl Message<'_> {
    pub fn serialize(
        &self,
        out: &mut [u8],
        peer_extended_messages: &dyn Fn() -> PeerExtendedMessageIds,
    ) -> Result<usize, SerializeError> {
        match self {
            Message::Request(request) => serialize_request(out, MSGID_REQUEST, request),
            Message::Cancel(request) => serialize_request(out, MSGID_CANCEL, request),
            Message::Bitfield(b) => {
                let data = b.as_ref();
                reserve_frame(out, MSGID_BITFIELD, data.len())?.copy_from_slice(data);
                Ok(PREAMBLE_LEN + data.len())
            }
            Message::Choke => serialize_control(out, MSGID_CHOKE),
            Message::Unchoke => serialize_control(out, MSGID_UNCHOKE),
            Message::Interested => serialize_control(out, MSGID_INTERESTED),
            Message::NotInterested => serialize_control(out, MSGID_NOT_INTERESTED),
            Message::Piece(p) => {
                let payload_len = p
                    .len()
                    .checked_add(INTEGER_LEN * 2)
                    .ok_or(SerializeError::FrameLengthOverflow)?;
                let payload = reserve_frame(out, MSGID_PIECE, payload_len)?;
                p.serialize_unchecked_len(payload);
                Ok(PREAMBLE_LEN + payload_len)
            }
            Message::KeepAlive => {
                out.get_mut(..INTEGER_LEN)
                    .ok_or(SerializeError::NoSpaceInBuffer)?
                    .copy_from_slice(&0u32.to_be_bytes());
                Ok(INTEGER_LEN)
            }
            Message::Have(v) => {
                reserve_frame(out, MSGID_HAVE, INTEGER_LEN)?.copy_from_slice(&v.to_be_bytes());
                Ok(PREAMBLE_LEN + INTEGER_LEN)
            }
            Message::Extended(e) => {
                if out.len() < PREAMBLE_LEN + 2 {
                    return Err(SerializeError::NoSpaceInBuffer);
                }
                let msg_len = e.serialize(&mut out[PREAMBLE_LEN..], peer_extended_messages)?;
                reserve_frame(out, MSGID_EXTENDED, msg_len)?;
                Ok(PREAMBLE_LEN + msg_len)
            }
        }
    }
}

struct MessageHeader {
    id: MsgId,
    payload_len: usize,
    total_len: usize,
}

#[inline]
fn read_message_header(
    buf: &mut DoubleBufHelper<'_>,
) -> Result<Option<MessageHeader>, MessageDeserializeError> {
    let len_prefix = buf
        .read_u32_be()
        .map_err(|missing| MessageDeserializeError::NotEnoughData(missing, None))?;
    let total_len = usize::try_from(len_prefix)
        .ok()
        .and_then(|len| len.checked_add(INTEGER_LEN))
        .ok_or(MessageDeserializeError::FrameLengthOverflow(len_prefix))?;
    if len_prefix == 0 {
        return Ok(None);
    }

    let id = buf.read_u8().ok_or(MessageDeserializeError::NotEnoughData(
        len_prefix as usize,
        None,
    ))?;
    let payload_len = total_len - PREAMBLE_LEN;
    if buf.len() < payload_len {
        return Err(MessageDeserializeError::NotEnoughData(
            payload_len - buf.len(),
            Some(MsgIdDebug(id)),
        ));
    }
    Ok(Some(MessageHeader {
        id,
        payload_len,
        total_len,
    }))
}

impl Message<'_> {
    /// Checks that the first frame is complete and can be passed to the
    /// decoder without refilling or compacting the buffer.
    ///
    /// Call `deserialize` on the same slices to validate and decode it.
    #[inline]
    pub fn check_frame_ready(first: &[u8], second: &[u8]) -> Result<(), MessageDeserializeError> {
        let mut buf = DoubleBufHelper::new(first, second);
        let Some(header) = read_message_header(&mut buf)? else {
            return Ok(());
        };
        match header.id {
            MSGID_BITFIELD if buf.get_contiguous(header.payload_len).is_none() => {
                return Err(MessageDeserializeError::NeedContiguous);
            }
            MSGID_EXTENDED if buf.get_contiguous(header.payload_len).is_none() => {
                ExtendedMessage::check_frame_ready(buf.with_max_len(header.payload_len))?;
            }
            _ => {}
        }
        Ok(())
    }

    pub fn deserialize<'a>(
        buf: &'a [u8],
        buf2: &'a [u8],
    ) -> Result<(Message<'a>, usize), MessageDeserializeError> {
        let mut buf = DoubleBufHelper::new(buf, buf2);
        let Some(MessageHeader {
            id: msg_id,
            payload_len: msg_len,
            total_len,
        }) = read_message_header(&mut buf)?
        else {
            return Ok((Message::KeepAlive, INTEGER_LEN));
        };

        macro_rules! check_msg_len {
            ($expected:expr) => {{
                if msg_len != $expected {
                    return Err(MessageDeserializeError::IncorrectMsgLen {
                        received: msg_len as u32,
                        expected: $expected,
                        msg_id: MsgIdDebug(msg_id),
                    });
                }
            }};
            (min $expected:expr) => {{
                if msg_len < $expected {
                    return Err(MessageDeserializeError::IncorrectMsgLen {
                        received: msg_len as u32,
                        expected: $expected,
                        msg_id: MsgIdDebug(msg_id),
                    });
                }
            }};
        }

        match msg_id {
            MSGID_CHOKE => {
                check_msg_len!(0);
                Ok((Message::Choke, total_len))
            }
            MSGID_UNCHOKE => {
                check_msg_len!(0);
                Ok((Message::Unchoke, total_len))
            }
            MSGID_INTERESTED => {
                check_msg_len!(0);
                Ok((Message::Interested, total_len))
            }
            MSGID_NOT_INTERESTED => {
                check_msg_len!(0);
                Ok((Message::NotInterested, total_len))
            }
            MSGID_HAVE => {
                check_msg_len!(4);
                let have = buf.read_u32_be().unwrap();
                Ok((Message::Have(have), total_len))
            }
            MSGID_BITFIELD => {
                check_msg_len!(min 1);
                // In practice, as bitfield is always (almost) the first message, it should be contiguous.
                let data = buf
                    .get_contiguous(msg_len)
                    .ok_or(MessageDeserializeError::NeedContiguous)?;
                Ok((Message::Bitfield(ByteBuf::from(data)), total_len))
            }
            MSGID_REQUEST | MSGID_CANCEL => {
                check_msg_len!(12);
                const I32: usize = 4;
                const I32_3: usize = I32 * 3;
                let req = buf.consume::<I32_3>().unwrap();
                let request = Request {
                    index: BE::read_u32(&req[0..I32]),
                    begin: BE::read_u32(&req[I32..I32 * 2]),
                    length: BE::read_u32(&req[I32 * 2..I32 * 3]),
                };
                let req = if msg_id == MSGID_REQUEST {
                    Message::Request(request)
                } else {
                    Message::Cancel(request)
                };
                Ok((req, total_len))
            }
            MSGID_PIECE => {
                const MIN_PAYLOAD: usize = 1;
                const MIN_LENGTH: usize = INTEGER_LEN * 2 + MIN_PAYLOAD;
                if msg_len < MIN_LENGTH {
                    return Err(MessageDeserializeError::IncorrectMsgLen {
                        expected: MIN_LENGTH as u32,
                        received: msg_len as u32,
                        msg_id: MsgIdDebug(msg_id),
                    });
                }

                let index = buf.read_u32_be().unwrap();
                let begin = buf.read_u32_be().unwrap();

                let block_len = msg_len - INTEGER_LEN * 2;
                let (block_0, block_1) = buf.consume_variable(block_len).unwrap();

                Ok((
                    Message::Piece(Piece {
                        index,
                        begin,
                        block_0: block_0.into(),
                        block_1: block_1.into(),
                    }),
                    total_len,
                ))
            }
            MSGID_EXTENDED => {
                check_msg_len!(min 1);
                Ok((
                    Message::Extended(ExtendedMessage::deserialize(buf.with_max_len(msg_len))?),
                    total_len,
                ))
            }
            msg_id => Err(MessageDeserializeError::UnsupportedMessageId(msg_id)),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct Handshake {
    pub reserved: u64,
    pub info_hash: Id20,
    pub peer_id: Id20,
}

impl Handshake {
    pub fn new(info_hash: Id20, peer_id: Id20) -> Handshake {
        debug_assert_eq!(PSTR_BT1.len(), 19);

        let mut reserved: u64 = 0;
        // supports extended messaging
        reserved |= 1 << 20;

        Handshake {
            reserved,
            info_hash,
            peer_id,
        }
    }

    pub fn deserialize(b: &[u8]) -> Result<(Handshake, usize), MessageDeserializeError> {
        const LEN: usize = 1 + PSTR_BT1.len() + 8 + 20 + 20;
        if b.len() < LEN {
            return Err(MessageDeserializeError::NotEnoughData(LEN - b.len(), None));
        }
        if b[0] as usize != PSTR_BT1.len() {
            return Err(MessageDeserializeError::HandshakePstrWrongLength(b[0]));
        }
        if &b[1..20] != PSTR_BT1.as_bytes() {
            return Err(MessageDeserializeError::HandshakePstrWrongContent);
        }

        let h = Handshake {
            reserved: BE::read_u64(&b[20..28]),
            info_hash: Id20::new(b[28..48].try_into().unwrap()),
            peer_id: Id20::new(b[48..68].try_into().unwrap()),
        };
        Ok((h, LEN))
    }

    pub fn supports_extended(&self) -> bool {
        self.reserved.to_be_bytes()[5] & 0x10 > 0
    }

    #[must_use]
    pub fn serialize_unchecked_len(&self, buf: &mut [u8]) -> usize {
        debug_assert_eq!(PSTR_BT1.len(), 19);
        buf[0] = 19;
        buf[1..20].copy_from_slice(PSTR_BT1.as_bytes());
        buf[20..28].copy_from_slice(&self.reserved.to_be_bytes());
        buf[28..48].copy_from_slice(&self.info_hash.0);
        buf[48..68].copy_from_slice(&self.peer_id.0);
        68
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy)]
pub struct Request {
    pub index: u32,
    pub begin: u32,
    pub length: u32,
}

impl Request {
    pub fn new(index: u32, begin: u32, length: u32) -> Self {
        Self {
            index,
            begin,
            length,
        }
    }

    pub fn serialize_unchecked_len(&self, buf: &mut [u8]) -> usize {
        buf[0..4].copy_from_slice(&self.index.to_be_bytes());
        buf[4..8].copy_from_slice(&self.begin.to_be_bytes());
        buf[8..12].copy_from_slice(&self.length.to_be_bytes());
        12
    }
}

#[cfg(test)]
mod tests {
    use anyhow::Context;

    use crate::extended::handshake::ExtendedHandshake;

    const EXTENDED: &[u8] = include_bytes!("../tests/fixtures/extended-handshake.bin");

    use super::*;
    #[test]
    fn test_handshake_serialize() {
        let info_hash = Id20::new([
            1u8, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20,
        ]);
        let peer_id = Id20::new([
            1u8, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20,
        ]);
        let mut buf = [0u8; 100];
        let se = Handshake::new(info_hash, peer_id);
        let len = se.serialize_unchecked_len(&mut buf);
        assert_eq!(len, 20 + 20 + 8 + 19 + 1);
        assert_eq!(buf[0], 19);
        assert_eq!(&buf[1..20], PSTR_BT1.as_bytes());
        assert_eq!(&buf[28..48], &info_hash.0);
        assert_eq!(&buf[48..68], &peer_id.0);

        let (de, dlen) = Handshake::deserialize(&buf).unwrap();
        assert_eq!(dlen, len);
        assert_eq!(se, de);
    }

    #[test]
    fn test_extended_serialize() {
        let msg = Message::Extended(ExtendedMessage::Handshake(ExtendedHandshake::new()));
        let mut out = [0u8; 100];
        msg.serialize(&mut out, &Default::default).unwrap();
        dbg!(out);
    }

    #[test]
    fn test_deserialize_serialize_extended_non_contiguous() {
        let mut frames = EXTENDED.to_vec();
        frames.extend_from_slice(&[0; INTEGER_LEN]); // Following keep-alive.
        for split_point in 0..=frames.len() {
            let (first, second) = frames.split_at(split_point);
            let ready = Message::check_frame_ready(first, second);
            let res = Message::deserialize(first, second);
            if split_point > PREAMBLE_LEN + 1 && split_point < EXTENDED.len() {
                assert!(
                    matches!(ready, Err(MessageDeserializeError::NeedContiguous)),
                    "expected compaction before parsing: {split_point}"
                );
                assert!(
                    matches!(res, Err(MessageDeserializeError::NeedContiguous)),
                    "expected NeedContiguous: {split_point}"
                )
            } else {
                ready.unwrap();
                let (msg, len) = res
                    .inspect_err(|e| panic!("split_point={split_point:?}; error: {e:#}"))
                    .unwrap();
                assert!(matches!(msg, Message::Extended(..)));
                assert_eq!(len, EXTENDED.len());
            }
        }
    }

    #[test]
    fn readiness_does_not_decode_a_contiguous_extension_body() {
        // The extension ID ends the first slice and the invalid bencode
        // body is contiguous in the second slice.
        let first = [0, 0, 0, 3, MSGID_EXTENDED, 0];
        let second = b"x";
        assert!(Message::check_frame_ready(&first, second).is_ok());
        assert!(matches!(
            Message::deserialize(&first, second),
            Err(MessageDeserializeError::Bencode(_))
        ));
    }

    #[test]
    fn test_deserialize_piece() {
        const LEN: usize = 100;
        const EXTRA: usize = 100;
        let mut buf = [0u8; LEN + EXTRA];

        #[allow(clippy::needless_range_loop)]
        for id in 0..buf.len() {
            buf[id] = id as u8;
        }

        let block_len = LEN - PREAMBLE_LEN - INTEGER_LEN * 2;
        let len_prefix: u32 = (block_len + INTEGER_LEN * 2 + MSGID_LEN) as u32;
        let index: u32 = 42;
        let begin: u32 = 43;

        buf[0..4].copy_from_slice(&len_prefix.to_be_bytes());
        buf[4] = MSGID_PIECE;
        buf[5..9].copy_from_slice(&index.to_be_bytes());
        buf[9..13].copy_from_slice(&begin.to_be_bytes());

        for split_point in 0..buf.len() {
            dbg!(split_point);
            let (first, second) = buf.split_at(split_point);
            let (msg, len) = Message::deserialize(first, second).unwrap();

            let piece = match &msg {
                Message::Piece(piece) => piece,
                other => panic!("expected piece got {other:?}"),
            };

            assert_eq!(piece.len(), block_len);
            assert_eq!(piece.index, index);
            assert_eq!(piece.begin, begin);
            assert_eq!(len, LEN);

            let mut tmp = [0u8; 100];
            let slen = msg.serialize(&mut tmp, &|| Default::default()).unwrap();
            assert_eq!(slen, len);
            assert_eq!(buf[..len], tmp[..len]);

            let (first, second) = piece.data();

            assert_eq!(first.len() + second.len(), block_len);
            assert_eq!(first, &buf[13..13 + first.len()]);
            assert_eq!(
                second,
                &buf[13 + first.len()..13 + first.len() + second.len()]
            );
        }
    }

    #[test]
    fn test_deserialize_request() {
        let mut buf = [0u8; 100];

        let len_prefix: u32 = (MSGID_LEN + INTEGER_LEN * 3) as u32;
        let index: u32 = 42;
        let begin: u32 = 43;
        let length: u32 = 44;

        buf[0..4].copy_from_slice(&len_prefix.to_be_bytes());
        buf[4] = MSGID_REQUEST;
        buf[5..9].copy_from_slice(&index.to_be_bytes());
        buf[9..13].copy_from_slice(&begin.to_be_bytes());
        buf[13..17].copy_from_slice(&length.to_be_bytes());

        for split_point in 0..buf.len() {
            dbg!(split_point);
            let (first, second) = buf.split_at(split_point);
            let (msg, len) = Message::deserialize(first, second).unwrap();

            let request = match msg {
                Message::Request(req) => req,
                other => panic!("expected request got {other:?}"),
            };

            assert_eq!(request.index, index);
            assert_eq!(request.begin, begin);
            assert_eq!(request.length, length);
            assert_eq!(len, 17);

            let mut tmp = [0u8; 100];
            let slen = msg.serialize(&mut tmp, &|| Default::default()).unwrap();
            assert_eq!(slen, len);
            assert_eq!(buf[..len], tmp[..len]);
        }
    }

    #[test]
    fn test_keepalive() {
        let buf = [0u8; 100];

        for split_point in 0..buf.len() {
            let (first, second) = buf.split_at(split_point);
            let (msg, len) = Message::deserialize(first, second).unwrap();
            assert!(matches!(msg, Message::KeepAlive));
            assert_eq!(len, 4);
            let mut tmp = [0u8; 100];
            let slen = msg.serialize(&mut tmp, &|| Default::default()).unwrap();
            assert_eq!(slen, len);
            assert_eq!(buf[..len], tmp[..len]);
        }
    }

    #[test]
    fn test_have() {
        let mut buf = [0u8; 100];
        buf[0..4].copy_from_slice(&5u32.to_be_bytes());
        buf[4] = MSGID_HAVE;
        buf[5..9].copy_from_slice(&42u32.to_be_bytes());

        for split_point in 0..buf.len() {
            let (first, second) = buf.split_at(split_point);
            let (msg, len) = Message::deserialize(first, second).unwrap();
            assert!(matches!(msg, Message::Have(42)));
            assert_eq!(len, 9);
            let mut tmp = [0u8; 100];
            let slen = msg.serialize(&mut tmp, &|| Default::default()).unwrap();
            assert_eq!(slen, len);
            assert_eq!(buf[..len], tmp[..len]);
        }
    }

    #[test]
    fn test_bitfield() {
        let mut buf = [0u8; 100];
        buf[0..4].copy_from_slice(&43u32.to_be_bytes());
        buf[4] = MSGID_BITFIELD;
        for byte in buf[5..47].iter_mut() {
            *byte = 0b10101010;
        }

        for split_point in 0..buf.len() {
            let (first, second) = buf.split_at(split_point);
            let res = Message::deserialize(first, second);
            if (6..47).contains(&split_point) {
                assert!(
                    matches!(res, Err(MessageDeserializeError::NeedContiguous)),
                    "expected NeedContiguous: split_point={split_point}"
                );
                continue;
            }
            let (msg, len) = res.context(split_point).unwrap();
            let bf = match &msg {
                Message::Bitfield(bf) => bf,
                other => panic!("expected bitfield, got {other:?}"),
            };
            assert_eq!(len, 47);
            assert_eq!(bf.as_ref().len(), 42);
            for byte in bf.as_ref() {
                assert_eq!(*byte, 0b10101010);
            }
            let mut tmp = [0u8; 100];
            let slen = msg.serialize(&mut tmp, &|| Default::default()).unwrap();
            assert_eq!(slen, len);
            assert_eq!(buf[..len], tmp[..len]);
        }
    }

    #[test]
    fn test_no_data_messages() {
        let mut buf = [0u8; 100];

        for msgid in [
            MSGID_CHOKE,
            MSGID_UNCHOKE,
            MSGID_INTERESTED,
            MSGID_NOT_INTERESTED,
        ] {
            buf[0..4].copy_from_slice(&1u32.to_be_bytes());
            buf[4] = msgid;
            for split_point in 0..buf.len() {
                let (first, second) = buf.split_at(split_point);
                let (msg, len) = Message::deserialize(first, second).unwrap();
                match (msgid, &msg) {
                    (MSGID_CHOKE, Message::Choke)
                    | (MSGID_UNCHOKE, Message::Unchoke)
                    | (MSGID_INTERESTED, Message::Interested)
                    | (MSGID_NOT_INTERESTED, Message::NotInterested) => {}
                    (msgid, msg) => panic!("msgid={msgid}, msg={msg:?}"),
                }
                assert_eq!(len, 5);
                let mut tmp = [0u8; 100];
                let slen = msg.serialize(&mut tmp, &|| Default::default()).unwrap();
                assert_eq!(slen, len);
                assert_eq!(buf[..len], tmp[..len]);
            }
        }
    }
}

#[cfg(test)]
mod checked_frame_tests {
    use super::*;

    #[test]
    fn request_and_control_frames_keep_wire_bytes_across_splits() {
        let request = || Request {
            index: 2,
            begin: 4,
            length: 16,
        };
        let block = [0, 0, 0, 2, 0, 0, 0, 4, 0, 0, 0, 16];
        let extension_ids = || panic!("ordinary frame consulted the peer's extension IDs");
        for (msg, id, payload) in [
            (Message::Request(request()), 6, block.as_slice()),
            (Message::Cancel(request()), 8, block.as_slice()),
            (Message::Choke, 0, &[]),
            (Message::Unchoke, 1, &[]),
            (Message::Interested, 2, &[]),
            (Message::NotInterested, 3, &[]),
        ] {
            let len = PREAMBLE_LEN + payload.len();
            let mut buffer = [0xf2; 27];
            assert!(matches!(
                msg.serialize(&mut buffer[..len - 1], &extension_ids),
                Err(SerializeError::NoSpaceInBuffer)
            ));
            assert_eq!(buffer, [0xf2; 27]);
            assert_eq!(msg.serialize(&mut buffer, &extension_ids).unwrap(), len);
            assert_eq!(&buffer[..4], &((payload.len() as u32 + 1).to_be_bytes()));
            assert_eq!(buffer[4], id);
            assert_eq!(&buffer[5..len], payload);
            assert!(buffer[len..].iter().all(|&b| b == 0xf2));
            for split in 0..=len {
                let (parsed, used) =
                    Message::deserialize(&buffer[..split], &buffer[split..len]).unwrap();
                let mut roundtrip = [0; 27];
                assert_eq!(used, len);
                assert_eq!(
                    parsed.serialize(&mut roundtrip, &extension_ids).unwrap(),
                    len
                );
                assert_eq!(&roundtrip[..len], &buffer[..len]);
            }
        }
    }

    #[test]
    fn extended_frame_without_an_extension_id_is_complete_but_invalid() {
        let frames = [0, 0, 0, 1, MSGID_EXTENDED, 0, 0, 0, 0];
        for available in [PREAMBLE_LEN, frames.len()] {
            for split in 0..=available {
                let parsed = Message::deserialize(&frames[..split], &frames[split..available]);
                assert!(matches!(
                    parsed,
                    Err(MessageDeserializeError::IncorrectMsgLen {
                        received: 0,
                        expected: 1,
                        msg_id: MsgIdDebug(MSGID_EXTENDED),
                    })
                ));
            }
        }
        let next = Message::deserialize(&frames[PREAMBLE_LEN..], &[]).unwrap();
        assert!(matches!(next, (Message::KeepAlive, INTEGER_LEN)));
    }

    #[test]
    fn out_of_range_wire_payload_keeps_destination_intact() {
        let mut output = [0x76; 12];
        assert!(matches!(
            reserve_frame(&mut output, MSGID_BITFIELD, u32::MAX as usize),
            Err(SerializeError::FrameLengthOverflow)
        ));
        assert_eq!(output, [0x76; 12]);
    }
}
