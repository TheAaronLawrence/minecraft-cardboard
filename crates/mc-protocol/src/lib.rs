//! The Java Edition network protocol, as vanilla 26.3 speaks it (protocol 777): framing and
//! compression, the handshake, login and configuration states, and the play packets a client
//! needs to stand in a world and receive its chunks.
//!
//! Packet ids come from the data generator's `packets.json` report in `resources/reports/`,
//! so they always match the version the reports were made from. Only offline-mode servers
//! are supported so far: online mode needs a Microsoft login and encryption.

mod chunk;
pub mod codec;
mod connection;

use anyhow::{Context, Result, bail};
use codec::{Put, Reader};
use connection::Connection;
use mc_registry::Registry;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender};

/// What the server tells the game, in the order it arrives.
pub enum Event {
    /// The server placed the player: feet position, yaw and pitch.
    Position {
        pos: [f64; 3],
        yaw: f32,
        pitch: f32,
    },
    Chunk(mc_world::Chunk),
    /// The chunk left the player's view.
    Forget(i32, i32),
    /// The connection ended, with the server's reason.
    Disconnected(String),
}

/// A connection in the play state. Events arrive on `events` from a background thread,
/// which also answers keep-alives, pings, teleports and chunk batches.
pub struct Session {
    pub events: Receiver<Event>,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum State {
    Handshake,
    Login,
    Configuration,
    Play,
}

impl State {
    fn report_name(self) -> &'static str {
        match self {
            State::Handshake => "handshake",
            State::Login => "login",
            State::Configuration => "configuration",
            State::Play => "play",
        }
    }
}

/// Packet ids by state and name, both ways.
struct Packets {
    serverbound: HashMap<(State, String), i32>,
    clientbound: HashMap<(State, i32), String>,
}

impl Packets {
    fn load() -> Result<Self> {
        let path = mc_assets::locate("resources/reports/packets.json");
        let report: serde_json::Value = serde_json::from_slice(
            &std::fs::read(&path).with_context(|| format!("reading {}", path.display()))?,
        )?;
        let mut packets = Self {
            serverbound: HashMap::new(),
            clientbound: HashMap::new(),
        };
        for state in [
            State::Handshake,
            State::Login,
            State::Configuration,
            State::Play,
        ] {
            let ids = |direction: &str| {
                report[state.report_name()][direction]
                    .as_object()
                    .into_iter()
                    .flatten()
                    .filter_map(|(name, v)| Some((name.clone(), v["protocol_id"].as_i64()? as i32)))
            };
            for (name, id) in ids("serverbound") {
                packets.serverbound.insert((state, name), id);
            }
            for (name, id) in ids("clientbound") {
                packets.clientbound.insert((state, id), name);
            }
        }
        Ok(packets)
    }

    fn id(&self, state: State, name: &str) -> Result<i32> {
        self.serverbound
            .get(&(state, format!("minecraft:{name}")))
            .copied()
            .with_context(|| format!("no serverbound {name} packet in {state:?}"))
    }

    fn name(&self, state: State, id: i32) -> &str {
        self.clientbound
            .get(&(state, id))
            .map_or("unknown", |n| n.strip_prefix("minecraft:").unwrap_or(n))
    }
}

/// The protocol version from `resources/reports/source.json`, which `gen-reports` records.
fn protocol_version() -> Result<i32> {
    let path = mc_assets::locate("resources/reports/source.json");
    let source: serde_json::Value = serde_json::from_slice(&std::fs::read(&path)?)?;
    source["protocol_version"]
        .as_i64()
        .map(|v| v as i32)
        .context("source.json has no protocol_version; run `cargo xtask gen-reports`")
}

/// The UUID an offline-mode server gives a name: `UUID.nameUUIDFromBytes` (MD5, version 3)
/// of `OfflinePlayer:<name>`.
pub fn offline_uuid(name: &str) -> u128 {
    use md5::Digest;
    let mut hash: [u8; 16] = md5::Md5::digest(format!("OfflinePlayer:{name}")).into();
    hash[6] = hash[6] & 0x0F | 0x30;
    hash[8] = hash[8] & 0x3F | 0x80;
    u128::from_be_bytes(hash)
}

/// What the client tells the server about itself (`ClientInformation`).
fn client_information(view_distance: u8) -> Vec<u8> {
    let mut body = Vec::new();
    body.put_string("en_us")
        .put_u8(view_distance)
        .put_varint(0) // chat visible
        .put_bool(true) // chat colours
        .put_u8(0x7F) // every skin layer
        .put_varint(1) // right-handed
        .put_bool(false) // text filtering
        .put_bool(true) // listed in the server list
        .put_varint(0); // all particles
    body
}

