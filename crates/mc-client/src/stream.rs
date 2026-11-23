//! Chunk streaming: a background thread owns the chunks' source, loads the chunks around the
//! camera nearest first, meshes each once its eight neighbours are loaded (so culling, light
//! and biome blending see across its edges), and drops chunks the camera has left behind.
//! The render thread uploads what it sends. From a save the thread reads chunks itself; from
//! a server they arrive as the server sends them.

use glam::Vec3;
use mc_registry::Registry;
use mc_render::bake::BlockModels;
use mc_render::mesh::{Mesh, Mesher, WorldView};
use mc_render::tint::Biomes;
use std::collections::HashSet;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, TryRecvError};

/// A section's biomes: its y, its palette of biome names, and each 4×4×4 cell's index
/// into the palette.
pub type SectionBiomes = (i32, Vec<String>, [u8; 64]);

/// A chunk's blocks by section y, and its biomes.
pub type ChunkData = (Vec<(i32, mc_world::Blocks)>, Vec<SectionBiomes>);

pub enum Update {
    /// Draw this mesh for the chunk, replacing what it drew before.
    Mesh((i32, i32), Mesh),
    /// Stop drawing the chunk.
    Remove((i32, i32)),
    /// Everything in range is loaded and meshed; the thread waits for the camera to move.
    Idle,
    /// The server placed the player; the game moves the player there and accepts it.
    Teleport(mc_protocol::Teleport),
    /// The server connection ended, for this reason.
    Disconnected(String),
    /// A server chunk's blocks by section y, for collision, and its biomes; `None` when
    /// it's dropped.
    Collision((i32, i32), Option<ChunkData>),
    /// The server changed these blocks: position and new state.
    Blocks(Vec<([i32; 3], u32)>),
    /// A server chunk's light, for lighting entities: sky and block light by section y.
    /// `whole` replaces the chunk's light; otherwise only the sections and layers given
    /// change.
    Light {
        chunk: (i32, i32),
        sections: Vec<(i32, crate::walk::SectionLight)>,
        whole: bool,
    },
    /// Another server event for the game: the login, or the player's abilities.
    Server(mc_protocol::Event),
}

/// Where chunks come from.
pub enum Source {
    Save(mc_world::World),
    Server(Receiver<mc_protocol::Event>),
}

pub struct Streamer {
    eye: Sender<Vec3>,
    updates: Receiver<Update>,
    last: (i32, i32),
    last_eye: Vec3,
    /// The render distance in chunks, which the thread reads as it goes.
    distance: Arc<std::sync::atomic::AtomicI32>,
}

/// The chunk a position is in.
pub fn chunk_of(p: Vec3) -> (i32, i32) {
    (
        (p.x.floor() as i32).div_euclid(16),
        (p.z.floor() as i32).div_euclid(16),
    )
}

/// What the streaming thread works with. `view` may already hold chunks.
pub struct Sources {
    pub source: Source,
    pub view: WorldView,
    pub models: Arc<BlockModels>,
    pub biomes: Arc<Biomes>,
    pub registry: Arc<Registry>,
}

impl Streamer {
    /// Starts streaming the chunks within `distance` chunks of `eye` (vanilla's render
    /// distance, measured square).
    pub fn spawn(sources: Sources, distance: i32, eye: Vec3) -> Self {
        let distance = Arc::new(std::sync::atomic::AtomicI32::new(distance));
        let shared = distance.clone();
        let (eye_tx, eye_rx) = std::sync::mpsc::channel();
        let (update_tx, updates) = std::sync::mpsc::channel();
        std::thread::Builder::new()
            .name("chunk streaming".into())
            .spawn(move || work(sources, &distance, eye, &eye_rx, &update_tx))
            .expect("spawning the chunk streaming thread");
        Self {
            eye: eye_tx,
            updates,
            last: chunk_of(eye),
            last_eye: eye,
            distance: shared,
        }
    }

    /// The Render Distance option changed: chunks load and go for the new distance.
    pub fn set_distance(&mut self, distance: i32) {
        self.distance
            .store(distance, std::sync::atomic::Ordering::Relaxed);
        // Wakes the thread, which waits for the camera to move on a save.
        let _ = self.eye.send(self.last_eye);
    }

    /// Tells the thread where the camera is, when it has moved into another chunk.
    pub fn follow(&mut self, eye: Vec3) {
        self.last_eye = eye;
        let chunk = chunk_of(eye);
        if chunk != self.last {
            self.last = chunk;
            let _ = self.eye.send(eye);
        }
    }

    /// What has changed since the last call.
    pub fn updates(&self) -> impl Iterator<Item = Update> + '_ {
        self.updates.try_iter()
    }
}

