//! Which sounds play: the sound events of `sounds.json` (`SoundManager`), each picking one of
//! its sounds at random by weight (`WeighedSoundEvents`) and played at a place, following an
//! entity or relative to the listener (`SoundInstance`), by `SoundEngine`'s rules for volume,
//! pitch, categories and channels; and the music (`MusicManager`). `mc_audio` plays them.

use glam::{DVec3, Vec3};
use mc_render::bake::JavaRandom;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

/// `SoundSource`, in its order, which is also how the protocol numbers them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Category {
    Master,
    Music,
    Records,
    Weather,
    Blocks,
    Hostile,
    Neutral,
    Players,
    Ambient,
    Voice,
    Ui,
}

impl Category {
    pub fn from_id(id: i32) -> Self {
        use Category::*;
        [
            Master, Music, Records, Weather, Blocks, Hostile, Neutral, Players, Ambient, Voice, Ui,
        ]
        .get(id as usize)
        .copied()
        .unwrap_or(Master)
    }
}

/// One sound file of an event (`Sound`), or the sound picked from another event, with this
/// entry's volume and pitch applied on top.
#[derive(Clone, Debug, PartialEq)]
struct Sound {
    /// The file, as `sounds/` names it, such as `minecraft:step/stone1`.
    name: String,
    volume: f32,
    pitch: f32,
    weight: u32,
    stream: bool,
    attenuation_distance: i32,
}

enum Entry {
    File(Sound),
    /// `"type": "event"`: another event's pick, scaled by this entry's volume and pitch.
    Event(Sound),
}

/// `sounds.json`'s events, by id such as `minecraft:block.stone.step`.
#[derive(Default)]
struct Registry {
    events: HashMap<String, Vec<Entry>>,
}

impl Registry {
    /// `SoundManager.prepare`: every event, keeping only the files that exist
    /// (`validateSoundResource`), as a missing one would weigh in for nothing.
    fn load(root: &std::path::Path) -> anyhow::Result<Self> {
        let path = root.join("assets/minecraft/sounds.json");
        let json: serde_json::Value = serde_json::from_slice(&std::fs::read(&path)?)?;
        let mut events = HashMap::new();
        for (name, event) in json.as_object().into_iter().flatten() {
            let mut entries = Vec::new();
            for sound in event["sounds"].as_array().into_iter().flatten() {
                // A bare string is a file with the defaults
                // (`SoundEventRegistrationSerializer.getSound`).
                let (file, obj) = match sound.as_str() {
                    Some(name) => (name, &serde_json::Value::Null),
                    None => (sound["name"].as_str().unwrap_or_default(), sound),
                };
                let s = Sound {
                    name: with_namespace(file),
                    volume: obj["volume"].as_f64().unwrap_or(1.0) as f32,
                    pitch: obj["pitch"].as_f64().unwrap_or(1.0) as f32,
                    weight: obj["weight"].as_u64().unwrap_or(1) as u32,
                    stream: obj["stream"].as_bool().unwrap_or(false),
                    attenuation_distance: obj["attenuation_distance"].as_i64().unwrap_or(16) as i32,
                };
                if obj["type"].as_str() == Some("event") {
                    entries.push(Entry::Event(s));
                } else if file_path(root, &s.name).exists() {
                    entries.push(Entry::File(s));
                }
            }
            events.insert(with_namespace(name), entries);
        }
        Ok(Self { events })
    }

    /// `WeighedSoundEvents.getWeight`: an event entry weighs as much as its whole event.
    fn weight(&self, event: &str, depth: u32) -> u32 {
        let Some(entries) = self.events.get(event) else {
            return 0;
        };
        entries
            .iter()
            .map(|e| match e {
                Entry::File(s) => s.weight,
                Entry::Event(s) if depth < 8 => self.weight(&s.name, depth + 1),
                Entry::Event(_) => 0,
            })
            .sum()
    }

