//! The Java Edition network protocol, as vanilla 26.3 speaks it (protocol 777): framing and
//! compression, the handshake, login and configuration states, and the play packets a client
//! needs to stand in a world, receive its chunks, and chat.
//!
//! Packet ids come from the data generator's `packets.json` report in `resources/reports/`,
//! so they always match the version the reports were made from. Only offline-mode servers
//! are supported so far: online mode needs a Microsoft login and encryption.

pub mod chat;
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
    /// survival 0, creative 1, adventure 2, spectator 3), on a hardcore server or not, and
    /// whether dying shows the death screen (the `immediate_respawn` game rule is off).
    Login {
        entity_id: i32,
        hardcore: bool,
        game_mode: u8,
        show_death_screen: bool,
    },
    /// The player's game mode changed (`GameType`'s id) by command.
    GameMode(u8),
    /// The player respawned, in this game mode.
    Respawn {
        game_mode: u8,
    },
    /// The `immediate_respawn` game rule changed: whether dying shows the death screen.
    ShowDeathScreen(bool),
    /// A player died: their entity id and the death message, in English.
    Died {
        entity_id: i32,
        message: String,
    },
    /// The player's score (`Player.DATA_SCORE_ID`), which the death screen shows.
    Score(i32),
    /// An explosion pushed the player by this much velocity, in blocks a tick
    /// (`ClientboundExplodePacket.playerKnockback`).
    ExplosionPush([f64; 3]),
    /// Something happened to an entity (`ClientboundEntityEventPacket`): its id, and the
    /// event's id, such as 60 for a mob's death poof.
    EntityEvent {
        id: i32,
        event: i8,
    },
    /// An entity was hurt (`ClientboundDamageEventPacket`); only its id is read.
    Damaged(i32),
    /// An entity was hurt from a direction (`ClientboundHurtAnimationPacket`): its id, and
    /// the direction relative to its yaw, in degrees.
    HurtAnimation {
        id: i32,
        yaw: f32,
    },
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
    /// Slots of the player's inventory menu (`InventoryMenu`: 0 the crafting result, 1 to 4
    /// the grid, 5 to 8 armour, 9 to 35 the inventory, 36 to 44 the hotbar, 45 the
    /// offhand) now hold these items: name and count, or nothing. With the state id the
    /// server numbers the menu's changes by, when the packet has one.
    Inventory {
        state_id: Option<i32>,
        slots: Vec<(u8, Option<(String, i32)>)>,
    },
    /// The item held on the mouse cursor in an open inventory.
    Carried(Option<(String, i32)>),
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
    /// A message for the chat: from the server, or a player's, decorated by its chat type.
    Chat(Vec<chat::Span>),
    /// Another entity came into view (`ClientboundAddEntityPacket`): its id, UUID, type (an
    /// `entity_type` registry id), feet position, velocity in blocks a tick, rotations in
    /// degrees, and the type's own data.
    AddEntity {
        id: i32,
        uuid: u128,
        kind: u32,
        pos: [f64; 3],
        velocity: [f64; 3],
        yaw: f32,
        pitch: f32,
        head_yaw: f32,
        data: i32,
    },
    /// These entities left view.
    RemoveEntities(Vec<i32>),
    /// An entity moved by an offset from where the client last placed it, turned, or both
    /// (`ClientboundMoveEntityPacket`).
    MoveEntity {
        id: i32,
        delta: Option<Delta>,
        rotation: Option<(f32, f32)>,
        on_ground: bool,
    },
    /// An entity's exact position, which also resets the base later offsets count from
    /// (`ClientboundEntityPositionSyncPacket`).
    SyncEntity {
        id: i32,
        path: Path,
        yaw: f32,
        pitch: f32,
        on_ground: bool,
    },
    /// An entity was teleported (`ClientboundTeleportEntityPacket`). `relative` is a
    /// `Relative` bit set: which of x, y, z, yaw, pitch and the velocity's parts are
    /// offsets from what the entity has.
    TeleportEntity {
        id: i32,
        pos: [f64; 3],
        velocity: [f64; 3],
        yaw: f32,
        pitch: f32,
        relative: i32,
        on_ground: bool,
    },
    /// An entity turned its head to this yaw.
    HeadRotation {
        id: i32,
        yaw: f32,
    },
    /// An entity's velocity, in blocks a tick.
    EntityMotion {
        id: i32,
        velocity: [f64; 3],
    },
    /// Players joined the tab list (`ClientboundPlayerInfoUpdatePacket.Action.ADD_PLAYER`):
    /// their profiles.
    AddPlayers(Vec<Profile>),
    /// Players left the tab list, by UUID.
    RemovePlayers(Vec<u128>),
    /// What an entity holds and wears (`ClientboundSetEquipmentPacket`): `EquipmentSlot`
    /// ids (main hand 0, off hand 1, feet 2, legs 3, chest 4, head 5, body 6, saddle 7)
    /// and what's in each now.
    Equipment {
        id: i32,
        slots: Vec<(u8, Option<Equipped>)>,
    },
    /// An entity swings an arm (`ClientboundSwingAnimationPacket`): with its main hand (0) or
    /// off hand (1), as a `SwingAnimationType` (none 0, whack 1, stab 2), lasting so many
    /// ticks.
    Swing {
        id: i32,
        hand: u8,
        kind: u8,
        duration: i32,
    },
    /// Some of an entity's synced data (`SynchedEntityData`), by index.
    EntityData {
        id: i32,
        values: Vec<(u8, DataValue)>,
    },
    /// The connection ended, with the server's reason.
    Disconnected(String),
}

