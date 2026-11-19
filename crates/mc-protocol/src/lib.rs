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
    /// The player has joined the world as this entity, in this game mode (`GameType`'s id:
    /// survival 0, creative 1, adventure 2, spectator 3), on a hardcore server or not.
    Login {
        entity_id: i32,
        hardcore: bool,
        game_mode: u8,
    },
    /// The player's game mode changed (`GameType`'s id), on respawning or by command.
    GameMode(u8),
    /// The player's health, food level and saturation.
    Health {
        health: f32,
        food: i32,
        saturation: f32,
    },
    /// The player's experience: progress through the level (0 to 1) and the level.
    Experience {
        progress: f32,
        level: i32,
    },
    /// What the player may do: fly (creative), and whether it is flying, at what speed;
    /// and break blocks instantly (creative).
    Abilities {
        flying: bool,
        may_fly: bool,
        instabuild: bool,
        flying_speed: f32,
    },
    /// The server selected this hotbar slot (0 to 8), such as after picking a block.
    HeldSlot(u8),
    /// Hotbar slots (0 to 8) now hold these items: name and count, or nothing.
    Hotbar(Vec<(u8, Option<(String, i32)>)>),
    /// Another player's mining of a block: their entity id, the block, and the crack stage
    /// (0 to 9), or `None` once they stop.
    Destruction {
        id: i32,
        pos: [i32; 3],
        stage: Option<u8>,
    },
    /// The server placed the player: feet position, yaw and pitch.
    Position {
        pos: [f64; 3],
        yaw: f32,
        pitch: f32,
    },
    Chunk(mc_world::Chunk),
    /// The chunk left the player's view.
    Forget(i32, i32),
    /// Blocks changed: position and new state.
    Blocks(Vec<([i32; 3], u32)>),
    /// New light for some sections of a chunk column: by section y, sky then block light.
    /// A section left out keeps its light.
    Light {
        x: i32,
        z: i32,
        sky: Vec<(i32, Box<[u8; 2048]>)>,
        block: Vec<(i32, Box<[u8; 2048]>)>,
    },
    /// The connection ended, with the server's reason.
    Disconnected(String),
}

/// A connection in the play state. Events arrive on `events` from a background thread,
/// which also answers keep-alives, pings, teleports and chunk batches; `outbox` sends the
/// game's own packets.
pub struct Session {
    pub events: Receiver<Event>,
    pub outbox: Outbox,
}

/// Sends play-state packets by name, from any thread.
#[derive(Clone)]
pub struct Outbox {
    writer: connection::Writer,
    ids: Arc<HashMap<String, i32>>,
}