    /// `WeighedSoundEvents.getSound`: `None` for an unknown event; an event with nothing to
    /// play gives `SoundManager.EMPTY_SOUND`, which doesn't play either.
    fn pick(&self, event: &str, random: &mut JavaRandom, depth: u32) -> Option<Sound> {
        let entries = self.events.get(event)?;
        let total = self.weight(event, depth);
        if total == 0 || depth >= 8 {
            return None;
        }
        let mut index = random.next_int(total as i32);
        for e in entries {
            let weight = match e {
                Entry::File(s) => s.weight,
                Entry::Event(s) => self.weight(&s.name, depth + 1),
            } as i32;
            index -= weight;
            if index < 0 {
                return match e {
                    Entry::File(s) => Some(s.clone()),
                    Entry::Event(s) => {
                        let inner = self.pick(&s.name, random, depth + 1)?;
                        Some(Sound {
                            volume: inner.volume * s.volume,
                            pitch: inner.pitch * s.pitch,
                            weight: s.weight,
                            stream: inner.stream || s.stream,
                            ..inner
                        })
                    }
                };
            }
        }
        None
    }
}

fn with_namespace(id: &str) -> String {
    if id.contains(':') {
        id.to_owned()
    } else {
        format!("minecraft:{id}")
    }
}

fn file_path(root: &std::path::Path, name: &str) -> PathBuf {
    let (namespace, path) = name.split_once(':').unwrap_or(("minecraft", name));
    root.join("assets")
        .join(namespace)
        .join("sounds")
        .join(format!("{path}.ogg"))
}

/// A sound to play (`SoundInstance`).
#[derive(Clone, Debug)]
pub struct Instance {
    /// The sound event, such as `minecraft:block.stone.step`.
    pub event: String,
    pub category: Category,
    pub volume: f32,
    pub pitch: f32,
    pub pos: DVec3,
    /// Placed from the listener rather than in the world.
    pub relative: bool,
    /// `Attenuation.LINEAR`: fades out over the sound's distance; `NONE` doesn't.
    pub attenuation: bool,
    pub looping: bool,
    /// Ticks between plays of a looping sound.
    pub delay: u32,
    /// Seeds the pick of the event's sounds (`RandomSource.create(seed)`).
    pub seed: i64,
    /// `EntityBoundSoundInstance`: follows this entity, and stops when it goes.
    pub follow: Option<i32>,
}

impl Instance {
    /// `SimpleSoundInstance` at a place.
    pub fn at(
        event: &str,
        category: Category,
        volume: f32,
        pitch: f32,
        pos: DVec3,
        seed: i64,
    ) -> Self {
        Self {
            event: with_namespace(event),
            category,
            volume,
            pitch,
            pos,
            relative: false,
            attenuation: true,
            looping: false,
            delay: 0,
            seed,
            follow: None,
        }
    }

    /// `SimpleSoundInstance(..., BlockPos)`: at the block's centre.
    pub fn at_block(
        event: &str,
        category: Category,
        volume: f32,
        pitch: f32,
        pos: [i32; 3],
        seed: i64,
    ) -> Self {
        let pos = DVec3::new(pos[0] as f64, pos[1] as f64, pos[2] as f64) + 0.5;
        Self::at(event, category, volume, pitch, pos, seed)
    }

    /// `SimpleSoundInstance.forUI`: quiet, in the middle of the head.
    pub fn ui(event: &str, pitch: f32, seed: i64) -> Self {
        Self {
            relative: true,
            attenuation: false,
            ..Self::at(event, Category::Ui, 0.25, pitch, DVec3::ZERO, seed)
        }
    }

    /// `SimpleSoundInstance.forMusic`.
    pub fn music(event: &str, seed: i64) -> Self {
        Self {
            relative: true,
            attenuation: false,
            ..Self::at(event, Category::Music, 1.0, 1.0, DVec3::ZERO, seed)
        }
    }

    /// `EntityBoundSoundInstance`.
    pub fn following(
        event: &str,
        category: Category,
        volume: f32,
        pitch: f32,
        entity: (i32, DVec3),
        seed: i64,
    ) -> Self {
        Self {
            follow: Some(entity.0),
            ..Self::at(event, category, volume, pitch, entity.1, seed)
        }
    }
}

