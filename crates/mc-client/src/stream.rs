//! Chunk streaming: a background thread owns the save, loads the chunks around the camera
//! nearest first, meshes each once its eight neighbours are loaded (so culling, light and
//! biome blending see across its edges), and drops chunks the camera has left behind.
//! The render thread uploads what it sends.

use glam::Vec3;
use mc_registry::Registry;
use mc_render::bake::BlockModels;
use mc_render::mesh::{Mesh, Mesher, WorldView};
use mc_render::tint::Biomes;
use std::collections::HashSet;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, TryRecvError};

pub enum Update {
    /// Draw this mesh for the chunk, replacing what it drew before.
    Mesh((i32, i32), Mesh),
    /// Stop drawing the chunk.
    Remove((i32, i32)),
    /// Everything in range is loaded and meshed; the thread waits for the camera to move.
    Idle,
}

pub struct Streamer {
    eye: Sender<Vec3>,
    updates: Receiver<Update>,
    last: (i32, i32),
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
    pub world: mc_world::World,
    pub view: WorldView,
    pub models: Arc<BlockModels>,
    pub biomes: Arc<Biomes>,
    pub registry: Arc<Registry>,
}

impl Streamer {
    /// Starts streaming the chunks within `distance` chunks of `eye` (vanilla's render
    /// distance, measured square).
    pub fn spawn(sources: Sources, distance: i32, eye: Vec3) -> Self {
        let (eye_tx, eye_rx) = std::sync::mpsc::channel();
        let (update_tx, updates) = std::sync::mpsc::channel();
        std::thread::Builder::new()
            .name("chunk streaming".into())
            .spawn(move || work(sources, distance, eye, &eye_rx, &update_tx))
            .expect("spawning the chunk streaming thread");
        Self {
            eye: eye_tx,
            updates,
            last: chunk_of(eye),
        }
    }

    /// Tells the thread where the camera is, when it has moved into another chunk.
    pub fn follow(&mut self, eye: Vec3) {
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
    distance: i32,
    mut eye: Vec3,
    eye_rx: &Receiver<Vec3>,
    tx: &Sender<Update>,
) {
    let Sources {
        mut world,
        mut view,
        models,
        biomes,
        registry,
    } = sources;
    let mesher = Mesher::new(&models, &biomes, &registry);
    // Chunks the save doesn't have, so they aren't looked for again.
    let mut missing: HashSet<(i32, i32)> = HashSet::new();
    // Chunks meshed (or found unmeshable) and not removed since.
    let mut meshed: HashSet<(i32, i32)> = HashSet::new();
    loop {
        loop {
            match eye_rx.try_recv() {
                Ok(e) => eye = e,
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return,
            }
        }
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
        let to_mesh = around(distance)
            .filter(|&k| !meshed.contains(&k) && view.contains(k.0, k.1))
            .filter(|&(x, z)| (-1..=1).all(|dx| (-1..=1).all(|dz| known((x + dx, z + dz)))))
            .min_by_key(|&k| square(k));
        let to_load = around(distance + 1)
            .filter(|&k| !known(k))
            .min_by_key(|&k| square(k));

        // Whichever is nearer, meshing on a tie.
        let mesh_first = match (to_mesh, to_load) {
            (Some(m), Some(l)) => square(m) <= square(l),
            (m, _) => m.is_some(),
        };
        match (to_mesh.filter(|_| mesh_first), to_load) {
            (Some(key), _) => {
                meshed.insert(key);
                // A chunk beside one the save never had can't be meshed; it stays blank.
                if let Some(mut mesh) = mesher.mesh_chunk(&view, key.0, key.1) {
                    mesh.sort_translucent(eye);
                    if tx.send(Update::Mesh(key, mesh)).is_err() {
                        return;
                    }
                }
            }
            (_, Some((x, z))) => match world.chunk(&registry, x, z) {
                Ok(Some(chunk)) => view.insert(chunk),
                Ok(None) => {
                    missing.insert((x, z));
                }
                Err(e) => {
                    eprintln!("chunk {x}, {z}: {e:#}");
                    missing.insert((x, z));
                }
            },
            (None, None) => {
                if tx.send(Update::Idle).is_err() {
                    return;
                }
                match eye_rx.recv() {
                    Ok(e) => eye = e,
                    Err(_) => return,
                }
            }
        }
    }
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
            world: mc_world::World::open(mc_assets::locate("fixtures/worlds/test-world")),
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
