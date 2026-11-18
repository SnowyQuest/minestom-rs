//! Divan microbenchmarks for M1 hot paths: `VarInt`, network NBT, `StatusResponsePacket`, and `FrameCodec`.

use std::hint::black_box;

use bytes::BytesMut;
use minestom::nbt::{NbtCompound, NbtTag, read_network_compound, write_network_compound};
use minestom::prelude::*;
use minestom::protocol::status::{ServerStatus, StatusPlayers, StatusResponsePacket};
use minestom::protocol::{
    Decode, Encode, FrameCodec, MAX_PACKET_SIZE, VarInt, decode_packet_exact, encode_packet_payload,
};

fn main() {
    divan::main();
}

#[divan::bench]
fn varint_encode_decode_batch(bencher: divan::Bencher) {
    let values = [
        0_i32,
        1,
        127,
        128,
        16_384,
        2_097_151,
        -1,
        i32::MIN,
        i32::MAX,
    ];
    bencher.bench_local(|| {
        let mut buf = BytesMut::with_capacity(64);
        for &v in black_box(&values) {
            VarInt(v).encode(&mut buf).unwrap();
        }
        let mut reader = buf.freeze();
        let mut sum = 0_i32;
        for _ in 0..values.len() {
            sum = sum.wrapping_add(VarInt::decode(&mut reader).unwrap().0);
        }
        black_box(sum)
    });
}

#[divan::bench]
fn network_nbt_compound_roundtrip(bencher: divan::Bencher) {
    let compound = NbtCompound::new()
        .with("type", "text")
        .with("text", "Hello from minestom-rs benchmark!")
        .with("color", "gold")
        .with("bold", true)
        .with("ints", NbtTag::IntArray(vec![1, 2, 3, 4, 5, 6, 7, 8]));

    bencher.bench_local(|| {
        let mut buf = BytesMut::with_capacity(256);
        write_network_compound(&mut buf, black_box(&compound)).unwrap();
        let mut reader = buf.freeze();
        black_box(read_network_compound(&mut reader).unwrap())
    });
}

#[divan::bench]
fn status_response_frame_roundtrip_compressed(bencher: divan::Bencher) {
    let pkt = StatusResponsePacket {
        status: ServerStatus {
            players: Some(StatusPlayers::new(42, 500)),
            description: Component::text("Benchmarking minestom-rs 26.2 server status payload")
                .color(NamedTextColor::Aqua)
                .bold(true),
            ..ServerStatus::default()
        },
    };
    let mut codec = FrameCodec::new(MAX_PACKET_SIZE);
    codec.set_compression_threshold(Some(64));

    bencher.bench_local(|| {
        let mut wire = BytesMut::with_capacity(512);
        codec.encode_packet(black_box(&pkt), &mut wire).unwrap();
        let frame = codec.try_decode_frame(&mut wire).unwrap().unwrap();
        black_box(decode_packet_exact::<StatusResponsePacket>(frame.payload).unwrap())
    });
}

#[divan::bench]
fn status_response_payload_encode(bencher: divan::Bencher) {
    let pkt = StatusResponsePacket {
        status: ServerStatus::default(),
    };
    bencher.bench_local(|| black_box(encode_packet_payload(black_box(&pkt)).unwrap()));
}