/// A sound playing on a voice, or waiting to be removed (`instanceToChannel`).
struct Playing {
    instance: Instance,
    sound: Sound,
    voice: Option<u64>,
    /// Ticks before it may be removed once stopped (`soundDeleteTime`).
    delete_at: u32,
    stopped: bool,
}

/// A sound's number, to stop it or ask whether it still plays.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Handle(u64);

/// `Library`'s channel pools for OpenAL Soft's 256 sources: `sqrt` of them, at most 8, for
/// streams and the rest for whole sounds.
const STATIC_VOICES: usize = 248;
const STREAMING_VOICES: usize = 8;

/// `SoundManager` and `SoundEngine`.
pub struct Sounds {
    audio: Option<mc_audio::Audio>,
    root: PathBuf,
    registry: Registry,
    playing: HashMap<Handle, Playing>,
    voices: HashMap<u64, Handle>,
    queued: Vec<(Instance, u32)>,
    ticks: u32,
    /// `gainBySource`: what the music fades with.
    gains: HashMap<Category, f32>,
    /// `Options.soundSourceVolumes`: each category's volume from the options, by
    /// `Category`'s order.
    pub volumes: [f64; 11],
    /// `SoundPreviewHandler`'s sound and its category.
    preview: Option<(Handle, Category)>,
    warned: HashSet<String>,
    next: u64,
    /// `RandomSource.create()` for sounds without a seed of their own.
    pub random: JavaRandom,
    pub music: Music,
}