/// A player's `GameProfile` as the server sends it: UUID, name, and the `textures`
/// property with its signature, if the server sent them.
#[derive(Clone, Debug, PartialEq)]
pub struct Profile {
    pub uuid: u128,
    pub name: String,
    pub textures: Option<Textures>,
}

/// A profile's `textures` property: base64 JSON naming the skin's URL, and Mojang's
/// base64 signature of it.
#[derive(Clone, Debug, PartialEq)]
pub struct Textures {
    pub value: String,
    pub signature: Option<String>,
}

/// One synced entity data value, of the serializers the game reads.
#[derive(Clone, Debug, PartialEq)]
pub enum DataValue {
    Byte(u8),
    Int(i32),
    Float(f32),
    Bool(bool),
    /// An item stack: its item and count, or nothing.
    Item(Option<(String, i32)>),
    /// A cow, pig or chicken variant, by name, such as `minecraft:warm`.
    Variant(String),
}

/// An item an entity holds or wears: its name, and its `dyed_color` if it has one.
#[derive(Clone, Debug, PartialEq)]
pub struct Equipped {
    pub item: String,
    pub dye: Option<u32>,
}

/// A `PositionPath`: where an entity moves to, straight there, or through positions each
/// reached so many ticks after the last.
#[derive(Clone, Debug, PartialEq)]
pub enum Path {
    Linear([f64; 3]),
    Stepped(Vec<([f64; 3], i32)>),
}

impl Path {
    /// `PositionPath.endPosition`.
    pub fn end(&self) -> [f64; 3] {
        match self {
            Path::Linear(p) => *p,
            Path::Stepped(steps) => steps.last().map_or([0.0; 3], |s| s.0),
        }
    }
}

/// A `VecDelta`: an entity's move in 1/4096 blocks from the base position the client keeps
/// for it (`VecDeltaCodec`), as one offset, or as steps each a number of ticks long.
#[derive(Clone, Debug, PartialEq)]
pub enum Delta {
    Linear([i16; 3]),
    Stepped(Vec<(i32, [i16; 3])>),
}

impl Delta {
    /// `VecDelta.decode`: each step counts from the one before; the path's end becomes the
    /// next base.
    pub fn decode(&self, base: [f64; 3]) -> Path {
        match self {
            Delta::Linear(d) => Path::Linear(decode_delta(base, *d)),
            Delta::Stepped(steps) if steps.is_empty() => Path::Linear(base),
            Delta::Stepped(steps) => {
                let mut base = base;
                Path::Stepped(
                    steps
                        .iter()
                        .map(|&(ticks, d)| {
                            base = decode_delta(base, d);
                            (base, ticks)
                        })
                        .collect(),
                )
            }
        }
    }

    /// `VecDelta.read` with `ClientboundMoveEntityPacket`'s step count.
    fn read(r: &mut Reader, steps: u32) -> Result<Self> {
        let offset = |r: &mut Reader| Ok::<_, anyhow::Error>([r.i16()?, r.i16()?, r.i16()?]);
        if steps == 0 {
            return Ok(Delta::Linear(offset(r)?));
        }
        // Each step is at least 7 bytes, which bounds a hostile count.
        anyhow::ensure!(steps as usize <= r.remaining() / 7, "too many steps");
        (0..steps)
            .map(|_| Ok((r.varint()?, offset(r)?)))
            .collect::<Result<_>>()
            .map(Delta::Stepped)
    }
}

