//! Divan microbenchmarks for M2 world & event hot paths:
//! `GenerationUnit::fill_height`, `ChunkDataPacket` serialization vs `Bytes` packet cache,
//! and `EventNode` cached listener dispatch.

use std::hint::black_box;

use minestom::prelude::*;

fn main() {
    divan::main();
}

#[divan::bench]
fn flat_chunk_generate_and_serialize_first_time(bencher: divan::Bencher) {
    let dim = DimensionType::overworld();
    bencher.bench_local(|| {
        let mut chunk = Chunk::new(ChunkPos::new(0, 0), &dim);
        {
            let mut unit = GenerationUnit::new(&mut chunk);
            unit.modifier().fill_height(0, 40, Block::GRASS_BLOCK);
        }
        black_box(chunk.encoded_packet_bytes().unwrap())
    });
}

#[divan::bench]
fn cached_chunk_packet_clone_hit(bencher: divan::Bencher) {
    let dim = DimensionType::overworld();
    let mut chunk = Chunk::new(ChunkPos::new(0, 0), &dim);
    chunk.fill_height(0, 40, Block::GRASS_BLOCK);
    let _ = chunk.encoded_packet_bytes().unwrap();

    bencher.bench_local(|| black_box(chunk.encoded_packet_bytes().unwrap()));
}

#[divan::bench]
fn event_node_cached_dispatch(bencher: divan::Bencher) {
    struct TickCounter(u64);
    impl Event for TickCounter {}

    let root = EventNode::all("root");
    root.add_listener(|ev: &mut TickCounter| ev.0 = ev.0.wrapping_add(1));
    let child = EventNode::all("child").with_priority(10);
    child.add_listener(|ev: &mut TickCounter| ev.0 = ev.0.wrapping_add(2));
    root.add_child(child);

    // Warm compiled listener chain
    let mut ev = TickCounter(0);
    root.call(&mut ev);

    bencher.bench_local(|| {
        root.call(black_box(&mut ev));
        black_box(ev.0)
    });
}