impl Sounds {
    /// Opens the audio device and reads `sounds.json`; without either, nothing plays, as
    /// vanilla carries on when OpenAL won't start.
    pub fn new() -> Self {
        let root = mc_assets::default_root();
        let registry = Registry::load(&root).unwrap_or_else(|e| {
            eprintln!("no sounds: {e:#}");
            Registry::default()
        });
        let audio = if registry.events.is_empty() {
            None
        } else {
            mc_audio::Audio::open()
                .map_err(|e| eprintln!("no sound: {e:#}"))
                .ok()
        };
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos() as i64);
        Self {
            audio,
            root,
            registry,
            playing: HashMap::new(),
            voices: HashMap::new(),
            queued: Vec::new(),
            ticks: 0,
            gains: HashMap::new(),
            volumes: [1.0; 11],
            preview: None,
            warned: HashSet::new(),
            next: 0,
            random: JavaRandom::new(seed),
            music: Music::new(seed ^ 0x5DEE_CE66),
        }
    }

    /// A seed for a sound that has none of its own (`level.random.nextLong()`).
    pub fn seed(&mut self) -> i64 {
        self.random.next_long()
    }

    /// `SoundEngine.calculateVolume`: the instance's volume, then its category's from the
    /// options, then the category's gain (the music's fade).
    fn volume(&self, volume: f32, category: Category) -> f32 {
        volume.clamp(0.0, 1.0)
            * self.final_volume(category).clamp(0.0, 1.0)
            * self.gains.get(&category).copied().unwrap_or(1.0)
    }

    /// `Options.getFinalSoundSourceVolume`: a category's volume times the master volume.
    fn final_volume(&self, category: Category) -> f32 {
        let master = self.volumes[Category::Master as usize] as f32;
        match category {
            Category::Master => master,
            _ => self.volumes[category as usize] as f32 * master,
        }
    }

    /// A volume slider moved: the sounds playing take the category's new volume
    /// (`SoundEngine.refreshCategoryVolume`), every category's for the master volume.
    pub fn set_volume(&mut self, category: Category, volume: f64) {
        self.volumes[category as usize] = volume.clamp(0.0, 1.0);
        self.refresh(category);
    }

    fn refresh(&self, category: Category) {
        let Some(audio) = &self.audio else { return };
        for p in self.playing.values() {
            if (category == Category::Master || p.instance.category == category)
                && let Some(id) = p.voice
            {
                audio.update(id, self.voice(p));
            }
        }
    }

    /// `SoundPreviewHandler.preview`: out of a world, moving a category's slider plays a
    /// sound of that category at the slider's volume, one at a time; moving another
    /// category's stops the last one's.
    pub fn preview(&mut self, category: Category, volume: f32) {
        if let Some((handle, previous)) = self.preview
            && previous != category
        {
            self.stop(handle);
            self.preview = None;
        }
        if self.preview.is_some_and(|(h, _)| self.is_active(h)) {
            return;
        }
        let event = match category {
            Category::Records => "minecraft:block.note_block.guitar",
            Category::Weather => "minecraft:entity.lightning_bolt.thunder",
            Category::Blocks => "minecraft:block.grass.place",
            Category::Hostile => "minecraft:entity.zombie.ambient",
            Category::Neutral => "minecraft:entity.cow.ambient",
            Category::Players => "minecraft:entity.generic.eat",
            Category::Ambient => "minecraft:ambient.cave",
            Category::Ui => "minecraft:ui.button.click",
            _ => return,
        };
        let seed = self.seed();
        let instance = Instance {
            volume,
            ..Instance::ui(event, 1.0, seed)
        };
        self.preview = self.play(instance).map(|h| (h, category));
    }

    /// `SoundEngine.play`: the sound's number, unless it doesn't play.
    pub fn play(&mut self, instance: Instance) -> Option<Handle> {
        self.audio.as_ref()?;
        let mut random = JavaRandom::new(instance.seed);
        if !self.registry.events.contains_key(&instance.event) {
            if self.warned.insert(instance.event.clone()) {
                eprintln!("Unable to play unknown soundEvent: {}", instance.event);
            }
            return None;
        }
        let Some(sound) = self.registry.pick(&instance.event, &mut random, 0) else {
            if self.warned.insert(instance.event.clone()) {
                eprintln!("Unable to play empty soundEvent: {}", instance.event);
            }
            return None;
        };
        let volume = instance.volume * sound.volume;
        let attenuation_distance = volume.max(1.0) * sound.attenuation_distance as f32;
        let gain = self.volume(volume, instance.category);
        // Music starts even when silent, so it's there when turned up.
        if gain == 0.0 && instance.category != Category::Music {
            return None;
        }
        // `ChannelAccess.createHandle`: none left in the pool, nothing plays.
        let pool = self
            .playing
            .values()
            .filter(|p| !p.stopped && p.sound.stream == sound.stream)
            .count();
        if pool
            >= if sound.stream {
                STREAMING_VOICES
            } else {
                STATIC_VOICES
            }
        {
            return None;
        }
        let looping = instance.looping && instance.delay == 0;
        let voice = mc_audio::Voice {
            pos: instance.pos.to_array(),
            relative: instance.relative,
            attenuation: instance.attenuation.then_some(attenuation_distance),
            gain,
            pitch: (instance.pitch * sound.pitch).clamp(0.5, 2.0),
            looping,
        };
        let audio = self.audio.as_mut().expect("checked");
        let id = audio.play(&file_path(&self.root, &sound.name), sound.stream, voice);
        self.next += 1;
        let handle = Handle(self.next);
        self.voices.insert(id, handle);
        self.playing.insert(
            handle,
            Playing {
                instance,
                sound,
                voice: Some(id),
                delete_at: self.ticks + 20,
                stopped: false,
            },
        );
        Some(handle)
    }

    pub fn stop(&mut self, handle: Handle) {
        if let (Some(audio), Some(p)) = (&self.audio, self.playing.get(&handle))
            && let Some(id) = p.voice
        {
            audio.stop(id);
        }
    }

    /// `SoundEngine.stop(Identifier, SoundSource)`: the sounds of an event, a category, both,
    /// or everything.
    pub fn stop_matching(&mut self, event: Option<&str>, category: Option<Category>) {
        if event.is_none() && category.is_none() {
            self.stop_all();
            return;
        }
        let event = event.map(with_namespace);
        let handles: Vec<Handle> = self
            .playing
            .iter()
            .filter(|(_, p)| {
                event.as_ref().is_none_or(|e| *e == p.instance.event)
                    && category.is_none_or(|c| c == p.instance.category)
            })
            .map(|(h, _)| *h)
            .collect();
        for h in handles {
            self.stop(h);
        }
    }

    /// `SoundEngine.stopAll`: also forgets queued sounds and the music's fade.
    pub fn stop_all(&mut self) {
        if let Some(audio) = &self.audio {
            for id in self.playing.values().filter_map(|p| p.voice) {
                audio.stop(id);
            }
        }
        self.playing.clear();
        self.voices.clear();
        self.queued.clear();
        self.gains.clear();
    }

    /// `SoundEngine.isActive`.
    pub fn is_active(&self, handle: Handle) -> bool {
        self.playing.contains_key(&handle)
    }

    /// `SoundEngine.updateCategoryVolume`.
    pub fn set_gain(&mut self, category: Category, gain: f32) {
        self.gains.insert(category, gain.clamp(0.0, 1.0));
        self.refresh(category);
    }

    fn voice(&self, p: &Playing) -> mc_audio::Voice {
        let volume = p.instance.volume * p.sound.volume;
        mc_audio::Voice {
            pos: p.instance.pos.to_array(),
            relative: p.instance.relative,
            attenuation: p
                .instance
                .attenuation
                .then_some(volume.max(1.0) * p.sound.attenuation_distance as f32),
            gain: self.volume(volume, p.instance.category),
            pitch: (p.instance.pitch * p.sound.pitch).clamp(0.5, 2.0),
            looping: p.instance.looping && p.instance.delay == 0,
        }
    }

    /// `SoundEngine.updateSource`: the listener at the camera.
    pub fn listener(&self, eye: Vec3, forward: Vec3, up: Vec3) {
        if let Some(audio) = &self.audio {
            audio.listener(mc_audio::Listener {
                pos: eye.as_dvec3().to_array(),
                forward: forward.to_array(),
                up: up.to_array(),
            });
        }
    }

    /// `SoundEngine.tickInGameSound`: sounds bound to an entity follow it (or stop when it's
    /// gone, as `entity` says), finished sounds go after their minimum life, looping sounds
    /// with a delay come round again, and delayed sounds start.
    pub fn tick(&mut self, entity: &dyn Fn(i32) -> Option<DVec3>) {
        self.ticks += 1;
        let Some(audio) = &self.audio else { return };
        for id in audio.finished() {
            if let Some(h) = self.voices.remove(&id)
                && let Some(p) = self.playing.get_mut(&h)
            {
                p.stopped = true;
                p.voice = None;
            }
        }
        let mut gone = Vec::new();
        let mut moved = Vec::new();
        for (h, p) in &mut self.playing {
            if let (Some(id), Some(e)) = (p.voice, p.instance.follow) {
                match entity(e) {
                    Some(pos) => {
                        p.instance.pos = pos;
                        moved.push((id, *h));
                    }
                    None => gone.push(id),
                }
            }
        }
        for id in gone {
            audio.stop(id);
        }
        for (id, h) in moved {
            audio.update(id, self.voice(&self.playing[&h]));
        }
        let ticks = self.ticks;
        let mut again = Vec::new();
        self.playing.retain(|_, p| {
            if p.stopped && p.delete_at <= ticks {
                if p.instance.looping && p.instance.delay > 0 {
                    again.push((p.instance.clone(), ticks + p.instance.delay));
                }
                false
            } else {
                true
            }
        });
        self.queued.extend(again);
        let (due, later): (Vec<_>, Vec<_>) = std::mem::take(&mut self.queued)
            .into_iter()
            .partition(|(_, at)| ticks >= *at);
        self.queued = later;
        for (instance, _) in due {
            self.play(instance);
        }
    }
}