/// `VecDeltaCodec.decode`: a part that doesn't change keeps the base exactly; one that does
/// is the base rounded to 1/4096 plus the offset.
fn decode_delta(base: [f64; 3], delta: [i16; 3]) -> [f64; 3] {
    std::array::from_fn(|i| match delta[i] {
        0 => base[i],
        // `Math.round`, which rounds halves up.
        d => ((base[i] * 4096.0 + 0.5).floor() + d as f64) / 4096.0,
    })
}

/// `LpVec3.read`: a velocity packed as three 15-bit fractions of a shared scale, or one zero
/// byte for none.
fn lp_vec3(r: &mut Reader) -> Result<[f64; 3]> {
    let lowest = r.u8()? as u64;
    if lowest == 0 {
        return Ok([0.0; 3]);
    }
    let middle = r.u8()? as u64;
    let highest = r.i32()? as u32 as u64;
    let buffer = highest << 16 | middle << 8 | lowest;
    let mut scale = lowest & 3;
    if lowest & 4 != 0 {
        scale |= (r.varint()? as u32 as u64) << 2;
    }
    let unpack = |v: u64| ((v & 32767) as f64).min(32766.0) * 2.0 / 32766.0 - 1.0;
    Ok([3, 18, 33].map(|shift| unpack(buffer >> shift) * scale as f64))
}

