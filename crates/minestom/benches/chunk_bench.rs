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

#[divan::bench]
fn block_with_property_mixed_radix(bencher: divan::Bencher) {
    let base = Block::OAK_STAIRS;
    bencher.bench_local(|| {
        let b = black_box(base)
            .with_property("facing", "east")
            .unwrap()
            .with_property("half", "top")
            .unwrap();
        black_box(b.get_property("facing"))
    });
}

#[divan::bench]
fn relative_batch_apply_125_blocks(bencher: divan::Bencher) {
    let mgr = InstanceManager::new();
    let inst = mgr.create_instance_container();
    let mut cube = RelativeBlockBatch::new();
    for dx in 0..5 {
        for dy in 0..5 {
            for dz in 0..5 {
                cube.set_relative(dx, dy, dz, Block::STONE);
            }
        }
    }
    let abs = cube.to_absolute(BlockVec::new(0, 40, 0));

    bencher.bench_local(|| black_box(abs.apply(&inst)));
}

#[divan::bench]
fn bfs_chunk_lighting_sky_and_block(bencher: divan::Bencher) {
    let dim = DimensionType::overworld();
    let mut chunk = Chunk::new(ChunkPos::new(0, 0), &dim);
    chunk.fill_height(0, 40, Block::STONE);
    chunk.fill_height(50, 51, Block::GLASS);
    chunk.set_block(8, 42, 8, Block::GLOWSTONE);

    bencher.bench_local(|| black_box(compute_chunk_light(black_box(&chunk))));
}

#[divan::bench]
fn anvil_chunk_nbt_roundtrip(bencher: divan::Bencher) {
    let dim = DimensionType::overworld();
    let pos = ChunkPos::new(0, 0);
    let mut chunk = Chunk::new(pos, &dim);
    chunk.fill_height(0, 40, Block::STONE);
    chunk.set_block(4, 41, 4, Block::OAK_LOG.with_property("axis", "x").unwrap());
    chunk.set_block(5, 41, 5, Block::CHEST);

    bencher.bench_local(|| {
        let nbt = minestom::world::encode_chunk_to_anvil_nbt(black_box(&chunk), 1000);
        black_box(minestom::world::decode_chunk_from_anvil_nbt(
            pos, &dim, &nbt,
        ))
    });
}

#[divan::bench]
fn scheduler_tick_phase_100_tasks(bencher: divan::Bencher) {
    let sched = Scheduler::new();
    for _ in 0..100 {
        let _ = sched
            .build_task(|| {
                black_box(1_u64);
            })
            .repeat(TaskSchedule::Tick(1))
            .schedule();
    }
    sched.process_tick_start();

    bencher.bench_local(|| {
        sched.process_tick_start();
    });
}

#[divan::bench]
fn astar_pathfinding_around_obstacle_wall(bencher: divan::Bencher) {
    let mgr = InstanceManager::new();
    let inst = mgr.create_instance_container();
    inst.set_generator(|unit: &mut GenerationUnit| {
        unit.modifier().fill_height(0, 40, Block::STONE);
    });
    let _ = inst.chunk_packet_payloads(&[ChunkPos::new(0, 0), ChunkPos::new(1, 0)]);
    for z in -3..=3 {
        inst.set_block(BlockVec::new(5, 40, z), Block::COBBLESTONE);
        inst.set_block(BlockVec::new(5, 41, z), Block::COBBLESTONE);
    }

    bencher.bench_local(|| {
        black_box(find_path(
            black_box(&inst),
            BlockVec::new(1, 40, 0),
            BlockVec::new(10, 40, 0),
        ))
    });
}