/// Joins an offline-mode server at `address` (`host:port`) as `name`, and returns once the
/// connection reaches the play state. `view_distance` is in chunks.
pub fn connect(
    address: &str,
    name: &str,
    view_distance: u8,
    registry: Arc<Registry>,
) -> Result<Session> {
    let packets = Packets::load()?;
    let (host, port) = address.rsplit_once(':').unwrap_or((address, "25565"));
    let port: u16 = port.parse().context("port is not a number")?;
    let mut conn = Connection::open(&format!("{host}:{port}"))?;

    let mut body = Vec::new();
    body.put_varint(protocol_version()?)
        .put_string(host)
        .put_u16(port)
        .put_varint(2); // log in
    conn.send(packets.id(State::Handshake, "intention")?, &body)?;

    let mut body = Vec::new();
    body.put_string(name).put_uuid(offline_uuid(name));
    conn.send(packets.id(State::Login, "hello")?, &body)?;
    loop {
        let (id, body) = conn.recv()?;
        let mut r = Reader::new(&body);
        match packets.name(State::Login, id) {
            "login_compression" => conn.compress(r.varint()?),
            "login_finished" => break,
            "login_disconnect" => {
                bail!("the server refused the login: {}", text_json(&r.string()?))
            }
            "hello" => bail!("the server is in online mode, which needs a Microsoft login"),
            "custom_query" => {
                // Nothing here understands login plugins: answer each with no payload.
                let mut answer = Vec::new();
                answer.put_varint(r.varint()?).put_bool(false);
                conn.send(packets.id(State::Login, "custom_query_answer")?, &answer)?;
            }
            other => eprintln!("login: ignoring {other}"),
        }
    }
    conn.send(packets.id(State::Login, "login_acknowledged")?, &[])?;

    let config = configure(&mut conn, &packets, view_distance)?;
    let (tx, events) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("server connection".into())
        .spawn(move || {
            let result = play(&mut conn, &packets, &config, &registry, &tx);
            let reason = match result {
                Ok(reason) => reason,
                Err(e) => format!("{e:#}"),
            };
            let _ = tx.send(Event::Disconnected(reason));
        })?;
    Ok(Session { events })
}

/// What configuration leaves the play state needing: each synced registry's entries in id
/// order, with the inline data the server sent for any it didn't expect us to know.
struct Config {
    registries: HashMap<String, Vec<(String, Option<mc_nbt::Tag>)>>,
}

impl Config {
    fn entries(&self, registry: &str) -> &[(String, Option<mc_nbt::Tag>)] {
        self.registries.get(registry).map_or(&[], Vec::as_slice)
    }
}

fn configure(conn: &mut Connection, packets: &Packets, view_distance: u8) -> Result<Config> {
    const STATE: State = State::Configuration;
    conn.send(
        packets.id(STATE, "client_information")?,
        &client_information(view_distance),
    )?;
    let mut config = Config {
        registries: HashMap::new(),
    };
    loop {
        let (id, body) = conn.recv()?;
        let mut r = Reader::new(&body);
        match packets.name(STATE, id) {
            "select_known_packs" => {
                // We ship vanilla's data, so we know every pack the server lists: echo them
                // back and the server leaves their registry contents out.
                let count = r.count()?;
                let mut reply = Vec::new();
                reply.put_varint(count as i32);
                for _ in 0..count {
                    for _ in 0..3 {
                        reply.put_string(&r.string()?);
                    }
                }
                conn.send(packets.id(STATE, "select_known_packs")?, &reply)?;
            }
            "registry_data" => {
                let registry = r.string()?;
                let entries = (0..r.count()?)
                    .map(|_| {
                        let name = r.string()?;
                        let data = if r.bool()? { r.nbt()? } else { None };
                        Ok((name, data))
                    })
                    .collect::<Result<_>>()?;
                config.registries.insert(registry, entries);
            }
            "keep_alive" => conn.send(packets.id(STATE, "keep_alive")?, r.take(8)?)?,
            "ping" => conn.send(packets.id(STATE, "pong")?, r.take(4)?)?,
            "code_of_conduct" => conn.send(packets.id(STATE, "accept_code_of_conduct")?, &[])?,
            "disconnect" => bail!("the server disconnected: {}", text_nbt(r.nbt()?)),
            "finish_configuration" => {
                conn.send(packets.id(STATE, "finish_configuration")?, &[])?;
                return Ok(config);
            }
            // Tags, features, server links and the like don't affect drawing the world yet.
            _ => {}
        }
    }
}