/// `Music`: what plays, how long to wait between songs, and whether it cuts in on other music.
#[derive(Clone, Debug, PartialEq)]
pub struct Track {
    pub event: String,
    pub min_delay: i32,
    pub max_delay: i32,
    pub replace: bool,
}

impl Track {
    /// `Musics.MENU`.
    pub fn menu() -> Self {
        Self {
            event: "minecraft:music.menu".into(),
            min_delay: 20,
            max_delay: 600,
            replace: true,
        }
    }
}

/// `LevelEventHandler.levelEvent`'s events that only play a sound at the block: the event,
/// category, volume and pitch for event `kind` with `data`, picking any randomness from
/// `random` as `level.random` would.
pub fn level_event(
    kind: i32,
    data: i32,
    random: &mut JavaRandom,
) -> Option<(&'static str, Category, f32, f32)> {
    use Category::{Blocks, Hostile, Neutral};
    // `(random.nextFloat() - random.nextFloat()) * 0.2 + 1`, and `nextFloat() * 0.1 + 0.9`.
    fn mob(random: &mut JavaRandom) -> f32 {
        (random.next_float() - random.next_float()) * 0.2 + 1.0
    }
    let sound = match kind {
        1000 => ("block.dispenser.dispense", Blocks, 1.0, 1.0),
        1001 => ("block.dispenser.fail", Blocks, 1.0, 1.2),
        1002 => ("block.dispenser.launch", Blocks, 1.0, 1.2),
        1004 => ("entity.firework_rocket.shoot", Neutral, 1.0, 1.2),
        1009 => {
            let r = (random.next_float() - random.next_float()) * 0.8;
            match data {
                0 => ("block.fire.extinguish", Blocks, 0.5, 2.6 + r),
                1 => ("entity.generic.extinguish_fire", Blocks, 0.7, 1.6 + r / 2.0),
                _ => return None,
            }
        }
        1015 => ("entity.ghast.warn", Hostile, 10.0, mob(random)),
        1016 => ("entity.ghast.shoot", Hostile, 10.0, mob(random)),
        1017 => ("entity.ender_dragon.shoot", Hostile, 10.0, mob(random)),
        1018 => ("entity.blaze.shoot", Hostile, 2.0, mob(random)),
        1019 => (
            "entity.zombie.attack_wooden_door",
            Hostile,
            2.0,
            mob(random),
        ),
        1020 => ("entity.zombie.attack_iron_door", Hostile, 2.0, mob(random)),
        1021 => ("entity.zombie.break_wooden_door", Hostile, 2.0, mob(random)),
        1022 => ("entity.wither.break_block", Hostile, 2.0, mob(random)),
        1024 => ("entity.wither.shoot", Hostile, 2.0, mob(random)),
        1025 => ("entity.bat.takeoff", Neutral, 0.05, mob(random)),
        1026 => ("entity.zombie.infect", Hostile, 2.0, mob(random)),
        1027 => (
            "entity.zombie_villager.converted",
            Hostile,
            2.0,
            mob(random),
        ),
        1029 => (
            "block.anvil.destroy",
            Blocks,
            1.0,
            random.next_float() * 0.1 + 0.9,
        ),
        1030 => (
            "block.anvil.use",
            Blocks,
            1.0,
            random.next_float() * 0.1 + 0.9,
        ),
        1031 => (
            "block.anvil.land",
            Blocks,
            0.3,
            random.next_float() * 0.1 + 0.9,
        ),
        1033 => ("block.chorus_flower.grow", Blocks, 1.0, 1.0),
        1034 => ("block.chorus_flower.death", Blocks, 1.0, 1.0),
        1035 => ("block.brewing_stand.brew", Blocks, 1.0, 1.0),
        1039 => (
            "entity.phantom.bite",
            Hostile,
            0.3,
            random.next_float() * 0.1 + 0.9,
        ),
        1040 => (
            "entity.zombie.converted_to_drowned",
            Hostile,
            2.0,
            mob(random),
        ),
        1041 => ("entity.husk.converted_to_zombie", Hostile, 2.0, mob(random)),
        1042 => (
            "block.grindstone.use",
            Blocks,
            1.0,
            random.next_float() * 0.1 + 0.9,
        ),
        1043 => (
            "item.book.page_turn",
            Blocks,
            1.0,
            random.next_float() * 0.1 + 0.9,
        ),
        1044 => (
            "block.smithing_table.use",
            Blocks,
            1.0,
            random.next_float() * 0.1 + 0.9,
        ),
        1048 => (
            "entity.skeleton.converted_to_stray",
            Hostile,
            2.0,
            mob(random),
        ),
        1049 => ("block.crafter.craft", Blocks, 1.0, 1.0),
        1050 => ("block.crafter.fail", Blocks, 1.0, 1.0),
        1501 => {
            let r = (random.next_float() - random.next_float()) * 0.8;
            ("block.lava.extinguish", Blocks, 0.5, 2.6 + r)
        }
        1502 => {
            let r = (random.next_float() - random.next_float()) * 0.8;
            ("block.redstone_torch.burnout", Blocks, 0.5, 2.6 + r)
        }
        1503 => ("block.end_portal_frame.fill", Blocks, 1.0, 1.0),
        1505 => ("item.bone_meal.use", Blocks, 1.0, 1.0),
        _ => return None,
    };
    Some(sound)
}