impl Outbox {
    /// `name` without its namespace, such as `move_player_pos`.
    pub fn send(&self, name: &str, body: &[u8]) -> Result<()> {
        let id = self
            .ids
            .get(&format!("minecraft:{name}"))
            .with_context(|| format!("no serverbound {name} packet"))?;
        self.writer.send(*id, body)
    }
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
                let json = r.string()?;
                let component = serde_json::from_str(&json).unwrap_or(json.into());
                bail!("the server refused the login: {}", reason(&component))
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
    let outbox = Outbox {
        writer: conn.writer(),
        ids: Arc::new(
            packets
                .serverbound
                .iter()
                .filter(|((state, _), _)| *state == State::Play)
                .map(|((_, name), id)| (name.clone(), *id))
                .collect(),
        ),
    };
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
    Ok(Session { events, outbox })
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
            "disconnect" => bail!("the server disconnected: {}", nbt_reason(r.nbt()?)),
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
                let entity_id = r.i32()?;
                let hardcore = r.bool()?;
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
                tx.send(Event::Login {
                    entity_id,
                    hardcore,
                    game_mode: spawn_game_mode(&mut r)?,
                })
                .is_ok()
            }
            "respawn" => {
                // The same spawn info as the login's, from its dimension type on. Changing
                // dimension isn't supported yet, but the game mode can change too.
                r.varint()?;
                tx.send(Event::GameMode(spawn_game_mode(&mut r)?)).is_ok()
            }
            "game_event" => {
                // `ClientboundGameEventPacket.CHANGE_GAME_MODE`, its parameter rounded as
                // `handleGameEvent` does.
                let (event, param) = (r.u8()?, r.f32()?);
                event != 3
                    || tx
                        .send(Event::GameMode((param + 0.5).floor() as u8))
                        .is_ok()
            }
            "set_health" => tx
                .send(Event::Health {
                    health: r.f32()?,
                    food: r.varint()?,
                    saturation: r.f32()?,
                })
                .is_ok(),
            "set_experience" => {
                let progress = r.f32()?;
                tx.send(Event::Experience {
                    progress,
                    level: r.varint()?,
                })
                .is_ok()
            }
            "player_abilities" => {
                // Flags: invulnerable 1, flying 2, may fly 4, instant build 8.
                let flags = r.u8()?;
                tx.send(Event::Abilities {
                    flying: flags & 2 != 0,
                    may_fly: flags & 4 != 0,
                    instabuild: flags & 8 != 0,
                    flying_speed: r.f32()?,
                })
                .is_ok()
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
            "container_set_content" => {
                // The player's inventory menu is container 0: crafting result and grid (0 to
                // 4), armour (5 to 8), the inventory (9 to 35), then the hotbar (36 to 44).
                let container = r.varint()?;
                r.varint()?; // state id
                let mut slots = Vec::new();
                if container == 0 {
                    for slot in 0..r.count()? {
                        // A stack whose data this can't read ends the list early.
                        let Some(stack) = item_stack(&mut r, registry).ok() else {
                            break;
                        };
                        if (36..45).contains(&slot) {
                            slots.push(((slot - 36) as u8, stack));
                        }
                    }
                }
                tx.send(Event::Hotbar(slots)).is_ok()
            }
            "container_set_slot" => {
                let container = r.varint()?;
                r.varint()?; // state id
                let slot = r.i16()?;
                let changed = (container == 0 && (36..45).contains(&slot))
                    .then(|| {
                        Ok::<_, anyhow::Error>(((slot - 36) as u8, item_stack(&mut r, registry)?))
                    })
                    .transpose()?;
                tx.send(Event::Hotbar(changed.into_iter().collect()))
                    .is_ok()
            }
            "set_player_inventory" => {
                // An `Inventory` index, where the hotbar is 0 to 8.
                let slot = r.varint()?;
                let changed = (0..9)
                    .contains(&slot)
                    .then(|| Ok::<_, anyhow::Error>((slot as u8, item_stack(&mut r, registry)?)))
                    .transpose()?;
                tx.send(Event::Hotbar(changed.into_iter().collect()))
                    .is_ok()
            }
            "block_destruction" => {
                let id = r.varint()?;
                let pos = r.block_pos()?;
                let stage = r.u8()?;
                tx.send(Event::Destruction {
                    id,
                    pos,
                    stage: (stage < 10).then_some(stage),
                })
                .is_ok()
            }
            "set_held_slot" => tx
                .send(Event::HeldSlot(r.varint()?.clamp(0, 8) as u8))
                .is_ok(),
            "block_update" => {
                let pos = r.block_pos()?;
                tx.send(Event::Blocks(vec![(pos, r.count()? as u32)]))
                    .is_ok()
            }
            "section_blocks_update" => {
                // `SectionPos.asLong`: x in the top 22 bits, then z in 22, then y in 20.
                let section = r.i64()?;
                let origin = [
                    (section >> 42) as i32 * 16,
                    (section << 44 >> 44) as i32 * 16,
                    (section << 22 >> 42) as i32 * 16,
                ];
                // Each entry: the state id above 12 bits of x, z and y in the section.
                let changes = (0..r.count()?)
                    .map(|_| {
                        let v = r.varlong()?;
                        let at = |shift: i64| (v >> shift & 15) as i32;
                        let pos = [origin[0] + at(8), origin[1] + at(0), origin[2] + at(4)];
                        Ok((pos, (v >> 12) as u32))
                    })
                    .collect::<Result<_>>()?;
                tx.send(Event::Blocks(changes)).is_ok()
            }
            "light_update" => {
                let dimension = dimension
                    .as_ref()
                    .context("light arrived before the login packet")?;
                let (x, z) = (r.varint()?, r.varint()?);
                let light = chunk::light(&mut r, dimension.light_sections())?;
                let by_y = |arrays: Vec<(usize, Box<[u8; 2048]>)>| {
                    arrays
                        .into_iter()
                        .map(|(i, a)| (dimension.light_section_y(i), a))
                        .collect()
                };
                tx.send(Event::Light {
                    x,
                    z,
                    sky: by_y(light.sky),
                    block: by_y(light.block),
                })
                .is_ok()
            }
            "forget_level_chunk" => {
                // `ChunkPos.pack`: x in the low 32 bits, z in the high.
                let packed = r.i64()?;
                tx.send(Event::Forget(packed as i32, (packed >> 32) as i32))
                    .is_ok()
            }
            "disconnect" => return Ok(nbt_reason(r.nbt()?)),
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

/// The game mode from a `CommonPlayerSpawnInfo`, read from just after its dimension type:
/// the dimension's name and the hashed seed come first.
fn spawn_game_mode(r: &mut Reader) -> Result<u8> {
    r.string()?;
    r.i64()?;
    Ok(r.varint()? as u8)
}

/// An `ItemStack.OPTIONAL_STREAM_CODEC` stack: its item and count, or `None` for an empty
/// slot. Its data components follow without lengths, so each must be read to reach the
/// next stack; the common ones are, and any other fails the read.
fn item_stack(r: &mut Reader, registry: &Registry) -> Result<Option<(String, i32)>> {
    let count = r.varint()?;
    if count <= 0 {
        return Ok(None);
    }
    let item = r.count()? as u32;
    let name = registry
        .item_name(item)
        .with_context(|| format!("unknown item {item}"))?
        .to_owned();
    let (added, removed) = (r.count()?, r.count()?);
    for _ in 0..added {
        let kind = r.count()? as u32;
        let kind = registry.component_type(kind).unwrap_or_default();
        match kind.strip_prefix("minecraft:").unwrap_or(kind) {
            "max_damage" | "damage" | "repair_cost" => {
                r.varint()?;
            }
            "unbreakable" => {}
            "enchantment_glint_override" => {
                r.bool()?;
            }
            "custom_name" | "item_name" => {
                r.nbt()?;
            }
            "lore" => {
                for _ in 0..r.count()? {
                    r.nbt()?;
                }
            }
            "enchantments" | "stored_enchantments" => {
                for _ in 0..r.count()? {
                    r.varint()?; // enchantment
                    r.varint()?; // level
                }
            }
            other => bail!("can't read item component {other}"),
        }
    }
    for _ in 0..removed {
        r.varint()?;
    }
    Ok(Some((name, count)))
}

/// A disconnect reason as a player reads it: the chat component in English, plus how to
/// fix it when the fix is on the player's side.
fn reason(component: &serde_json::Value) -> String {
    let lang = std::fs::read(mc_assets::locate(
        "resources/assets/minecraft/lang/en_us.json",
    ))
    .ok()
    .and_then(|bytes| serde_json::from_slice(&bytes).ok())
    .unwrap_or_default();
    let text = component_text(component, &lang);
    let hint = match component["translate"].as_str().unwrap_or_default() {
        "multiplayer.disconnect.not_whitelisted" => {
            "Run `whitelist off` in the server console, or `whitelist add <name>` for this name."
        }
        "multiplayer.disconnect.outdated_client" | "multiplayer.disconnect.outdated_server" => {
            "This client speaks Minecraft 26.3; the server needs to run the same version."
        }
        "multiplayer.disconnect.name_taken" | "multiplayer.disconnect.duplicate_login" => {
            "Someone is already playing with this name: pick another with --name."
        }
        "multiplayer.disconnect.banned" | "multiplayer.disconnect.banned.reason" => {
            "Run `pardon <name>` in the server console to lift the ban."
        }
        _ => "",
    };
    if hint.is_empty() {
        text
    } else {
        format!("{text}\n{hint}")
    }
}

/// The text of a chat component (JSON form, which NBT components convert to): `text`, or
/// `translate` looked up in `lang` with its `%s` / `%1$s` slots filled from `with`, then
/// each of `extra`.
fn component_text(
    c: &serde_json::Value,
    lang: &serde_json::Map<String, serde_json::Value>,
) -> String {
    use serde_json::Value;
    let mut out = match c {
        Value::String(s) => return s.clone(),
        Value::Array(parts) => return parts.iter().map(|p| component_text(p, lang)).collect(),
        Value::Object(_) => String::new(),
        other => return other.to_string(),
    };
    if let Some(text) = c["text"].as_str() {
        out.push_str(text);
    } else if let Some(key) = c["translate"].as_str() {
        let args: Vec<String> = c["with"]
            .as_array()
            .map(|with| with.iter().map(|a| component_text(a, lang)).collect())
            .unwrap_or_default();
        let format = lang.get(key).and_then(Value::as_str).unwrap_or(key);
        out.push_str(&fill(format, &args));
    }
    if let Some(extra) = c["extra"].as_array() {
        out.extend(extra.iter().map(|e| component_text(e, lang)));
    }
    out
}

/// Fills a translation's `%s` (next argument), `%N$s` (argument N) and `%%` slots, as
/// vanilla's `TranslatableContents` does. A missing argument fills nothing.
fn fill(format: &str, args: &[String]) -> String {
    let mut out = String::new();
    let mut next = 0;
    let mut rest = format;
    while let Some(at) = rest.find('%') {
        out.push_str(&rest[..at]);
        rest = &rest[at + 1..];
        if let Some(after) = rest.strip_prefix('%') {
            out.push('%');
            rest = after;
        } else if let Some(after) = rest.strip_prefix('s') {
            out.push_str(args.get(next).map_or("", String::as_str));
            next += 1;
            rest = after;
        } else if let Some((n, after)) = rest
            .split_once("$s")
            .filter(|(n, _)| n.parse::<usize>().is_ok())
        {
            let n: usize = n.parse().unwrap();
            out.push_str(args.get(n.wrapping_sub(1)).map_or("", String::as_str));
            rest = after;
        } else {
            out.push('%');
        }
    }
    out.push_str(rest);
    out
}

fn nbt_reason(tag: Option<mc_nbt::Tag>) -> String {
    reason(&tag.as_ref().map_or(serde_json::Value::Null, nbt_json))
}

/// A chat component sent as NBT, in the JSON form `component_text` reads.
fn nbt_json(tag: &mc_nbt::Tag) -> serde_json::Value {
    use mc_nbt::Tag;
    match tag {
        Tag::String(s) => s.clone().into(),
        Tag::Compound(c) => c.iter().map(|(k, v)| (k.clone(), nbt_json(v))).collect(),
        Tag::List(l) => l.items.iter().map(nbt_json).collect(),
        Tag::Byte(n) => (*n).into(),
        Tag::Short(n) => (*n).into(),
        Tag::Int(n) => (*n).into(),
        Tag::Long(n) => (*n).into(),
        Tag::Float(n) => (*n).into(),
        Tag::Double(n) => (*n).into(),
        other => format!("{other:?}").into(),
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

    /// Translated reasons fill `%s` in order and `%N$s` by number, append `extra`, and
    /// read the same from NBT; the whitelist kick says how to fix it.
    #[test]
    fn disconnect_reasons_read_like_vanilla() {
        let lang = serde_json::json!({ "k": "%2$s then %s, %s %%", "inner": "<%s>" });
        let lang = lang.as_object().unwrap();
        let c = serde_json::json!({
            "translate": "k",
            "with": ["a", { "translate": "inner", "with": [7] }],
            "extra": [{ "text": "!" }]
        });
        assert_eq!(component_text(&c, lang), "<7> then a, <7> %!");
        assert_eq!(
            component_text(&nbt_json(&mc_nbt::Tag::String("x".into())), lang),
            "x"
        );

        let kick = mc_nbt::Tag::Compound(
            [(
                "translate".to_owned(),
                mc_nbt::Tag::String("multiplayer.disconnect.not_whitelisted".into()),
            )]
            .into_iter()
            .collect(),
        );
        assert_eq!(
            nbt_reason(Some(kick)),
            "You are not white-listed on this server!\nRun `whitelist off` in the server console, or `whitelist add <name>` for this name."
        );
    }
}