/// The play state, until the server disconnects us; returns its reason.
fn play(
    conn: &mut Connection,
    packets: &Packets,
    config: &Config,
    registry: &Registry,
    tx: &Sender<Event>,
) -> Result<String> {
    const STATE: State = State::Play;
    let biomes: Vec<String> = config
        .entries("minecraft:worldgen/biome")
        .iter()
        .map(|(name, _)| name.clone())
        .collect();
    let block_entity_names = |id: u32| registry.block_entity_type(id).map(str::to_owned);
    let mut dimension: Option<chunk::Dimension> = None;
    let mut position = ([0.0f64; 3], 0.0f32, 0.0f32);
    let mut loaded = false;
    loop {
        let (id, body) = conn.recv()?;
        let mut r = Reader::new(&body);
        let send = |conn: &mut Connection, name: &str, body: &[u8]| -> Result<()> {
            conn.send(packets.id(STATE, name)?, body)
        };
        let delivered = match packets.name(STATE, id) {
            "keep_alive" => {
                send(conn, "keep_alive", r.take(8)?)?;
                true
            }
            "ping" => {
                send(conn, "pong", r.take(4)?)?;
                true
            }
            "login" => {
                // Entity id, hardcore, the level names, max players, view and simulation
                // distances, and three flags come before the spawn info's dimension type.
                r.i32()?;
                r.bool()?;
                for _ in 0..r.count()? {
                    r.string()?;
                }
                for _ in 0..3 {
                    r.varint()?;
                }
                for _ in 0..3 {
                    r.bool()?;
                }
                let index = r.count()?;
                let (name, data) = config
                    .entries("minecraft:dimension_type")
                    .get(index)
                    .context("the login's dimension type isn't in the registry data")?;
                dimension = Some(chunk::Dimension::new(name, data.as_ref())?);
                true
            }
            "player_position" => {
                let teleport = r.varint()?;
                let [x, y, z] = [r.f64()?, r.f64()?, r.f64()?];
                for _ in 0..3 {
                    r.f64()?; // velocity
                }
                let (yaw, pitch) = (r.f32()?, r.f32()?);
                // `Relative`: which of x, y, z, yaw and pitch are offsets from where we are.
                let relative = r.i32()?;
                let pick = |bit: i32, value: f64, current: f64| {
                    if relative & 1 << bit != 0 {
                        current + value
                    } else {
                        value
                    }
                };
                let pos = [
                    pick(0, x, position.0[0]),
                    pick(1, y, position.0[1]),
                    pick(2, z, position.0[2]),
                ];
                let yaw = pick(3, yaw as f64, position.1 as f64) as f32;
                let pitch = pick(4, pitch as f64, position.2 as f64) as f32;
                position = (pos, yaw, pitch);

                let mut accept = Vec::new();
                accept.put_varint(teleport);
                for c in pos {
                    accept.put_f64(c);
                }
                accept.put_f32(yaw).put_f32(pitch);
                // Since 26.3 the acceptance carries the position, and no movement packet
                // follows: the server kicks a client that sends two positions in one tick.
                send(conn, "accept_teleportation", &accept)?;
                if !loaded {
                    // Until the client says it has loaded, the server keeps the player
                    // frozen and invulnerable.
                    send(conn, "player_loaded", &[])?;
                    loaded = true;
                }
                tx.send(Event::Position { pos, yaw, pitch }).is_ok()
            }
            "chunk_batch_finished" => {
                // How many chunks a tick we can take; the server's upper limit.
                let mut reply = Vec::new();
                reply.put_f32(64.0);
                send(conn, "chunk_batch_received", &reply)?;
                true
            }
            "level_chunk_with_light" => {
                let dimension = dimension
                    .as_ref()
                    .context("a chunk arrived before the login packet")?;
                let chunk =
                    chunk::decode(&mut r, dimension, registry, &biomes, &block_entity_names)?;
                tx.send(Event::Chunk(chunk)).is_ok()
            }
            "forget_level_chunk" => {
                // `ChunkPos.pack`: x in the low 32 bits, z in the high.
                let packed = r.i64()?;
                tx.send(Event::Forget(packed as i32, (packed >> 32) as i32))
                    .is_ok()
            }
            "disconnect" => return Ok(text_nbt(r.nbt()?)),
            "start_configuration" => {
                return Ok("the server asked to reconfigure, which isn't supported yet".into());
            }
            _ => true,
        };
        if !delivered {
            return Ok("the game closed the connection".into());
        }
    }
}

/// A chat component's text, from its JSON form (login disconnects send it so).
fn text_json(json: &str) -> String {
    match serde_json::from_str::<serde_json::Value>(json) {
        Ok(serde_json::Value::String(s)) => s,
        Ok(v) => v["text"]
            .as_str()
            .or(v["translate"].as_str())
            .map_or_else(|| json.to_owned(), str::to_owned),
        Err(_) => json.to_owned(),
    }
}

/// A chat component's text, from its NBT form: a plain string, or a compound's `text` or
/// `translate` key.
fn text_nbt(tag: Option<mc_nbt::Tag>) -> String {
    match tag {
        Some(mc_nbt::Tag::String(s)) => s,
        Some(mc_nbt::Tag::Compound(c)) => ["text", "translate"]
            .iter()
            .find_map(|k| c.get(*k).and_then(mc_nbt::Tag::as_str))
            .map_or_else(|| format!("{c:?}"), str::to_owned),
        other => format!("{other:?}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Offline UUIDs as vanilla's `UUIDUtil.createOfflinePlayerUUID` makes them.
    #[test]
    fn offline_uuid_matches_vanilla() {
        assert_eq!(
            format!("{:032x}", offline_uuid("Notch")),
            "b50ad385829d3141a2167e7d7539ba7f"
        );
    }
}
