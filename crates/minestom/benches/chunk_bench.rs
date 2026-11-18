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

#[divan::bench]
fn listener_handle_hot_dispatch(bencher: divan::Bencher) {
    struct FastTick(u64);
    impl Event for FastTick {}

    let root = EventNode::all("root");
    root.add_listener(|ev: &mut FastTick| ev.0 = ev.0.wrapping_add(1));
    let handle = root.listener_handle::<FastTick>();

    let mut ev = FastTick(0);
    handle.call(&mut ev);

    bencher.bench_local(|| {
        handle.call(black_box(&mut ev));
        black_box(ev.0)
    });
}

#[divan::bench]
fn command_manager_parse_and_execute(bencher: divan::Bencher) {
    let manager = CommandManager::new();
    let x_arg = Argument::integer("amount").between(1, 64);
    let x_ref = x_arg.clone();
    manager.register(
        Command::new("give").add_syntax([x_arg.untyped()], move |_, ctx| {
            black_box(*ctx.get(&x_ref).unwrap());
        }),
    );
    let mut sender = CommandSender::console();

    bencher.bench_local(|| {
        manager.execute(&mut sender, black_box("give 32")).unwrap();
    });
}

#[divan::bench]
fn spatial_entity_tracker_reconcile_1000_entities(bencher: divan::Bencher) {
    let mut tracker = EntityTracker::new();
    let inst = InstanceId::default();
    for i in 0..1000_i32 {
        let x = f64::from((i % 40) * 12);
        let z = f64::from((i / 40) * 12);
        tracker.register(i + 1, inst, Pos::new(x, 40.0, z));
    }
    let viewer = PlayerId::default();

    bencher.bench_local(|| {
        black_box(tracker.reconcile_viewer(viewer, 1, inst, black_box(ChunkPos::new(5, 5)), 4))
    });
}