/// A dimension type's `minecraft:audio/background_music`: its default, creative and
/// underwater music, each `None` where it has none.
pub fn background_music(res: &mc_assets::Resources, dimension: &str) -> [Option<Track>; 3] {
    let path = res.data_path("dimension_type", dimension, "json");
    let json: serde_json::Value = std::fs::read(path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();
    let music = &json["attributes"]["minecraft:audio/background_music"];
    ["default", "creative", "underwater"].map(|key| {
        let m = &music[key];
        Some(Track {
            event: with_namespace(m["sound"].as_str()?),
            min_delay: m["min_delay"].as_i64()? as i32,
            max_delay: m["max_delay"].as_i64()? as i32,
            replace: m["replace_current_music"].as_bool().unwrap_or(false),
        })
    })
}

/// `MusicManager` at the default `MusicFrequency`: a new song at most 20 minutes after the
/// last.
pub struct Music {
    random: JavaRandom,
    current: Option<(Handle, String)>,
    gain: f32,
    next_song_delay: i32,
}

const MAX_FREQUENCY: i32 = 20 * 1200;

impl Music {
    fn new(seed: i64) -> Self {
        Self {
            random: JavaRandom::new(seed),
            current: None,
            gain: 1.0,
            next_song_delay: 100,
        }
    }

    /// `Mth.nextInt`.
    fn next_int(&mut self, min: i32, max: i32) -> i32 {
        if min >= max {
            min
        } else {
            self.random.next_int(max - min + 1) + min
        }
    }

    /// `MusicFrequency.getNextSongDelay`.
    fn song_delay(&mut self, track: Option<&Track>) -> i32 {
        match track {
            None => MAX_FREQUENCY,
            Some(t) => {
                let (min, max) = (
                    t.min_delay.min(MAX_FREQUENCY),
                    t.max_delay.min(MAX_FREQUENCY),
                );
                self.next_int(min, max)
            }
        }
    }
}

impl Sounds {
    /// `MusicManager.tick`: `track` is `Minecraft.getSituationalMusic`, `volume`
    /// `getMusicVolume`, and `loading` whether the world is still loading, when no song
    /// starts.
    pub fn tick_music(&mut self, track: Option<&Track>, volume: f32, loading: bool) {
        if self.music.current.is_some() && self.music.gain != volume && !self.fade_music(volume) {
            return;
        }
        let Some(track) = track else {
            self.music.next_song_delay = self.music.next_song_delay.max(100);
            return;
        };
        if let Some((handle, event)) = self.music.current.clone() {
            // `canReplace`.
            if track.replace && track.event != event {
                self.stop(handle);
                self.music.next_song_delay = self.music.next_int(0, track.min_delay / 2);
            }
            if !self.is_active(handle) {
                self.music.current = None;
                let delay = self.music.song_delay(Some(track));
                self.music.next_song_delay = self.music.next_song_delay.min(delay);
            }
        }
        self.music.next_song_delay = self.music.next_song_delay.min(track.max_delay);
        if self.music.current.is_none() && !loading {
            self.music.next_song_delay -= 1;
            if self.music.next_song_delay <= 0 {
                // `startPlaying`.
                let seed = self.seed();
                if let Some(handle) = self.play(Instance::music(&track.event, seed)) {
                    self.music.current = Some((handle, track.event.clone()));
                }
                self.music.next_song_delay = i32::MAX;
            }
        }
    }

    /// `MusicManager.stopPlaying`.
    pub fn stop_music(&mut self, track: Option<&Track>) {
        if let Some((handle, _)) = self.music.current.take() {
            self.stop(handle);
        }
        self.music.next_song_delay = self.music.song_delay(track) + 100;
    }

    /// `MusicManager.fadePlaying`: eases the music's gain toward `volume`; false once it has
    /// faded out and stopped.
    fn fade_music(&mut self, volume: f32) -> bool {
        if self.music.current.is_none() {
            return false;
        }
        let m = &mut self.music;
        if m.gain == volume {
            return true;
        }
        if m.gain < volume {
            m.gain += m.gain.clamp(5.0e-4, 0.005);
            if m.gain > volume {
                m.gain = volume;
            }
        } else {
            m.gain = 0.03 * volume + 0.97 * m.gain;
            if (m.gain - volume).abs() < 1.0e-4 || m.gain < volume {
                m.gain = volume;
            }
        }
        m.gain = m.gain.clamp(0.0, 1.0);
        if m.gain <= 1.0e-4 {
            self.stop_music(None);
            false
        } else {
            let gain = self.music.gain;
            self.set_gain(Category::Music, gain);
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registry(json: &str, files: &[&str]) -> (Registry, tempdir::Dir) {
        let dir = tempdir::Dir::new();
        std::fs::create_dir_all(dir.0.join("assets/minecraft/sounds")).unwrap();
        std::fs::write(dir.0.join("assets/minecraft/sounds.json"), json).unwrap();
        for f in files {
            let path = file_path(&dir.0, &with_namespace(f));
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, b"").unwrap();
        }
        (Registry::load(&dir.0).unwrap(), dir)
    }

    mod tempdir {
        pub struct Dir(pub std::path::PathBuf);
        impl Dir {
            pub fn new() -> Self {
                static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
                let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let p =
                    std::env::temp_dir().join(format!("mc-sound-test-{}-{n}", std::process::id()));
                let _ = std::fs::remove_dir_all(&p);
                Self(p)
            }
        }
        impl Drop for Dir {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
    }

    /// The pick follows `WeighedSoundEvents.getSound` for a seeded `java.util.Random`: one
    /// `nextInt` over the summed weights, an event entry weighing as its whole event, and
    /// a missing file left out of the sum.
    #[test]
    fn events_pick_by_weight_with_vanillas_random() {
        let (r, _dir) = registry(
            r#"{
              "a": {"sounds": ["x/one", {"name": "x/two", "weight": 3, "volume": 0.5}, "x/missing"]},
              "b": {"sounds": [{"name": "a", "type": "event", "pitch": 2.0, "weight": 9}, "x/three"]}
            }"#,
            &["x/one", "x/two", "x/three"],
        );
        assert_eq!(r.weight("minecraft:a", 0), 4);
        // `b` weighs `a`'s 4 plus `x/three`'s 1.
        assert_eq!(r.weight("minecraft:b", 0), 5);
        for seed in 0..50 {
            let mut random = JavaRandom::new(seed);
            let mut expect = JavaRandom::new(seed);
            let i = expect.next_int(4);
            let want = if i < 1 {
                "minecraft:x/one"
            } else {
                "minecraft:x/two"
            };
            assert_eq!(r.pick("minecraft:a", &mut random, 0).unwrap().name, want);
        }
        // Through `b`'s event entry, `a`'s pick takes `b`'s pitch on top of its own.
        let picks: Vec<_> = (0..200)
            .map(|seed| {
                r.pick("minecraft:b", &mut JavaRandom::new(seed), 0)
                    .unwrap()
            })
            .collect();
        let two = picks.iter().find(|s| s.name == "minecraft:x/two").unwrap();
        assert_eq!((two.volume, two.pitch, two.weight), (0.5, 2.0, 9));
        let threes = picks
            .iter()
            .filter(|s| s.name == "minecraft:x/three")
            .count();
        assert!(
            (20..60).contains(&threes),
            "{threes} of 200 should be about 40"
        );
        assert!(
            r.pick("minecraft:nothing", &mut JavaRandom::new(1), 0)
                .is_none()
        );
    }
}