/// `Mth.unpackDegrees`: a byte angle, 256 to a turn.
fn angle(r: &mut Reader) -> Result<f32> {
    Ok(r.i8()? as f32 * 360.0 / 256.0)
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
    /// Signed messages received and not yet acknowledged to the server
    /// (`LastSeenMessagesTracker.offset`).
    unacknowledged: Arc<std::sync::atomic::AtomicI32>,
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

    /// `ClientPacketListener.sendChat` for an unsigned message, acknowledging the signed
    /// messages seen since the last acknowledgement.
    pub fn send_chat(&self, message: &str) -> Result<()> {
        use std::sync::atomic::Ordering;
        let offset = self.unacknowledged.swap(0, Ordering::SeqCst);
        self.send("chat", &chat::chat_packet(message, offset)?)
    }

    /// `ClientPacketListener.sendCommand`, unsigned: `command` without its `/`.
    pub fn send_command(&self, command: &str) -> Result<()> {
        let mut body = Vec::new();
        body.put_string(command);
        self.send("chat_command", &body)
    }

    /// Leaves the server: closes the connection, as quitting does.
    pub fn close(&self) {
        self.writer.close();
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
        unacknowledged: Default::default(),
    };
    let unacknowledged = outbox.unacknowledged.clone();
    let (tx, events) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("server connection".into())
        .spawn(move || {
            let result = play(
                &mut conn,
                &packets,
                &config,
                &registry,
                &tx,
                &unacknowledged,
            );
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
    unacknowledged: &std::sync::atomic::AtomicI32,
) -> Result<String> {
    const STATE: State = State::Play;
    let chat_types = chat::chat_types(config.entries("minecraft:chat_type"));
    let lang = english();
    let biomes: Vec<String> = config
        .entries("minecraft:worldgen/biome")
        .iter()
        .map(|(name, _)| name.clone())
        .collect();
    let block_entity_names = |id: u32| registry.block_entity_type(id).map(str::to_owned);
    let mut dimension: Option<chunk::Dimension> = None;
    let mut position = ([0.0f64; 3], 0.0f32, 0.0f32);
    let mut loaded = false;
    let mut own_id = i32::MIN;
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
                // Reduced debug info, whether to show the death screen, limited crafting.
                let show_death_screen = [r.bool()?, r.bool()?, r.bool()?][1];
                let index = r.count()?;
                let (name, data) = config
                    .entries("minecraft:dimension_type")
                    .get(index)
                    .context("the login's dimension type isn't in the registry data")?;
                dimension = Some(chunk::Dimension::new(name, data.as_ref())?);
                own_id = entity_id;
                tx.send(Event::Login {
                    entity_id,
                    hardcore,
                    game_mode: spawn_game_mode(&mut r)?,
                    show_death_screen,
                })
                .is_ok()
            }
            "respawn" => {
                // The same spawn info as the login's, from its dimension type on. Changing
                // dimension isn't supported yet, but the game mode can change too.
                r.varint()?;
                // The server waits for the client to load again before the player can move.
                loaded = false;
                tx.send(Event::Respawn {
                    game_mode: spawn_game_mode(&mut r)?,
                })
                .is_ok()
            }
            "game_event" => {
                // `ClientboundGameEventPacket.CHANGE_GAME_MODE`, its parameter rounded as
                // `handleGameEvent` does; and `IMMEDIATE_RESPAWN`, where 0 shows the death
                // screen.
                let (event, param) = (r.u8()?, r.f32()?);
                match event {
                    3 => tx
                        .send(Event::GameMode((param + 0.5).floor() as u8))
                        .is_ok(),
                    11 => tx.send(Event::ShowDeathScreen(param == 0.0)).is_ok(),
                    _ => true,
                }
            }
            "player_combat_kill" => {
                let entity_id = r.varint()?;
                tx.send(Event::Died {
                    entity_id,
                    message: message_text(r.nbt()?),
                })
                .is_ok()
            }
            "swing_animation" => tx
                .send(Event::Swing {
                    id: r.varint()?,
                    hand: r.varint()? as u8,
                    kind: r.varint()? as u8,
                    duration: r.varint()?,
                })
                .is_ok(),
            "set_equipment" => {
                let id = r.varint()?;
                let mut slots = Vec::new();
                // A stack with a component this can't read ends the packet there; the
                // slots before it still count.
                loop {
                    let slot = r.u8()?;
                    let Ok(stack) = item_stack_dyed(&mut r, registry) else {
                        break;
                    };
                    slots.push((
                        slot & 0x7f,
                        stack.map(|(item, _, dye)| Equipped { item, dye }),
                    ));
                    if slot & 0x80 == 0 {
                        break;
                    }
                }
                tx.send(Event::Equipment { id, slots }).is_ok()
            }
            "set_entity_data" => {
                let id = r.varint()?;
                let values = entity_data(&mut r, registry, config);
                // `Player.DATA_SCORE_ID`: index 18 (after `Entity`'s 8, `LivingEntity`'s 7,
                // `Avatar`'s 2 and `Player`'s absorption).
                let score = match values.iter().find(|(i, _)| *i == 18) {
                    Some((_, DataValue::Int(score))) if id == own_id => Some(*score),
                    _ => None,
                };
                score.is_none_or(|score| tx.send(Event::Score(score)).is_ok())
                    && tx.send(Event::EntityData { id, values }).is_ok()
            }
            "add_entity" => {
                let id = r.varint()?;
                let uuid = u128::from_be_bytes(r.take(16)?.try_into()?);
                let kind = r.count()? as u32;
                let pos = [r.f64()?, r.f64()?, r.f64()?];
                let velocity = lp_vec3(&mut r)?;
                let (pitch, yaw, head_yaw) = (angle(&mut r)?, angle(&mut r)?, angle(&mut r)?);
                id == own_id
                    || tx
                        .send(Event::AddEntity {
                            id,
                            uuid,
                            kind,
                            pos,
                            velocity,
                            yaw,
                            pitch,
                            head_yaw,
                            data: r.varint()?,
                        })
                        .is_ok()
            }
            "player_info_update" => {
                let added = player_info_update(&mut r)?;
                added.is_empty() || tx.send(Event::AddPlayers(added)).is_ok()
            }
            "player_info_remove" => {
                let ids = (0..r.count()?)
                    .map(|_| Ok(u128::from_be_bytes(r.take(16)?.try_into()?)))
                    .collect::<Result<_>>()?;
                tx.send(Event::RemovePlayers(ids)).is_ok()
            }
            "entity_event" => {
                let id = r.i32()?;
                tx.send(Event::EntityEvent { id, event: r.i8()? }).is_ok()
            }
            "remove_entities" => {
                let ids = (0..r.count()?).map(|_| r.varint()).collect::<Result<_>>()?;
                tx.send(Event::RemoveEntities(ids)).is_ok()
            }
            name @ ("move_entity_pos" | "move_entity_pos_rot" | "move_entity_rot") => {
                let id = r.varint()?;
                // Position moves pack on-ground with the step count; a turn has a flag.
                let (delta, on_ground) = if name == "move_entity_rot" {
                    (None, r.bool()?)
                } else {
                    let properties = r.varint()? as u32;
                    (
                        Some(Delta::read(&mut r, properties >> 1)?),
                        properties & 1 != 0,
                    )
                };
                let rotation = if name == "move_entity_pos" {
                    None
                } else {
                    let yaw = angle(&mut r)?;
                    Some((yaw, angle(&mut r)?))
                };
                tx.send(Event::MoveEntity {
                    id,
                    delta,
                    rotation,
                    on_ground,
                })
                .is_ok()
            }
            "entity_position_sync" => {
                let id = r.varint()?;
                let vec3 = |r: &mut Reader| Ok::<_, anyhow::Error>([r.f64()?, r.f64()?, r.f64()?]);
                // `PositionPath.Type`: linear 0, stepped 1.
                let path = match r.varint()? {
                    1 => Path::Stepped(
                        (0..r.count()?)
                            .map(|_| Ok((vec3(&mut r)?, r.varint()?)))
                            .collect::<Result<_>>()?,
                    ),
                    _ => Path::Linear(vec3(&mut r)?),
                };
                tx.send(Event::SyncEntity {
                    id,
                    path,
                    yaw: r.f32()?,
                    pitch: r.f32()?,
                    on_ground: r.bool()?,
                })
                .is_ok()
            }
            "teleport_entity" => {
                let id = r.varint()?;
                let pos = [r.f64()?, r.f64()?, r.f64()?];
                let velocity = [r.f64()?, r.f64()?, r.f64()?];
                tx.send(Event::TeleportEntity {
                    id,
                    pos,
                    velocity,
                    yaw: r.f32()?,
                    pitch: r.f32()?,
                    relative: r.i32()?,
                    on_ground: r.bool()?,
                })
                .is_ok()
            }
            "rotate_head" => {
                let id = r.varint()?;
                tx.send(Event::HeadRotation {
                    id,
                    yaw: angle(&mut r)?,
                })
                .is_ok()
            }
            "set_entity_motion" => {
                let id = r.varint()?;
                tx.send(Event::EntityMotion {
                    id,
                    velocity: lp_vec3(&mut r)?,
                })
                .is_ok()
            }
            "damage_event" => tx.send(Event::Damaged(r.varint()?)).is_ok(),
            // Its centre, radius and block count, then the push; its particles and sound
            // after that aren't read.
            "explode" => {
                r.take(8 * 3 + 4 + 4)?;
                if r.bool()? {
                    let push = [r.f64()?, r.f64()?, r.f64()?];
                    tx.send(Event::ExplosionPush(push)).is_ok()
                } else {
                    true
                }
            }
            "hurt_animation" => {
                let id = r.varint()?;
                tx.send(Event::HurtAnimation { id, yaw: r.f32()? }).is_ok()
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
                // Only the player's inventory menu, container 0, is open in this game.
                let container = r.varint()?;
                let state_id = r.varint()?;
                if container != 0 {
                    continue;
                }
                let mut slots = Vec::new();
                let mut whole = true;
                for slot in 0..r.count()? {
                    // A stack whose data this can't read ends the list early.
                    let Some(stack) = item_stack(&mut r, registry).ok() else {
                        whole = false;
                        break;
                    };
                    slots.push((slot as u8, stack));
                }
                let sent = tx
                    .send(Event::Inventory {
                        state_id: Some(state_id),
                        slots,
                    })
                    .is_ok();
                match whole.then(|| item_stack(&mut r, registry)) {
                    Some(Ok(carried)) => sent && tx.send(Event::Carried(carried)).is_ok(),
                    _ => sent,
                }
            }
            "container_set_slot" => {
                let container = r.varint()?;
                let state_id = r.varint()?;
                let slot = r.i16()?;
                if container != 0 || !(0..46).contains(&slot) {
                    continue;
                }
                tx.send(Event::Inventory {
                    state_id: Some(state_id),
                    slots: vec![(slot as u8, item_stack(&mut r, registry)?)],
                })
                .is_ok()
            }
            "set_cursor_item" => tx
                .send(Event::Carried(item_stack(&mut r, registry)?))
                .is_ok(),
            "set_player_inventory" => {
                // An `Inventory` index: the hotbar 0 to 8, the rest 9 to 35, armour from
                // the feet up 36 to 39 and the offhand 40, as the menu's slots hold them.
                let slot = r.varint()?;
                let menu_slot = match slot {
                    0..9 => slot + 36,
                    9..36 => slot,
                    36..40 => 44 - slot,
                    40 => 45,
                    _ => continue,
                };
                tx.send(Event::Inventory {
                    state_id: None,
                    slots: vec![(menu_slot as u8, item_stack(&mut r, registry)?)],
                })
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
            "system_chat" => {
                let message = chat::component(r.nbt()?);
                // An overlay message goes above the hotbar, which isn't drawn yet.
                r.bool()? || tx.send(Event::Chat(chat::spans(&message, &lang))).is_ok()
            }
            "disguised_chat" => {
                let content = chat::component(r.nbt()?);
                match chat_bound(&mut r, &chat_types)? {
                    Some((ty, name, target)) => {
                        let message = ty.decorate(name, target, content);
                        tx.send(Event::Chat(chat::spans(&message, &lang))).is_ok()
                    }
                    None => true,
                }
            }
            "player_chat" => {
                // `ClientboundPlayerChatPacket`: indices, the sender, then the message.
                r.varint()?;
                r.take(16)?;
                r.varint()?;
                let signed = r.bool()?;
                if signed {
                    r.take(256)?;
                }
                let content = r.string()?;
                r.i64()?; // time stamp
                r.i64()?; // salt
                for _ in 0..r.count()? {
                    // `MessageSignature.Packed`: a cached id plus one, or 0 and the signature.
                    if r.varint()? == 0 {
                        r.take(256)?;
                    }
                }
                let unsigned = if r.bool()? { r.nbt()? } else { None };
                // `FilterMask`: pass through, fully filtered, or a bit set of what's hidden.
                let filter = r.varint()?;
                if filter == 2 {
                    r.long_array()?;
                }
                // `ClientPacketListener.markMessageAsProcessed`: every 64 signed messages
                // seen, acknowledge them.
                if signed {
                    use std::sync::atomic::Ordering;
                    if unacknowledged.fetch_add(1, Ordering::SeqCst) + 1 > 64 {
                        let offset = unacknowledged.swap(0, Ordering::SeqCst);
                        let mut ack = Vec::new();
                        ack.put_varint(offset);
                        send(conn, "chat_ack", &ack)?;
                    }
                }
                let content = match unsigned {
                    Some(tag) => chat::component(Some(tag)),
                    None => content.into(),
                };
                match chat_bound(&mut r, &chat_types)? {
                    Some((ty, name, target)) if filter != 1 => {
                        let message = ty.decorate(name, target, content);
                        tx.send(Event::Chat(chat::spans(&message, &lang))).is_ok()
                    }
                    _ => true,
                }
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

/// A `ChatType.Bound`: the chat type, the sender's name, and the target's if any. A chat
/// type sent inline rather than by registry id gives `None`.
fn chat_bound<'a>(
    r: &mut Reader,
    types: &'a [Option<chat::ChatType>],
) -> Result<
    Option<(
        &'a chat::ChatType,
        serde_json::Value,
        Option<serde_json::Value>,
    )>,
> {
    let id = r.varint()?;
    if id == 0 {
        return Ok(None);
    }
    let name = chat::component(r.nbt()?);
    let target = if r.bool()? {
        Some(chat::component(r.nbt()?))
    } else {
        None
    };
    Ok(types
        .get(id as usize - 1)
        .and_then(Option::as_ref)
        .map(|ty| (ty, name, target)))
}

/// A `ClientboundSetEntityDataPacket`'s values: each is its index, its
/// `EntityDataSerializers` id and the value, until index 255. Values follow without lengths,
/// so a serializer this can't read (or skip) ends the list there.
fn entity_data(r: &mut Reader, registry: &Registry, config: &Config) -> Vec<(u8, DataValue)> {
    let mut values = Vec::new();
    let mut read = || -> Result<()> {
        loop {
            let index = r.u8()?;
            if index == 0xff {
                return Ok(());
            }
            let value = match r.varint()? {
                0 => DataValue::Byte(r.u8()?),
                // Ints, directions, optional ints, poses, the sound variants, sniffer,
                // armadillo and copper states, arms and dye colours.
                1 | 12 | 19 | 20 | 24 | 29 | 31 | 35..=38 | 42 | 43 => DataValue::Int(r.varint()?),
                2 => DataValue::Int(r.varlong()? as i32),
                3 => DataValue::Float(r.f32()?),
                4 => {
                    r.string()?;
                    continue;
                }
                5 => {
                    r.nbt()?;
                    continue;
                }
                6 => {
                    if r.bool()? {
                        r.nbt()?;
                    }
                    continue;
                }
                7 => DataValue::Item(item_stack(r, registry)?),
                8 => DataValue::Bool(r.bool()?),
                9 | 39 => {
                    r.take(12)?;
                    continue;
                }
                10 => {
                    r.i64()?;
                    continue;
                }
                11 => {
                    if r.bool()? {
                        r.i64()?;
                    }
                    continue;
                }
                // An optional entity reference: a UUID.
                13 => {
                    if r.bool()? {
                        r.take(16)?;
                    }
                    continue;
                }
                14 | 15 => DataValue::Int(r.varint()?),
                // An empty particle list, as most entities have.
                17 if r.count()? == 0 => continue,
                // A villager's type, profession and level.
                18 => {
                    for _ in 0..3 {
                        r.varint()?;
                    }
                    continue;
                }
                // Cow, pig and chicken variants: ids in their synced registries.
                serializer @ (23 | 28 | 30) => {
                    let registry = match serializer {
                        23 => "minecraft:cow_variant",
                        28 => "minecraft:pig_variant",
                        _ => "minecraft:chicken_variant",
                    };
                    let id = r.count()?;
                    match config.entries(registry).get(id) {
                        Some((name, _)) => DataValue::Variant(name.clone()),
                        None => continue,
                    }
                }
                40 => {
                    r.take(16)?;
                    continue;
                }
                serializer => bail!("can't read entity data serializer {serializer}"),
            };
            values.push((index, value));
        }
    };
    // What was read before a failure still stands.
    let _ = read();
    values
}

/// The profiles a `ClientboundPlayerInfoUpdatePacket` adds. Its actions are a bit set of
/// `Action`s (one byte, as there are eight), and each entry is a UUID then each action's
/// data in order, so every action is read to reach the next entry.
fn player_info_update(r: &mut Reader) -> Result<Vec<Profile>> {
    let actions = r.u8()?;
    let mut added = Vec::new();
    for _ in 0..r.count()? {
        let uuid = u128::from_be_bytes(r.take(16)?.try_into()?);
        for action in (0..8).filter(|a| actions & 1 << a != 0) {
            match action {
                // ADD_PLAYER: the name and the profile's properties.
                0 => {
                    let name = r.string()?;
                    let mut textures = None;
                    for _ in 0..r.count()? {
                        let (key, value) = (r.string()?, r.string()?);
                        let signature = if r.bool()? { Some(r.string()?) } else { None };
                        // `getPackedTextures`: the first `textures` property.
                        if key == "textures" && textures.is_none() {
                            textures = Some(Textures { value, signature });
                        }
                    }
                    added.push(Profile {
                        uuid,
                        name,
                        textures,
                    });
                }
                // INITIALIZE_CHAT: an optional session id and its signed public key.
                1 => {
                    if r.bool()? {
                        r.take(16)?;
                        r.i64()?;
                        r.byte_array()?;
                        r.byte_array()?;
                    }
                }
                // Game mode, latency and list order.
                2 | 4 | 6 => {
                    r.varint()?;
                }
                // Listed, and whether the hat shows.
                3 | 7 => {
                    r.bool()?;
                }
                // An optional display name.
                _ => {
                    if r.bool()? {
                        r.nbt()?;
                    }
                }
            }
        }
    }
    Ok(added)
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
    Ok(item_stack_dyed(r, registry)?.map(|(item, count, _)| (item, count)))
}

/// [`item_stack`], with its `dyed_color` (RGB) if it has one.
fn item_stack_dyed(
    r: &mut Reader,
    registry: &Registry,
) -> Result<Option<(String, i32, Option<u32>)>> {
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
    let mut dye = None;
    for _ in 0..added {
        let kind = r.count()? as u32;
        let kind = registry.component_type(kind).unwrap_or_default();
        match kind.strip_prefix("minecraft:").unwrap_or(kind) {
            "max_damage" | "damage" | "repair_cost" => {
                r.varint()?;
            }
            "unbreakable" => {}
            "dyed_color" => dye = Some(r.i32()? as u32 & 0xff_ffff),
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
    Ok(Some((name, count, dye)))
}

/// A disconnect reason as a player reads it: the chat component in English, plus how to
/// fix it when the fix is on the player's side.
fn reason(component: &serde_json::Value) -> String {
    let text = component_text(component, &english());
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

/// The text of a chat component (JSON form, which NBT components convert to), without its
/// styles.
fn component_text(
    c: &serde_json::Value,
    lang: &serde_json::Map<String, serde_json::Value>,
) -> String {
    chat::plain(&chat::spans(c, lang))
}

/// The English translations, or none if they can't be read.
fn english() -> serde_json::Map<String, serde_json::Value> {
    std::fs::read(mc_assets::locate(
        "resources/assets/minecraft/lang/en_us.json",
    ))
    .ok()
    .and_then(|bytes| serde_json::from_slice(&bytes).ok())
    .unwrap_or_default()
}

/// A chat component sent as NBT, as English text without its styles.
fn message_text(tag: Option<mc_nbt::Tag>) -> String {
    component_text(
        &tag.as_ref().map_or(serde_json::Value::Null, nbt_json),
        &english(),
    )
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

    /// Entity moves decode as `VecDeltaCodec` does: an unchanged part keeps the base exactly,
    /// a changed one rounds the base to 1/4096 (halves up, as `Math.round`), and steps
    /// count from each other.
    #[test]
    fn entity_moves_decode_like_vanilla() {
        let path = Delta::Linear([0, 4096, -2048]).decode([10.3, 64.0, -5.7]);
        assert_eq!(path, Path::Linear([10.3, 65.0, -25395.0 / 4096.0]));
        let path = Delta::Linear([1, 0, 0]).decode([-0.5 / 4096.0, 0.0, 0.0]);
        assert_eq!(path.end(), [1.0 / 4096.0, 0.0, 0.0]);
        let steps = Delta::Stepped(vec![(2, [4096, 0, 0]), (3, [4096, 0, -4096])]);
        assert_eq!(
            steps.decode([0.0; 3]),
            Path::Stepped(vec![([1.0, 0.0, 0.0], 2), ([2.0, 0.0, -1.0], 3)])
        );
    }

    /// Velocities read as `LpVec3.read`: zero as one byte, small ones with the scale in the
    /// first byte, larger ones with the rest of the scale in a VarInt after.
    #[test]
    fn velocities_read_like_vanilla() {
        // `LpVec3.write`, by hand.
        let write = |v: [f64; 3]| {
            let scale = v.iter().fold(0.0f64, |m, c| m.max(c.abs())).ceil() as u64;
            let partial = scale & 3 != scale;
            let markers = if partial { scale & 3 | 4 } else { scale };
            let pack = |c: f64| ((c / scale as f64 * 0.5 + 0.5) * 32766.0 + 0.5).floor() as u64;
            let buffer = markers | pack(v[0]) << 3 | pack(v[1]) << 18 | pack(v[2]) << 33;
            let mut bytes = vec![buffer as u8, (buffer >> 8) as u8];
            bytes.extend((((buffer >> 16) as u32).to_be_bytes()).iter());
            if partial {
                bytes.put_varint((scale >> 2) as i32);
            }
            bytes
        };
        assert_eq!(lp_vec3(&mut Reader::new(&[0])).unwrap(), [0.0; 3]);
        for v in [[0.5, -0.25, 0.0], [5.0, 0.0, -2.5], [0.0, -13.0, 0.01]] {
            let bytes = write(v);
            let mut r = Reader::new(&bytes);
            let read = lp_vec3(&mut r).unwrap();
            assert_eq!(r.remaining(), 0, "{v:?}");
            for (a, b) in read.iter().zip(v) {
                assert!((a - b).abs() < 2e-3, "{read:?} != {v:?}");
            }
        }
    }

    /// A player info update with every action reads each entry through to the next, and
    /// gives the added players' `textures` with their signatures.
    #[test]
    fn player_info_reads_every_action() {
        let mut body = Vec::new();
        body.put_u8(0xff).put_varint(2);
        for (uuid, name) in [(7u128, "Alex"), (9, "Steve")] {
            body.put_uuid(uuid).put_string(name).put_varint(2);
            body.put_string("other").put_string("x").put_bool(false);
            body.put_string("textures").put_string(&format!("v-{name}"));
            body.put_bool(true).put_string("sig");
            // A chat session: id, expiry, key and signature.
            body.put_bool(true).put_uuid(1).put_i64(0);
            body.put_varint(2)
                .put_u8(1)
                .put_u8(2)
                .put_varint(1)
                .put_u8(3);
            body.put_varint(1).put_bool(true).put_varint(25);
            // A display name, as a network NBT string.
            body.put_bool(true).put_u8(8).put_u16(2);
            body.extend(b"Hi");
            body.put_varint(0).put_bool(true);
        }
        let added = player_info_update(&mut Reader::new(&body)).unwrap();
        let names: Vec<_> = added.iter().map(|p| (p.uuid, p.name.as_str())).collect();
        assert_eq!(names, [(7, "Alex"), (9, "Steve")]);
        assert_eq!(
            added[1].textures,
            Some(Textures {
                value: "v-Steve".into(),
                signature: Some("sig".into())
            })
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