fn work(
    sources: Sources,
    distance: &std::sync::atomic::AtomicI32,
    mut eye: Vec3,
    eye_rx: &Receiver<Vec3>,
    tx: &Sender<Update>,
) {
    let Sources {
        mut source,
        mut view,
        models,
        biomes,
        registry,
    } = sources;
    let mesher = Mesher::new(&models, &biomes, &registry);
    // Only server chunks change after they load; a save's light is already right.
    let lighting = match &source {
        Source::Server(_) => match crate::light::Lighting::new(&registry) {
            Ok(lighting) => Some(lighting),
            Err(e) => {
                eprintln!("relighting changed blocks is off: {e:#}");
                None
            }
        },
        Source::Save(_) => None,
    };
    // Chunks the save doesn't have, so they aren't looked for again.
    let mut missing: HashSet<(i32, i32)> = HashSet::new();
    // Chunks meshed (or found unmeshable) and not removed since.
    let mut meshed: HashSet<(i32, i32)> = HashSet::new();
    // Meshed chunks whose blocks or light have changed since, to mesh again.
    let mut dirty: HashSet<(i32, i32)> = HashSet::new();
    let mut idle = false;
    loop {
        if let Source::Server(events) = &source {
            for event in events.try_iter() {
                idle = false;
                if !receive(
                    event,
                    &mut view,
                    lighting.as_ref(),
                    &mut meshed,
                    &mut dirty,
                    tx,
                ) {
                    return;
                }
            }
        }
        loop {
            match eye_rx.try_recv() {
                Ok(e) => eye = e,
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return,
            }
        }
        for [x, _, z] in view.take_shut_lids(std::time::Instant::now()) {
            let key = (x.div_euclid(16), z.div_euclid(16));
            if meshed.contains(&key) {
                dirty.insert(key);
                idle = false;
            }
        }
        let distance = distance.load(std::sync::atomic::Ordering::Relaxed);
        let center = chunk_of(eye);
        let ring = |(x, z): (i32, i32)| (x - center.0).abs().max((z - center.1).abs());
        let square = |(x, z): (i32, i32)| (x - center.0).pow(2) + (z - center.1).pow(2);

        // Drawn chunks go once out of range; loaded ones are kept a ring further, as
        // neighbours of the outermost drawn chunks, and a ring more so a camera moving back
        // and forth across a chunk border doesn't reload them.
        let gone: Vec<_> = meshed
            .iter()
            .copied()
            .filter(|&k| ring(k) > distance)
            .collect();
        for key in gone {
            meshed.remove(&key);
            dirty.remove(&key);
            if tx.send(Update::Remove(key)).is_err() {
                return;
            }
        }
        let far: Vec<_> = view
            .chunk_positions()
            .filter(|&k| ring(k) > distance + 2)
            .collect();
        for (x, z) in far {
            view.remove(x, z);
        }
        missing.retain(|&k| ring(k) <= distance + 2);

        let around = |r: i32| {
            (center.0 - r..=center.0 + r)
                .flat_map(move |x| (center.1 - r..=center.1 + r).map(move |z| (x, z)))
        };
        let known = |k: (i32, i32)| view.contains(k.0, k.1) || missing.contains(&k);
        // A server sends the chunks; only a save is read here.
        let reads = matches!(source, Source::Save(_));
        let to_mesh = around(distance)
            .filter(|&k| (!meshed.contains(&k) || dirty.contains(&k)) && view.contains(k.0, k.1))
            .filter(|&(x, z)| (-1..=1).all(|dx| (-1..=1).all(|dz| known((x + dx, z + dz)))))
            .min_by_key(|&k| square(k));
        let to_load = around(distance + 1)
            .filter(|&k| reads && !known(k))
            .min_by_key(|&k| square(k));

        // Whichever is nearer, meshing on a tie.
        let mesh_first = match (to_mesh, to_load) {
            (Some(m), Some(l)) => square(m) <= square(l),
            (m, _) => m.is_some(),
        };
        match (to_mesh.filter(|_| mesh_first), to_load) {
            (Some(key), _) => {
                idle = false;
                meshed.insert(key);
                dirty.remove(&key);
                // A chunk beside one the save never had can't be meshed; it stays blank.
                if let Some(mut mesh) = mesher.mesh_chunk(&view, key.0, key.1) {
                    mesh.sort_translucent(eye);
                    if tx.send(Update::Mesh(key, mesh)).is_err() {
                        return;
                    }
                }
            }
            (_, Some((x, z))) => match &mut source {
                Source::Save(world) => match world.chunk(&registry, x, z) {
                    Ok(Some(chunk)) => view.insert(chunk),
                    Ok(None) => {
                        missing.insert((x, z));
                    }
                    Err(e) => {
                        eprintln!("chunk {x}, {z}: {e:#}");
                        missing.insert((x, z));
                    }
                },
                Source::Server(_) => unreachable!("server chunks aren't read"),
            },
            (None, None) => {
                if !idle {
                    idle = true;
                    if tx.send(Update::Idle).is_err() {
                        return;
                    }
                }
                match &source {
                    Source::Save(_) => match eye_rx.recv() {
                        Ok(e) => eye = e,
                        Err(_) => return,
                    },
                    // Wait for the server's next packet or the camera, whichever comes.
                    Source::Server(events) => {
                        match events.recv_timeout(std::time::Duration::from_millis(50)) {
                            Ok(event) => {
                                idle = false;
                                if !receive(
                                    event,
                                    &mut view,
                                    lighting.as_ref(),
                                    &mut meshed,
                                    &mut dirty,
                                    tx,
                                ) {
                                    return;
                                }
                            }
                            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                                // The connection is over; keep drawing what we have.
                                source = Source::Server(std::sync::mpsc::channel().1);
                                match eye_rx.recv() {
                                    Ok(e) => eye = e,
                                    Err(_) => return,
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Applies one server event; false once the renderer has gone.
/// How long after a chest starts to close its chunk has its lid again: the lid takes half
/// a second to shut, and the game draws it for a second after (`Walker::chest_lids`), so
/// the chunk's lid is back before the game's goes.
const LID_SHUT: std::time::Duration = std::time::Duration::from_millis(750);

fn receive(
    event: mc_protocol::Event,
    view: &mut WorldView,
    lighting: Option<&crate::light::Lighting>,
    meshed: &mut HashSet<(i32, i32)>,
    dirty: &mut HashSet<(i32, i32)>,
    tx: &Sender<Update>,
) -> bool {
    // A chunk is meshed with its neighbours' edge blocks and light, so a change in one
    // re-meshes those around it too.
    let mut touch =
        |cx: i32, cz: i32, dx: std::ops::RangeInclusive<i32>, dz: std::ops::RangeInclusive<i32>| {
            for x in dx {
                for z in dz.clone() {
                    if meshed.contains(&(cx + x, cz + z)) {
                        dirty.insert((cx + x, cz + z));
                    }
                }
            }
        };
    use mc_protocol::Event;
    let update = match event {
        Event::Chunk(chunk) => {
            let blocks = chunk
                .sections
                .iter()
                .map(|s| (s.y, s.blocks.clone()))
                .collect();
            let biomes = chunk
                .sections
                .iter()
                .map(|s| (s.y, s.biome_palette.clone(), s.biomes))
                .collect();
            let light = chunk
                .sections
                .iter()
                .map(|s| (s.y, [s.sky_light.clone(), s.block_light.clone()]))
                .collect();
            // A chunk sent again replaces the old one and is meshed afresh.
            meshed.remove(&(chunk.x, chunk.z));
            let key = (chunk.x, chunk.z);
            let light = Update::Light {
                chunk: key,
                sections: light,
                whole: true,
            };
            if tx.send(light).is_err() {
                return false;
            }
            view.insert(chunk);
            return tx
                .send(Update::Collision(key, Some((blocks, biomes))))
                .is_ok();
        }
        Event::Blocks(changes) => {
            // Blocks whose change can change the light, to relight as the vanilla client
            // does: the server sends new light only to players at the edge of its view.
            let mut relight = Vec::new();
            for &([x, y, z], state) in &changes {
                let old = view.state(x, y, z);
                if view.set_block([x, y, z], state) {
                    if lighting.is_some_and(|l| l.differs(old, state)) {
                        relight.push([x, y, z]);
                    }
                    // Only blocks on a chunk's edge show in the next chunk over.
                    let near = |c: i32| match c.rem_euclid(16) {
                        0 => -1..=0,
                        15 => 0..=1,
                        _ => 0..=0,
                    };
                    touch(x.div_euclid(16), z.div_euclid(16), near(x), near(z));
                }
            }
            if let Some(lighting) = lighting.filter(|_| !relight.is_empty()) {
                let mut columns: std::collections::BTreeMap<(i32, i32), Vec<i32>> =
                    Default::default();
                for (cx, section_y, cz) in lighting.relight(view, &relight) {
                    columns.entry((cx, cz)).or_default().push(section_y);
                }
                for ((cx, cz), section_ys) in columns {
                    // Light at a chunk's edge shades the faces next to it, and smooth
                    // lighting reaches one block further.
                    touch(cx, cz, -1..=1, -1..=1);
                    let Some(chunk) = view.column(cx, cz) else {
                        continue;
                    };
                    let sections = section_ys
                        .into_iter()
                        .filter_map(|y| chunk.section(y))
                        .map(|s| (s.y, [s.sky_light.clone(), s.block_light.clone()]))
                        .collect();
                    let light = Update::Light {
                        chunk: (cx, cz),
                        sections,
                        whole: false,
                    };
                    if tx.send(light).is_err() {
                        return false;
                    }
                }
            }
            Update::Blocks(changes)
        }
        Event::Light { x, z, sky, block } => {
            let mut sections = Vec::new();
            for (light, is_sky) in [(sky, true), (block, false)] {
                for (section_y, array) in light {
                    let mut layers = [None, None];
                    layers[usize::from(!is_sky)] = Some(array.clone());
                    sections.push((section_y, layers));
                    view.set_light((x, z), section_y, is_sky, array);
                }
            }
            touch(x, z, -1..=1, -1..=1);
            let light = Update::Light {
                chunk: (x, z),
                sections,
                whole: false,
            };
            if tx.send(light).is_err() {
                return false;
            }
            return true;
        }
        Event::Forget(x, z) => {
            view.remove(x, z);
            dirty.remove(&(x, z));
            if tx.send(Update::Collision((x, z), None)).is_err() {
                return false;
            }
            if !meshed.remove(&(x, z)) {
                return true;
            }
            Update::Remove((x, z))
        }
        // A chest's lid opening or closing: its chunk is meshed without it while the game
        // draws it, until a while after it's shut.
        Event::BlockEvent {
            pos,
            kind: 1,
            value,
            ref block,
        } if block.ends_with("chest") => {
            let shut = std::time::Instant::now() + LID_SHUT;
            if view.set_lid(pos, value > 0, shut) {
                touch(pos[0].div_euclid(16), pos[2].div_euclid(16), 0..=0, 0..=0);
            }
            Update::Server(event)
        }
        Event::Position(teleport) => Update::Teleport(teleport),
        Event::Disconnected(reason) => Update::Disconnected(reason),
        other => Update::Server(other),
    };
    tx.send(update).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    /// Collects updates until the thread goes idle, applying them to the set of drawn
    /// chunks as the renderer would.
    fn settle(streamer: &Streamer, drawn: &mut BTreeSet<(i32, i32)>) {
        loop {
            match streamer
                .updates
                .recv_timeout(std::time::Duration::from_secs(60))
                .expect("the streaming thread went quiet without going idle")
            {
                Update::Mesh(key, _) => {
                    drawn.insert(key);
                }
                Update::Remove(key) => {
                    assert!(drawn.remove(&key), "removed {key:?}, which wasn't drawn");
                }
                Update::Idle => return,
                _ => {}
            }
        }
    }

    fn square(
        x: std::ops::RangeInclusive<i32>,
        z: std::ops::RangeInclusive<i32>,
    ) -> BTreeSet<(i32, i32)> {
        x.flat_map(|x| z.clone().map(move |z| (x, z))).collect()
    }

    /// Flying two chunks east draws the two new columns and drops the two left behind,
    /// and nothing else changes; flying back draws the dropped ones again.
    #[test]
    fn chunks_follow_the_camera() {
        let res = mc_assets::Resources::new(mc_assets::default_root());
        let registry = Arc::new(Registry::vanilla().unwrap());
        let models = Arc::new(BlockModels::bake(&res, &registry, &[]).unwrap());
        let biomes = Arc::new(Biomes::load(&res, Vec::<String>::new()).unwrap());
        let sources = Sources {
            source: Source::Save(mc_world::World::open(mc_assets::locate(
                "fixtures/worlds/test-world",
            ))),
            view: WorldView::empty(&registry),
            models,
            biomes,
            registry,
        };
        let mut streamer = Streamer::spawn(sources, 2, Vec3::new(8.0, 100.0, 8.0));
        let mut drawn = BTreeSet::new();
        settle(&streamer, &mut drawn);
        assert_eq!(drawn, square(-2..=2, -2..=2));

        streamer.follow(Vec3::new(2.0 * 16.0 + 8.0, 100.0, 8.0));
        settle(&streamer, &mut drawn);
        assert_eq!(drawn, square(0..=4, -2..=2));

        // Flying back draws the dropped chunks again.
        streamer.follow(Vec3::new(8.0, 100.0, 8.0));
        settle(&streamer, &mut drawn);
        assert_eq!(drawn, square(-2..=2, -2..=2));
    }
}
