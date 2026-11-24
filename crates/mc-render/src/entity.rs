//! Entities in the world: players and mobs as vanilla 26.3's `ModelPart` models, posed by
//! their models' `setupAnim` and placed as `LivingEntityRenderer.submit` places them, and
//! dropped items as `ItemEntityRenderer` bobs and spins them. Each frame's entities become
//! world-space quads, lit by the light where the entity is and shaded by vanilla's two
//! entity lights (`Lighting`'s level setup, as the `minecraft_mix_light` shader mixes them).
//!
//! Not drawn yet: armour trims and enchantment glint, most babies' own models (a baby
//! without one is the adult model at half size), armour on babies that have one, cold and warm variants' own models (they get their textures on the
//! temperate model), and hurt and death animations.

use crate::atlas::Atlas;
use crate::bake::Quad;
use crate::entity_model::{Builder, Cube, DYE_RGB, Node, cube};
use crate::item::ItemIcon;
use crate::mesh::Vertex;
use glam::{Mat4, Quat, Vec3};
use std::collections::HashMap;
use std::f32::consts::PI;

/// The textures these models use, so the atlas loads them. Players' default skins are
/// loaded with the player heads' (`entity_model::sprites`).
pub fn sprites() -> Vec<String> {
    let mut out: Vec<String> = [
        ARROW[0],
        ARROW[1],
        ARROW[2],
        ZOMBIE,
        SPIDER_EYES,
        SHEEP_UNDERCOAT,
        SKELETON,
        CREEPER,
        SPIDER,
        SHEEP,
        SHEEP_WOOL,
        ENDERMAN,
        ENDERMAN_EYES,
        SLIME,
        WITCH,
        HUSK,
        DROWNED,
        DROWNED_OUTER,
        STRAY,
        STRAY_OVERLAY,
        VILLAGER,
        WOLF_COLLAR,
        ZOMBIE_VILLAGER,
        CAVE_SPIDER,
        IRON_GOLEM,
        SQUID,
        GLOW_SQUID,
        SQUID_BABY,
        GLOW_SQUID_BABY,
        OCELOT,
        OCELOT_BABY,
        CAT_COLLAR_BABY,
        PHANTOM,
        PHANTOM_EYES,
        CAT_COLLAR,
        BOGGED,
        BOGGED_OVERLAY,
        SNOW_GOLEM,
        MAGMA_CUBE,
        BLAZE,
        GHAST,
        GHAST_SHOOTING,
        SILVERFISH,
        ENDERMITE,
        POLAR_BEAR,
        GOAT,
        PIGLIN,
        PIGLIN_BRUTE,
        PIGLIN_BABY,
        ZOMBIFIED_PIGLIN_BABY,
        HOGLIN_BABY,
        ZOGLIN_BABY,
        STRIDER_BABY,
        STRIDER_COLD_BABY,
        ZOMBIFIED_PIGLIN,
        HOGLIN,
        ZOGLIN,
        STRIDER,
        STRIDER_COLD,
        WITHER_SKELETON,
        "minecraft:entity/fox/fox",
        "minecraft:entity/fox/fox_sleep",
        "minecraft:entity/fox/fox_snow",
        "minecraft:entity/fox/fox_snow_sleep",
        "minecraft:entity/fox/fox_baby",
        "minecraft:entity/fox/fox_sleep_baby",
        "minecraft:entity/fox/fox_snow_baby",
        "minecraft:entity/fox/fox_snow_sleep_baby",
        ZOMBIE_VILLAGER_BABY,
        GOAT_BABY,
        POLAR_BEAR_BABY,
        BAT,
        CAMEL,
        CAMEL_BABY,
        ARMADILLO,
        ARMADILLO_BABY,
        "minecraft:entity/frog/frog_temperate",
        "minecraft:entity/frog/frog_warm",
        "minecraft:entity/frog/frog_cold",
        "minecraft:entity/rabbit/rabbit_toast",
        "minecraft:entity/rabbit/rabbit_toast_baby",
    ]
    .map(str::to_owned)
    .into();
    for animal in ["cow", "pig", "chicken"] {
        for v in ["temperate", "warm", "cold"] {
            out.push(format!("minecraft:entity/{animal}/{animal}_{v}"));
        }
    }
    for variant in WOLF_VARIANTS {
        for mood in ["", "_tame", "_angry"] {
            out.push(wolf_texture(variant, mood));
        }
    }
    for base in ["villager", "zombie_villager"] {
        for t in VILLAGER_TYPES {
            out.push(format!("minecraft:entity/{base}/type/{t}"));
        }
        if base == "zombie_villager" {
            for t in VILLAGER_TYPES {
                out.push(format!("minecraft:entity/{base}/baby/{t}"));
            }
        }
        for p in VILLAGER_PROFESSIONS {
            out.push(format!("minecraft:entity/{base}/profession/{p}"));
        }
        for l in VILLAGER_LEVELS {
            out.push(format!("minecraft:entity/{base}/profession_level/{l}"));
        }
    }
    for level in ["low", "medium", "high"] {
        out.push(format!("{IRON_GOLEM}_crackiness_{level}"));
    }
    for v in RABBIT_VARIANTS {
        out.push(format!("minecraft:entity/rabbit/rabbit_{v}"));
        out.push(format!("minecraft:entity/rabbit/rabbit_{v}_baby"));
    }
    for cat in CAT_VARIANTS {
        out.push(format!("minecraft:entity/cat/cat_{cat}"));
        out.push(format!("minecraft:entity/cat/cat_{cat}_baby"));
    }
    out
}

/// The vanilla cat variants (`CatVariants`), whose textures are `cat_` and their name.
pub const CAT_VARIANTS: [&str; 11] = [
    "tabby",
    "black",
    "red",
    "siamese",
    "british_shorthair",
    "calico",
    "persian",
    "ragdoll",
    "white",
    "jellie",
    "all_black",
];

/// The vanilla wolf variants (`WolfVariants`), whose textures are named after them.
const WOLF_VARIANTS: [&str; 9] = [
    "pale", "spotted", "snowy", "black", "ashen", "rusty", "woods", "chestnut", "striped",
];

/// A wolf variant's texture, wild (`""`), tame (`"_tame"`) or angry (`"_angry"`): the pale
/// wolf's are plain `wolf`, the others' carry their name.
pub fn wolf_texture(variant: &str, mood: &str) -> String {
    let variant = variant.strip_prefix("minecraft:").unwrap_or(variant);
    let name = if variant == "pale" || !WOLF_VARIANTS.contains(&variant) {
        "wolf".to_owned()
    } else {
        format!("wolf_{variant}")
    };
    format!("minecraft:entity/wolf/{name}{mood}")
}

/// `VillagerType`s and `VillagerProfession`s with textures, and the badges of each level
/// (`VillagerProfessionLayer.LEVEL_LOCATIONS`).
pub const VILLAGER_TYPES: [&str; 7] = [
    "desert", "jungle", "plains", "savanna", "snow", "swamp", "taiga",
];
pub const VILLAGER_PROFESSIONS: [&str; 14] = [
    "armorer",
    "butcher",
    "cartographer",
    "cleric",
    "farmer",
    "fisherman",
    "fletcher",
    "leatherworker",
    "librarian",
    "mason",
    "nitwit",
    "shepherd",
    "toolsmith",
    "weaponsmith",
];
pub const VILLAGER_LEVELS: [&str; 5] = ["stone", "iron", "gold", "emerald", "diamond"];

/// `VillagerMetaDataSection.Hat` for each villager type and profession texture that has
/// one, by sprite id, read from their `.mcmeta` files: partial 1, full 2.
pub(crate) fn villager_hats(res: &mc_assets::Resources) -> HashMap<String, u8> {
    let mut hats = HashMap::new();
    let types = VILLAGER_TYPES.iter().map(|t| format!("type/{t}"));
    let professions = VILLAGER_PROFESSIONS
        .iter()
        .map(|p| format!("profession/{p}"));
    let paths: Vec<String> = types.chain(professions).collect();
    let ids = ["villager", "zombie_villager"].iter().flat_map(|base| {
        paths
            .iter()
            .map(move |p| format!("minecraft:entity/{base}/{p}"))
    });
    for id in ids {
        let file = res.asset_path("textures", &id, "png.mcmeta");
        let Ok(text) = std::fs::read_to_string(file) else {
            continue;
        };
        let meta: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
        let hat = match meta["villager"]["hat"].as_str() {
            Some("partial") => 1,
            Some("full") => 2,
            _ => continue,
        };
        hats.insert(id, hat);
    }
    hats
}
/// How many players' downloaded skins can be drawn at once: each has a blank 64×64 slot in
/// the atlas that `Renderer::write_sprite` fills once the skin arrives.
pub const SKIN_SLOTS: usize = 32;

/// The atlas id of a skin slot.
pub fn skin_slot(i: usize) -> String {
    format!("minecraft:cardboard/player_skin_{i}")
}

/// The blank skin slots, for `Atlas::build`.
pub(crate) fn skin_slots() -> impl Iterator<Item = (String, image::RgbaImage)> {
    (0..SKIN_SLOTS).map(|i| (skin_slot(i), image::RgbaImage::new(64, 64)))
}

/// `TippableArrowRenderer`'s and `SpectralArrowRenderer`'s textures: plain, tipped and
/// spectral.
pub const ARROW: [&str; 3] = [
    "minecraft:entity/projectiles/arrow",
    "minecraft:entity/projectiles/arrow_tipped",
    "minecraft:entity/projectiles/arrow_spectral",
];
pub const ZOMBIE: &str = "minecraft:entity/zombie/zombie";
pub const SKELETON: &str = "minecraft:entity/skeleton/skeleton";
pub const CREEPER: &str = "minecraft:entity/creeper/creeper";
pub const SPIDER: &str = "minecraft:entity/spider/spider";
pub const SHEEP: &str = "minecraft:entity/sheep/sheep";
const SHEEP_WOOL: &str = "minecraft:entity/sheep/sheep_wool";
const SHEEP_UNDERCOAT: &str = "minecraft:entity/sheep/sheep_wool_undercoat";
const SPIDER_EYES: &str = "minecraft:entity/spider/spider_eyes";
pub const ENDERMAN: &str = "minecraft:entity/enderman/enderman";
const ENDERMAN_EYES: &str = "minecraft:entity/enderman/enderman_eyes";
pub const SLIME: &str = "minecraft:entity/slime/slime";
pub const WITCH: &str = "minecraft:entity/witch/witch";
pub const HUSK: &str = "minecraft:entity/zombie/husk";
pub const DROWNED: &str = "minecraft:entity/zombie/drowned";
const DROWNED_OUTER: &str = "minecraft:entity/zombie/drowned_outer_layer";
pub const STRAY: &str = "minecraft:entity/skeleton/stray";
const STRAY_OVERLAY: &str = "minecraft:entity/skeleton/stray_overlay";
pub const VILLAGER: &str = "minecraft:entity/villager/villager";
const WOLF_COLLAR: &str = "minecraft:entity/wolf/wolf_collar";
pub const ZOMBIE_VILLAGER: &str = "minecraft:entity/zombie_villager/zombie_villager";
pub const CAVE_SPIDER: &str = "minecraft:entity/spider/cave_spider";
pub const IRON_GOLEM: &str = "minecraft:entity/iron_golem/iron_golem";
pub const SQUID: &str = "minecraft:entity/squid/squid";
pub const GLOW_SQUID: &str = "minecraft:entity/squid/glow_squid";
pub const SQUID_BABY: &str = "minecraft:entity/squid/squid_baby";
pub const GLOW_SQUID_BABY: &str = "minecraft:entity/squid/glow_squid_baby";
pub const OCELOT: &str = "minecraft:entity/cat/ocelot";
pub const OCELOT_BABY: &str = "minecraft:entity/cat/ocelot_baby";
pub const PHANTOM: &str = "minecraft:entity/phantom/phantom";
const PHANTOM_EYES: &str = "minecraft:entity/phantom/phantom_eyes";
const CAT_COLLAR: &str = "minecraft:entity/cat/cat_collar";
const CAT_COLLAR_BABY: &str = "minecraft:entity/cat/cat_collar_baby";
pub const BOGGED: &str = "minecraft:entity/skeleton/bogged";
const BOGGED_OVERLAY: &str = "minecraft:entity/skeleton/bogged_overlay";
pub const SNOW_GOLEM: &str = "minecraft:entity/snow_golem/snow_golem";
pub const MAGMA_CUBE: &str = "minecraft:entity/slime/magmacube";
pub const BLAZE: &str = "minecraft:entity/blaze/blaze";
pub const GHAST: &str = "minecraft:entity/ghast/ghast";
pub const GHAST_SHOOTING: &str = "minecraft:entity/ghast/ghast_shooting";
pub const SILVERFISH: &str = "minecraft:entity/silverfish/silverfish";
pub const ENDERMITE: &str = "minecraft:entity/endermite/endermite";
pub const POLAR_BEAR: &str = "minecraft:entity/bear/polarbear";
pub const GOAT: &str = "minecraft:entity/goat/goat";
pub const PIGLIN: &str = "minecraft:entity/piglin/piglin";
pub const PIGLIN_BRUTE: &str = "minecraft:entity/piglin/piglin_brute";
pub const PIGLIN_BABY: &str = "minecraft:entity/piglin/piglin_baby";
pub const ZOMBIFIED_PIGLIN: &str = "minecraft:entity/piglin/zombified_piglin";
pub const HOGLIN: &str = "minecraft:entity/hoglin/hoglin";
pub const ZOGLIN: &str = "minecraft:entity/hoglin/zoglin";
pub const STRIDER: &str = "minecraft:entity/strider/strider";
pub const STRIDER_COLD: &str = "minecraft:entity/strider/strider_cold";
pub const ZOMBIFIED_PIGLIN_BABY: &str = "minecraft:entity/piglin/zombified_piglin_baby";
pub const HOGLIN_BABY: &str = "minecraft:entity/hoglin/hoglin_baby";
pub const ZOGLIN_BABY: &str = "minecraft:entity/hoglin/zoglin_baby";
pub const STRIDER_BABY: &str = "minecraft:entity/strider/strider_baby";
pub const STRIDER_COLD_BABY: &str = "minecraft:entity/strider/strider_cold_baby";
pub const ZOMBIE_VILLAGER_BABY: &str = "minecraft:entity/zombie_villager/zombie_villager_baby";
pub const GOAT_BABY: &str = "minecraft:entity/goat/goat_baby";
pub const POLAR_BEAR_BABY: &str = "minecraft:entity/bear/polarbear_baby";
pub const WITHER_SKELETON: &str = "minecraft:entity/skeleton/wither_skeleton";
pub const BAT: &str = "minecraft:entity/bat/bat";
pub const CAMEL: &str = "minecraft:entity/camel/camel";
pub const CAMEL_BABY: &str = "minecraft:entity/camel/camel_baby";
pub const ARMADILLO: &str = "minecraft:entity/armadillo/armadillo";
pub const ARMADILLO_BABY: &str = "minecraft:entity/armadillo/armadillo_baby";
/// `Rabbit.Variant`'s textures (`RabbitRenderer.RABBIT_LOCATIONS`), by id: 0 to 5, then
/// `EVIL` (99), the killer bunny.
pub const RABBIT_VARIANTS: [&str; 7] = [
    "brown",
    "white",
    "black",
    "white_splotched",
    "gold",
    "salt",
    "caerbannog",
];

/// Which model an entity is drawn with, and what about it changes the model.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    /// `PlayerModel`, with slim (three-pixel) arms or wide ones, crouching while it's in
    /// the crouching pose (`HumanoidRenderState.isCrouching`).
    Player {
        slim: bool,
        crouching: bool,
    },
    /// `ZombieModel`: arms held out, higher when it's aggressive (`Mob.isAggressive`).
    Zombie {
        aggressive: bool,
    },
    Skeleton,
    Creeper,
    Spider,
    Cow,
    Pig,
    /// The wool's `DyeColor` id, and whether it's been sheared.
    Sheep {
        color: u8,
        sheared: bool,
    },
    Chicken,
    /// `EndermanModel`: screaming with its jaw dropped (`isCreepy`), and whether it
    /// carries a block (the block's item is its main hand's).
    Enderman {
        creepy: bool,
        carried: Option<u32>,
    },
    /// `SlimeModel`: its size, and how far it's squashed (`squish`) this frame.
    Slime {
        size: f32,
        squish: f32,
    },
    /// `VillagerModel`, shaking its head while it's unhappy. Its type's, profession's and
    /// level's textures are the entity's `layers`.
    Villager {
        unhappy: bool,
    },
    /// `WitchModel`: its entity id, which sets how fast its nose twitches, and whether it
    /// holds something.
    Witch {
        id: i32,
        holding: bool,
    },
    /// `AdultWolfModel`: angry, sitting, its tail's angle (`getTailAngle`), and its
    /// collar's `DyeColor` once it's tamed.
    Wolf {
        angry: bool,
        sitting: bool,
        tail: f32,
        collar: Option<u8>,
    },
    /// `ZombieModel` at 1.0625 times the size (`HuskRenderer`).
    Husk {
        aggressive: bool,
    },
    /// `DrownedModel`, with its outer layer.
    Drowned {
        aggressive: bool,
    },
    /// `SkeletonModel`, with its clothes over it (`SkeletonClothingLayer`).
    Stray,
    /// `ZombieVillagerModel`: a zombie with a villager's head and robe. Its type's,
    /// profession's and level's textures are the entity's `layers`.
    ZombieVillager {
        aggressive: bool,
    },
    /// `SpiderModel` at 0.7 times the size.
    CaveSpider,
    /// `IronGolemModel`: ticks left of its attack swing (`attackTicksRemaining`), and its
    /// `Crackiness.Level` (0 none to 3 high).
    IronGolem {
        attack: f32,
        cracks: u8,
    },
    /// `SquidModel`: its tentacles' angle, and its body's tilt and spin in degrees
    /// (`xBodyRot`, `zBodyRot`).
    Squid {
        tentacles: f32,
        tilt: f32,
        spin: f32,
    },
    /// `PhantomModel`: `flapTime`, and its size (`getPhantomSize`).
    Phantom {
        flap: f32,
        size: f32,
    },
    /// `AdultCatModel`: its pose, how far it's lain down (`lieDownAmount`, and its tail's)
    /// and how far it's lifted its head (`relaxStateOneAmount`), and its collar once tame.
    Cat {
        pose: FelinePose,
        lie: f32,
        lie_tail: f32,
        relax: f32,
        collar: Option<u8>,
    },
    /// `AdultOcelotModel`: a cat's model and poses, crouching or sprinting (it never sits
    /// or lies down), without a collar.
    Ocelot {
        pose: FelinePose,
    },
    /// `AdultFoxModel`: `Fox`'s flags (1 sitting, 4 crouching, 16 pouncing, 32 sleeping,
    /// 64 faceplanted), how far it's crouched (`crouchAmount`), and its head's tilt while
    /// it's interested (`headRollAngle`).
    Fox {
        flags: u8,
        crouch: f32,
        roll: f32,
    },
    /// `BoggedModel`: a skeleton with mushrooms on its head until it's sheared, and its
    /// clothes over it.
    Bogged {
        sheared: bool,
    },
    /// `SnowGolemModel`, with a carved pumpkin's block state on its head if it has one.
    SnowGolem {
        pumpkin: Option<u32>,
    },
    /// `MagmaCubeModel`: its size and squish, as a slime's.
    MagmaCube {
        size: f32,
        squish: f32,
    },
    /// `BlazeModel`: its rods circle its body.
    Blaze,
    /// `GhastModel`: tentacles that sway.
    Ghast,
    /// `SilverfishModel` and `EndermiteModel`: segments that wriggle.
    Silverfish,
    Endermite,
    /// `PolarBearModel`: how far it's reared up (`standScale`).
    PolarBear {
        stand: f32,
    },
    /// `GoatModel`: whether it has its left and right horns.
    Goat {
        horns: [bool; 2],
    },
    /// `AdultPiglinModel` (piglins and brutes), posed by its `PiglinArmPose`.
    Piglin {
        pose: PiglinPose,
    },
    /// `AdultZombifiedPiglinModel`: a piglin with a zombie's arms.
    ZombifiedPiglin {
        aggressive: bool,
    },
    /// `HoglinModel` (hoglins and zoglins): ticks left of its headbutt
    /// (`attackAnimationRemainingTicks`).
    Hoglin {
        attack: i32,
    },
    /// `AdultStriderModel`.
    Strider,
    /// `SkeletonModel` at 1.2 times the size.
    WitherSkeleton,
    /// `BatModel`: whether it's resting (hanging upside down, its head turned with its
    /// look), and ticks since its flying or resting animation started, whichever is
    /// running (`flyAnimationState`, `restAnimationState`).
    Bat {
        resting: bool,
        fly: Option<f32>,
        rest: Option<f32>,
    },
    /// `AdultRabbitModel` and `BabyRabbitModel`: ticks since its hop and its idle head
    /// tilt started, while they run (`hopAnimationState`, `idleHeadTiltAnimationState`).
    Rabbit {
        hop: Option<f32>,
        tilt: Option<f32>,
    },
    /// `FrogModel`: ticks since its jump, croak, tongue and idle-in-water animations
    /// started, while they run, and whether it's in water, where it swims rather than
    /// walks.
    Frog {
        anims: [Option<f32>; 4],
        swimming: bool,
    },
    /// `AdultCamelModel` and `BabyCamelModel`: ticks since its sit, sitting pose, stand
    /// up, idle and dash animations started, while they run.
    Camel {
        anims: [Option<f32>; 5],
    },
    /// `AdultArmadilloModel` and `BabyArmadilloModel`: hidden in its shell
    /// (`shouldHideInShell`), and ticks since its roll out, roll up and peek animations
    /// started, while they run.
    Armadillo {
        hiding: bool,
        anims: [Option<f32>; 3],
    },
}

/// `PiglinArmPose`, as far as it's posed: dancing, holding a melee weapon high while
/// aggressive, admiring the item in its off hand, or charging a crossbow (how far, 0 to 1).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PiglinPose {
    Default,
    Dancing,
    Melee,
    Admiring,
    CrossbowCharge(f32),
    /// Holding a loaded crossbow.
    CrossbowHold,
}

/// `FelineRenderState`'s poses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FelinePose {
    Standing,
    Crouching,
    Sprinting,
    Sitting,
}

/// A living entity's render state (`LivingEntityRenderState`), already interpolated to
/// the frame.
#[derive(Clone, Debug, PartialEq)]
pub struct Living {
    pub kind: Kind,
    /// The texture's sprite id.
    pub texture: String,
    /// Feet, in world space.
    pub pos: Vec3,
    /// `bodyRot`, the head's yaw relative to it, and the head's pitch, in degrees.
    pub body_yaw: f32,
    pub head_yaw: f32,
    pub pitch: f32,
    /// `WalkAnimationState`'s position and speed.
    pub walk_pos: f32,
    pub walk_speed: f32,
    /// Ticks since the entity was added, for arms' idle bob.
    pub age: f32,
    /// A chicken's wing flap angle and speed.
    pub flap: f32,
    pub flap_speed: f32,
    pub baby: bool,
    /// The name over its head, if it shows one.
    pub name_tag: Option<NameTag>,
    /// The armour it wears and the items it holds; only humanoids draw them.
    pub equipment: Equipment,
    /// The swing its arm is in, if any.
    pub swing: Option<Swing>,
    /// Its bounding box's width and height while it's on fire, for the flames around it.
    pub burning: Option<[f32; 2]>,
    /// Flashing red, as it does while hurt or dying (`hasRedOverlay`).
    pub hurt: bool,
    /// Ticks since it died, to the frame (`deathTime`), or 0 while it lives.
    pub death_time: f32,
    /// More textures drawn over the body on the same model: a villager's type, profession
    /// and level (`VillagerProfessionLayer`).
    pub layers: Vec<String>,
}

/// What a humanoid wears and holds (`HumanoidRenderState`'s equipment and hand items).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Equipment {
    /// Head, chest, legs and feet.
    pub armor: [Option<Armor>; 4],
    /// The items in its right and left hands, by name, such as `minecraft:iron_sword`.
    pub hands: [Option<String>; 2],
    /// Whether its main hand is its left.
    pub left_handed: bool,
    /// The item it's using, by arm, if any.
    pub using: Option<Using>,
}

/// `ItemUseAnimation`, as far as it's drawn: what using an item looks like.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UseAnim {
    None,
    Eat,
    Drink,
    /// A shield raised.
    Block,
    /// A bow drawn.
    Bow,
}

/// An item being used (`LivingEntity.useItem`), already interpolated to the frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Using {
    /// In first person, the main (0) or off (1) hand; in third person, the right (0) or
    /// left (1) arm.
    pub hand: usize,
    pub anim: UseAnim,
    /// `getUseDuration`.
    pub duration: i32,
    /// The ticks left, to the frame: `useItemRemainingTicks - partialTicks + 1`.
    pub remaining: f32,
    /// Whole ticks used (`UseDuration`), which picks a bow's pulling model.
    pub ticks: i32,
}

/// The icon to draw `item` with in `hand`: what it shows while used, if it's the item in
/// use and changes then (`minecraft:using_item`), or its own.
fn held_icon<'a>(
    models: &'a crate::bake::BlockModels,
    item: &str,
    hand: usize,
    using: Option<Using>,
) -> Option<&'a ItemIcon> {
    let used = using
        .filter(|u| u.hand == hand)
        .and_then(|u| Some(models.hud.using.get(item)?.pick(u.ticks)));
    used.or_else(|| models.hud.items.get(item))
}

/// An arm swinging (`ArmedEntityRenderState.currentSwing` and `swingAnimation`): which arm,
/// its `SwingAnimationType` (none 0, whack 1, stab 2), and how far through it is, 0 to 1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Swing {
    pub left: bool,
    pub kind: u8,
    pub progress: f32,
}

/// A piece of armour: its equipment asset (`equippable`'s `asset_id`, such as
/// `minecraft:diamond`) and its `dyed_color`, if any.
#[derive(Clone, Debug, PartialEq)]
pub struct Armor {
    pub asset: String,
    pub dye: Option<u32>,
}

/// An equipment asset's layers (`EquipmentClientInfo`) for humanoids and for leggings:
/// each layer's atlas sprite, and for dyeable layers the colour when undyed (0 for none,
/// which hides the layer until it's dyed).
#[derive(Debug, Default)]
pub struct EquipmentLayers {
    pub humanoid: Vec<(String, Option<u32>)>,
    pub leggings: Vec<(String, Option<u32>)>,
}

/// Every equipment asset in `assets/*/equipment`, by id, and the sprites their humanoid
/// layers need. Without the folder (unpacked by a launcher older than preview 12), there
/// are none, and armour isn't drawn.
pub(crate) fn equipment_assets(
    res: &mc_assets::Resources,
) -> anyhow::Result<(HashMap<String, EquipmentLayers>, Vec<String>)> {
    let mut assets = HashMap::new();
    let mut sprites = Vec::new();
    let dir = res.asset_path("equipment", "minecraft:x", "json");
    let dir = dir.parent().expect("a file in a folder");
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            eprintln!("no {}, so armour isn't drawn", dir.display());
            return Ok((assets, sprites));
        }
        Err(e) => {
            return Err(anyhow::Error::new(e).context(format!("reading {}", dir.display())));
        }
    };
    for entry in entries {
        let path = entry?.path();
        let Some(name) = path.file_stem().and_then(|n| n.to_str()) else {
            continue;
        };
        let json: serde_json::Value = serde_json::from_slice(&std::fs::read(&path)?)?;
        let mut layers = EquipmentLayers::default();
        for (kind, out) in [
            ("humanoid", &mut layers.humanoid),
            ("humanoid_leggings", &mut layers.leggings),
        ] {
            for layer in json["layers"][kind].as_array().into_iter().flatten() {
                let Some(texture) = layer["texture"].as_str() else {
                    continue;
                };
                let (ns, path) = mc_assets::split_id(texture);
                let sprite = format!("{ns}:entity/equipment/{kind}/{path}");
                let dyeable = layer.get("dyeable").map(|d| {
                    d["color_when_undyed"]
                        .as_i64()
                        .map_or(0, |c| c as u32 & 0xff_ffff | 0xff00_0000)
                });
                sprites.push(sprite.clone());
                out.push((sprite, dyeable));
            }
        }
        assets.insert(format!("minecraft:{name}"), layers);
    }
    Ok((assets, sprites))
}

/// A name shown over an entity (`EntityRenderState.nameTag`).
#[derive(Clone, Debug, PartialEq)]
pub struct NameTag {
    pub text: String,
    /// The `NAME_TAG` attachment's height above the feet: the entity's height in its pose.
    pub height: f32,
    /// `isDiscrete` (sneaking): the tag is fainter, hidden by walls, and shown only within
    /// 32 blocks.
    pub discrete: bool,
}

/// An item lying on the ground (`ItemEntityRenderState`).
#[derive(Clone, Debug, PartialEq)]
pub struct Dropped {
    /// The item's name, such as `minecraft:stone`.
    pub item: String,
    pub pos: Vec3,
    pub age: f32,
    /// `ItemEntity.bobOffs`: a random phase for the bob and spin, in radians.
    pub bob_offset: f32,
    /// Its bounding box's width and height while it's on fire, for the flames around it.
    pub burning: Option<[f32; 2]>,
}

#[derive(Clone, Debug, PartialEq)]
// A frame's few dozen entities, built and dropped each frame: boxing would cost more.
#[allow(clippy::large_enum_variant)]
pub enum Entity {
    Living(Living),
    Item(Dropped),
    Arrow(Arrow),
    Thrown(Thrown),
}

/// An arrow (`ArrowRenderState`), already interpolated to the frame.
#[derive(Clone, Debug, PartialEq)]
pub struct Arrow {
    pub pos: Vec3,
    /// `yRot` and `xRot` as arrows keep them: the way it flies, `atan2(x, z)`, and how
    /// steeply it climbs, in degrees.
    pub yaw: f32,
    pub pitch: f32,
    /// `shakeTime - partialTicks`: it quivers for 7 ticks after it sticks.
    pub shake: f32,
    /// One of [`ARROW`].
    pub texture: &'static str,
}

/// A thrown item, such as a snowball or ender pearl (`ThrownItemRenderState`).
#[derive(Clone, Debug, PartialEq)]
pub struct Thrown {
    /// The item it shows, such as `minecraft:snowball`.
    pub item: String,
    pub pos: Vec3,
    /// Ticks since it was added; it's hidden for two while it's near the camera.
    pub age: u32,
}

/// An entity and the sky and block light (0 to 15) where it is.
pub type Lit = (Entity, [u8; 2]);

/// Entity quads, four vertices each.
#[derive(Default)]
pub struct Meshes {
    /// Bodies, to draw with the cutout pipeline.
    pub cutout: Vec<Vertex>,
    /// Glowing layers such as spiders' eyes (`RenderTypes.eyes`), to blend over them at
    /// full brightness without writing depth.
    pub eyes: Vec<Vertex>,
    /// Name tags' backgrounds and faint text (`DisplayMode.SEE_THROUGH`), to blend over
    /// everything drawn so far, walls included, without testing depth.
    pub see_through: Vec<Vertex>,
    /// Name tags' text, to draw opaque over `see_through` where nothing hides it.
    pub tags: Vec<Vertex>,
    /// Lightning bolts (from `weather::lightning`), added to what's behind them
    /// (`RenderPipelines.LIGHTNING`).
    pub lightning: Vec<Vertex>,
    /// Translucent particles such as potion swirls (`TRANSLUCENT_PARTICLE`), blended over
    /// the world without writing depth.
    pub particles: Vec<Vertex>,
}

/// Quads for these entities as seen from `camera`, with their name tags unless the HUD is
/// hidden.
pub fn vertices(
    models: &crate::bake::BlockModels,
    camera: &crate::gpu::Camera,
    name_tags: bool,
    entities: &[Lit],
) -> Meshes {
    let (atlas, art) = (&models.atlas, &models.hud);
    let fire = ["minecraft:block/fire_0", "minecraft:block/fire_1"].map(|s| atlas.sprite(s).rect);
    let mut out = Meshes::default();
    for (entity, light) in entities {
        match entity {
            Entity::Living(l) => {
                // `AvatarRenderer.getRenderOffset`: a crouching player, and its name, are
                // drawn two pixels lower.
                let lowered;
                let l = if matches!(
                    l.kind,
                    Kind::Player {
                        crouching: true,
                        ..
                    }
                ) {
                    lowered = Living {
                        pos: l.pos - Vec3::Y * 2.0 / 16.0,
                        ..l.clone()
                    };
                    &lowered
                } else {
                    l
                };
                living(models, l, *light, camera.eye, &mut out);
                equipment(models, l, *light, &mut out.cutout);
                if let Some(tag) = l.name_tag.as_ref().filter(|_| name_tags) {
                    crate::hud::name_tag(art, tag, l.pos, *light, camera, &mut out);
                }
                if let Some(size) = l.burning {
                    flames(fire, l.pos, size, camera.yaw, *light, &mut out.cutout);
                }
            }
            Entity::Arrow(a) => arrow(atlas, a, camera, *light, &mut out.cutout),
            // `ThrowableProjectile.shouldRenderAtSqrDistance`: not in the thrower's face as
            // it leaves their hand.
            Entity::Thrown(t) if t.age < 2 && t.pos.distance_squared(camera.eye) < 12.25 => {}
            Entity::Thrown(t) => {
                if let Some(icon) = art.items.get(&t.item) {
                    // `ThrownItemRenderer.submit`: turned to face the camera, then placed as
                    // an item on the ground is.
                    let m = Mat4::from_quat(camera.rotation()) * ground_transform(icon);
                    for (corners, uv, tint) in item_quads(atlas, icon) {
                        let pos = corners.map(|c| m.transform_point3(c));
                        emit_quad(pos, uv, t.pos, tint, *light, &mut out.cutout);
                    }
                }
            }
            Entity::Item(d) => {
                if let Some(icon) = art.items.get(&d.item) {
                    dropped(atlas, icon, d, *light, &mut out.cutout);
                }
                if let Some(size) = d.burning {
                    flames(fire, d.pos, size, camera.yaw, *light, &mut out.cutout);
                }
            }
        }
    }
    out
}

/// `FlameFeatureRenderer`: fire around a burning entity whose bounding box is `width` by
/// `height`, as cards of `fire_0` and `fire_1` (their atlas rects) stacked up it, each a little narrower and
/// nearer the camera than the last, turned to face a camera looking along `yaw`. Lit at
/// full block light (its light already is, for a burning entity), and shaded as faces
/// pointing up are, which is not at all.
fn flames(
    fire: [[f32; 4]; 2],
    pos: Vec3,
    [width, height]: [f32; 2],
    yaw: f32,
    [sky, block]: [u8; 2],
    out: &mut Vec<Vertex>,
) {
    let s = width * 1.4;
    let mut h = height / s;
    // `Mth.rotationAroundAxis(Y_AXIS, camera.orientation)`: the camera's turn about y.
    let m = Mat4::from_translation(pos)
        * Mat4::from_scale(Vec3::splat(s))
        * Mat4::from_rotation_y(PI - yaw.to_radians())
        * Mat4::from_translation(Vec3::new(0.0, 0.0, 0.3 - h.trunc() * 0.02));
    let (mut r, mut y, mut z) = (0.5, 0.0, 0.0);
    let mut i = 0;
    while h > 0.0 {
        let [mut u0, v0, mut u1, v1] = fire[i % 2];
        if i / 2 % 2 == 0 {
            std::mem::swap(&mut u0, &mut u1);
        }
        let corners = [
            ([-r, y, z], [u1, v1]),
            ([r, y, z], [u0, v1]),
            ([r, y + 1.4, z], [u0, v0]),
            ([-r, y + 1.4, z], [u1, v0]),
        ];
        for (p, uv) in corners {
            out.push(Vertex {
                pos: m.transform_point3(Vec3::from(p)).to_array(),
                uv,
                color: [255; 4],
                light: [sky.min(15) * 16, block.min(15) * 16, 0, 0],
            });
        }
        h -= 0.45;
        y += 0.45;
        r *= 0.9;
        z -= 0.03;
        i += 1;
    }
}

/// `LivingEntityRenderer.submit`'s transform, without the move to the entity's position:
/// turned to face `bodyRot`, fallen over as far as its death has got (`setupRotations`),
/// flipped into model space (y down, 16 units to a block), and lowered so the model's
/// feet, 24 pixels down, stand at the origin.
fn living_root(l: &Living, model_scale: f32) -> Mat4 {
    living_root_scaled(l, Vec3::splat(model_scale))
}

/// [`living_root`] with a scale along each axis, as `AbstractCubeMobRenderer.scale` squashes
/// slimes.
fn living_root_scaled(l: &Living, model_scale: Vec3) -> Mat4 {
    root_with(l, rotations(l), Mat4::from_scale(model_scale))
}

/// Whether this is a baby with its own model, drawn full size, rather than the adult's
/// at half size. Vanilla 26.3 gives every baby its own; so far the piglin's (which a
/// zombified piglin shares), the hoglin's (and zoglin's), the strider's, the zombie
/// villager's, the fox's, the goat's, the polar bear's, the cat's (and ocelot's), the
/// squid's, the rabbit's, the camel's and the armadillo's are here.
fn baby_model(l: &Living) -> bool {
    l.baby
        && matches!(
            l.kind,
            Kind::Piglin { .. }
                | Kind::ZombifiedPiglin { .. }
                | Kind::Hoglin { .. }
                | Kind::Strider
                | Kind::ZombieVillager { .. }
                | Kind::Fox { .. }
                | Kind::Goat { .. }
                | Kind::PolarBear { .. }
                | Kind::Cat { .. }
                | Kind::Ocelot { .. }
                | Kind::Squid { .. }
                | Kind::Rabbit { .. }
                | Kind::Camel { .. }
                | Kind::Armadillo { .. }
        )
}

/// Where a humanoid baby with its own model hangs its parts: the head's y, the body's y
/// and z, the shoulders' x and y, and the legs' y (`BabyPiglinModel`,
/// `BabyZombieVillagerModel`).
fn baby_humanoid(l: &Living) -> Option<[f32; 6]> {
    if !baby_model(l) {
        return None;
    }
    match l.kind {
        Kind::Piglin { .. } | Kind::ZombifiedPiglin { .. } => {
            Some([15.0, 18.0, -0.5, 4.0, 15.0, 20.0])
        }
        Kind::ZombieVillager { .. } => Some([16.0, 18.75, 0.0, 3.0, 15.5, 21.5]),
        _ => None,
    }
}

/// `LivingEntityRenderer.setupRotations`: turned to face `bodyRot`, and over onto its side
/// in the first 12.5 ticks after it dies; a spider rolls onto its back
/// (`SpiderRenderer.getFlipDegrees`).
fn rotations(l: &Living) -> Mat4 {
    let fall = if l.death_time > 0.0 {
        // `SilverfishRenderer` and `EndermiteRenderer.getFlipDegrees` too.
        let flip = if matches!(
            l.kind,
            Kind::Spider | Kind::CaveSpider | Kind::Silverfish | Kind::Endermite
        ) {
            180.0
        } else {
            90.0
        };
        ((l.death_time - 1.0) / 20.0 * 1.6).sqrt().min(1.0) * flip
    } else {
        0.0
    };
    Mat4::from_rotation_y((180.0 - l.body_yaw).to_radians())
        * Mat4::from_rotation_z(fall.to_radians())
}

/// The rest of `LivingEntityRenderer.submit`'s transform after `setupRotations`: flipped
/// into model space, the renderer's `scale`, and lowered to the model's feet.
fn root_with(l: &Living, rotations: Mat4, scale: Mat4) -> Mat4 {
    let baby = if l.baby && !baby_model(l) { 0.5 } else { 1.0 };
    Mat4::from_scale(Vec3::splat(baby))
        * rotations
        * Mat4::from_scale(Vec3::new(-1.0, -1.0, 1.0))
        * scale
        * Mat4::from_translation(Vec3::new(0.0, -1.501, 0.0))
}

fn living(
    models: &crate::bake::BlockModels,
    l: &Living,
    light: [u8; 2],
    eye: Vec3,
    out: &mut Meshes,
) {
    let atlas = &models.atlas;
    let start = out.cutout.len();
    let (nodes, tex_size, model_scale) = match l.kind {
        // `AvatarRenderer.scale`: players are drawn at 15/16 size.
        Kind::Player { slim, .. } => (player(l, slim), [64.0, 64.0], 0.9375),
        Kind::Zombie { aggressive } => (humanoid(l, HUMANOID, Some(aggressive)), [64.0; 2], 1.0),
        Kind::Skeleton => (humanoid(l, SKELETON_LIMBS, None), [64.0, 32.0], 1.0),
        Kind::Creeper => (creeper(l), [64.0, 32.0], 1.0),
        Kind::Spider => (spider(l), [64.0, 32.0], 1.0),
        Kind::Cow => (quadruped(l, &COW), [64.0, 64.0], 1.0),
        Kind::Pig => (quadruped(l, &PIG), [64.0, 64.0], 1.0),
        Kind::Sheep { .. } => (quadruped(l, &SHEEP_BODY), [64.0, 32.0], 1.0),
        Kind::Chicken => (chicken(l), [64.0, 32.0], 1.0),
        Kind::Enderman { creepy, carried } => {
            (enderman(l, creepy, carried.is_some()), [64.0, 32.0], 1.0)
        }
        Kind::Slime { .. } => (SLIME_INNER.to_vec(), [64.0, 32.0], 1.0),
        // `LayerDefinitions`' `villagerLikeScale`.
        Kind::Villager { unhappy } => (villager(l, unhappy, true), [64.0, 64.0], 0.9375),
        Kind::Witch { id, holding } => (witch(l, id, holding), [64.0, 128.0], 0.9375),
        Kind::Wolf {
            angry,
            sitting,
            tail,
            ..
        } => (wolf(l, angry, sitting, tail), [64.0, 32.0], 1.0),
        Kind::Husk { aggressive } => (humanoid(l, HUMANOID, Some(aggressive)), [64.0; 2], 1.0625),
        Kind::Drowned { aggressive } => {
            (humanoid(l, DROWNED_LIMBS, Some(aggressive)), [64.0; 2], 1.0)
        }
        Kind::Stray => (humanoid(l, SKELETON_LIMBS, None), [64.0, 32.0], 1.0),
        Kind::ZombieVillager { aggressive } => {
            (zombie_villager(l, aggressive, true), [64.0; 2], 1.0)
        }
        // `LayerDefinitions`' `MeshTransformer.scaling(0.7F)`.
        Kind::CaveSpider => (spider(l), [64.0, 32.0], 0.7),
        Kind::IronGolem { attack, .. } => (iron_golem(l, attack), [128.0; 2], 1.0),
        Kind::Squid { tentacles, .. } if l.baby => (baby_squid(tentacles), [32.0; 2], 1.0),
        Kind::Squid { tentacles, .. } => (squid(tentacles), [64.0, 32.0], 1.0),
        Kind::Phantom { flap, .. } => (phantom(flap), [64.0; 2], 1.0),
        // `BabyCatModel` and `BabyOcelotModel`, unscaled.
        Kind::Cat {
            pose,
            lie,
            lie_tail,
            relax,
            ..
        } if l.baby => (baby_feline(l, pose, lie, lie_tail, relax), [32.0; 2], 1.0),
        Kind::Ocelot { pose } if l.baby => (baby_feline(l, pose, 0.0, 0.0, 0.0), [32.0; 2], 1.0),
        Kind::Ocelot { pose } => (feline(l, &FELINE, pose, 0.0, 0.0, 0.0), [64.0, 32.0], 1.0),
        // `AdultCatModel.CAT_TRANSFORMER`.
        Kind::Cat {
            pose,
            lie,
            lie_tail,
            relax,
            ..
        } => (
            feline(l, &FELINE, pose, lie, lie_tail, relax),
            [64.0, 32.0],
            0.8,
        ),
        Kind::Fox {
            flags,
            crouch,
            roll,
        } if l.baby => (baby_fox(l, flags, crouch, roll), [32.0; 2], 1.0),
        Kind::Fox {
            flags,
            crouch,
            roll,
        } => (fox(l, flags, crouch, roll), [48.0, 32.0], 1.0),
        Kind::Bogged { sheared } => (bogged(l, sheared), [64.0, 32.0], 1.0),
        Kind::SnowGolem { .. } => (snow_golem(l), [64.0; 2], 1.0),
        Kind::MagmaCube { squish, .. } => (magma_cube(squish), [64.0; 2], 1.0),
        Kind::Blaze => (blaze(l), [64.0, 32.0], 1.0),
        // `GhastModel`'s `MeshTransformer.scaling(4.5F)`.
        Kind::Ghast => (ghast(l), [64.0, 32.0], 4.5),
        Kind::Silverfish => (silverfish(l), [64.0, 32.0], 1.0),
        Kind::Endermite => (endermite(l), [64.0, 32.0], 1.0),
        // `PolarBearModel`'s `MeshTransformer.scaling(1.2F)`.
        // `BabyPolarBearModel`'s layer isn't scaled.
        Kind::PolarBear { stand } if l.baby => (baby_polar_bear(l, stand), [64.0; 2], 1.0),
        Kind::PolarBear { stand } => (polar_bear(l, stand), [128.0, 64.0], 1.2),
        Kind::Goat { horns } if l.baby => (baby_goat(l, horns), [64.0; 2], 1.0),
        Kind::Goat { horns } => (goat(l, horns), [64.0; 2], 1.0),
        Kind::Piglin { .. } => (piglin(l, None), [64.0; 2], 1.0),
        Kind::ZombifiedPiglin { aggressive } => (piglin(l, Some(aggressive)), [64.0; 2], 1.0),
        Kind::Hoglin { attack } if l.baby => (baby_hoglin(l, attack), [64.0; 2], 1.0),
        Kind::Hoglin { attack } => (hoglin(l, attack), [128.0, 64.0], 1.0),
        Kind::Strider if l.baby => (baby_strider(l), [32.0; 2], 1.0),
        Kind::Strider => (strider(l), [64.0, 128.0], 1.0),
        // `LayerDefinitions`' `witherSkeletonScale`.
        Kind::WitherSkeleton => (humanoid(l, SKELETON_LIMBS, None), [64.0, 32.0], 1.2),
        Kind::Bat { resting, fly, rest } => (bat(l, resting, fly, rest), [32.0; 2], 1.0),
        Kind::Rabbit { hop, tilt } if l.baby => (baby_rabbit(l, hop, tilt), [32.0; 2], 1.0),
        Kind::Rabbit { hop, tilt } => (rabbit(l, hop, tilt), [64.0; 2], 1.0),
        Kind::Frog { anims, swimming } => (frog(l, anims, swimming), [48.0; 2], 1.0),
        Kind::Camel { anims } if l.baby => (baby_camel(l, anims), [64.0; 2], 1.0),
        Kind::Camel { anims } => (camel(l, anims), [128.0; 2], 1.0),
        Kind::Armadillo { hiding, anims } => (armadillo(l, hiding, anims), [64.0; 2], 1.0),
    };
    let root = match l.kind {
        // `AbstractCubeMobRenderer.scale`: `downscaleSlightly`, then the size, wider and
        // flatter as it lands and taller as it leaps.
        Kind::Slime { size, squish } => {
            let ss = squish / (size * 0.5 + 1.0);
            let w = 1.0 / (ss + 1.0);
            living_root_scaled(l, Vec3::new(w * size, size / w, w * size) * 0.999)
                * Mat4::from_translation(Vec3::new(0.0, 0.001 / (size / w), 0.0))
        }
        // `AbstractCubeMobRenderer.applySizeAndSquish`, without the slime's slight downscale.
        Kind::MagmaCube { size, squish } => {
            let w = 1.0 / (squish / (size * 0.5 + 1.0) + 1.0);
            living_root_scaled(l, Vec3::new(w * size, size / w, w * size))
        }
        // `SquidRenderer.setupRotations`, which neither turns with `bodyRot` alone nor
        // falls over: raised half a block, tilted and spun about there.
        Kind::Squid { tilt, spin, .. } => {
            // A baby's half the height.
            let (up, down) = if l.baby { (0.25, -0.6) } else { (0.5, -1.2) };
            let r = Mat4::from_translation(Vec3::Y * up)
                * Mat4::from_rotation_y((180.0 - l.body_yaw).to_radians())
                * Mat4::from_rotation_x(tilt.to_radians())
                * Mat4::from_rotation_y(spin.to_radians())
                * Mat4::from_translation(Vec3::Y * down);
            root_with(l, r, Mat4::IDENTITY)
        }
        // `PhantomRenderer.setupRotations` pitches it with its heading, and `scale` grows
        // it with its size and lifts it.
        Kind::Phantom { size, .. } => {
            let r = rotations(l) * Mat4::from_rotation_x(l.pitch.to_radians());
            let scale = Mat4::from_scale(Vec3::splat(1.0 + 0.15 * size))
                * Mat4::from_translation(Vec3::new(0.0, 1.3125, 0.1875));
            root_with(l, r, scale)
        }
        // `IronGolemRenderer.setupRotations`: it rocks side to side as it walks.
        Kind::IronGolem { .. } if l.walk_speed >= 0.01 => {
            let wave = ((l.walk_pos + 6.0) % 13.0 - 6.5).abs() - 3.25;
            let r = rotations(l) * Mat4::from_rotation_z((6.5 * wave / 3.25).to_radians());
            root_with(l, r, Mat4::from_scale(Vec3::splat(model_scale)))
        }
        // `CatRenderer.setupRotations`: lying down, it rolls onto its side.
        Kind::Cat { lie, .. } if lie > 0.0 => {
            let r = rotations(l)
                * Mat4::from_translation(Vec3::new(0.4, 0.15, 0.1) * lie)
                * Mat4::from_rotation_z((90.0 * lie).to_radians());
            root_with(l, r, Mat4::from_scale(Vec3::splat(model_scale)))
        }
        // `FoxRenderer.setupRotations`: pouncing or faceplanted, it dives along its pitch.
        Kind::Fox { flags, .. } if flags & (16 | 64) != 0 => {
            let r = rotations(l) * Mat4::from_rotation_x(-l.pitch.to_radians());
            root_with(l, r, Mat4::IDENTITY)
        }
        _ => living_root(l, model_scale),
    };
    let mut b = Builder {
        quads: Vec::new(),
        sprite: atlas.sprite(&l.texture),
        tex_size,
        tint: -1,
    };
    b.nodes(root, &nodes);
    // `RenderTypes.entityCutoutCull`: only the faces turned towards the camera, which
    // matters for a bat's flat wings and ears, whose two faces lie in one plane.
    if matches!(l.kind, Kind::Bat { .. }) {
        b.quads.retain(|q| {
            let normal = (q.pos[1] - q.pos[0]).cross(q.pos[2] - q.pos[0]);
            normal.dot(eye - (l.pos + q.pos[0])) > 0.0
        });
    }
    emit(&b.quads, l.pos, [1.0; 3], light, &mut out.cutout);
    // A layer: the same or another model's parts, with another texture.
    let layer = |sprite: &str, nodes: &[Node]| {
        let mut b = Builder {
            quads: Vec::new(),
            sprite: atlas.sprite(sprite),
            tex_size: [64.0, 32.0],
            tint: -1,
        };
        b.nodes(root, nodes);
        b.quads
    };
    // `RenderTypes.eyes`: glowing over the body, whatever the light.
    let eyes = |quads: Vec<Quad>, out: &mut Meshes| {
        for q in quads {
            for (p, uv) in q.pos.iter().zip(q.uv) {
                out.eyes.push(Vertex {
                    pos: (l.pos + *p).to_array(),
                    uv,
                    color: [255; 4],
                    light: [240, 240, 0, 0],
                });
            }
        }
    };
    // A layer on a model with a texture of this size.
    let layer_tex = |sprite: &str, nodes: &[Node], tex_size: [f32; 2]| {
        let mut b = Builder {
            quads: Vec::new(),
            sprite: atlas.sprite(sprite),
            tex_size,
            tint: -1,
        };
        b.nodes(root, nodes);
        b.quads
    };
    // A layer on a model with a 64 × 64 texture.
    let layer64 = |sprite: &str, nodes: &[Node]| {
        let mut b = Builder {
            quads: Vec::new(),
            sprite: atlas.sprite(sprite),
            tex_size: [64.0, 64.0],
            tint: -1,
        };
        b.nodes(root, nodes);
        b.quads
    };
    let mut collar = None;
    match l.kind {
        // `SpiderEyesLayer`.
        Kind::Spider | Kind::CaveSpider => eyes(layer(SPIDER_EYES, &nodes), out),
        // `SkeletonClothingLayer` with the bogged's overlay.
        Kind::Bogged { .. } => {
            let clothes = humanoid_parts(l, STRAY_CLOTHES_PARTS, STRAY_CLOTHES_LIMBS, None);
            let quads = layer(BOGGED_OVERLAY, &clothes);
            emit(&quads, l.pos, [1.0; 3], light, &mut out.cutout);
        }
        // `SnowGolemHeadLayer`: the pumpkin's block model on its head, five eighths the
        // size, facing forward, flashing red with it.
        Kind::SnowGolem {
            pumpkin: Some(state),
        } => {
            if let Some(state) = models.states.get(state as usize) {
                let [x, y, z, rx, ry, rz] = nodes[0].pose;
                let m = root
                    * crate::entity_model::pose([x, y, z], [rx, ry, rz])
                    * Mat4::from_translation(Vec3::new(0.0, -0.34375, 0.0))
                    * Mat4::from_rotation_y(PI)
                    * Mat4::from_scale(Vec3::new(0.625, -0.625, -0.625))
                    * Mat4::from_translation(Vec3::splat(-0.5));
                for (_, model) in state.parts.iter().filter_map(|p| p.first()) {
                    for q in &model.quads {
                        let pos = q.pos.map(|c| m.transform_point3(c));
                        emit_quad(pos, q.uv, l.pos, [1.0; 3], light, &mut out.cutout);
                    }
                }
            }
        }
        // `PhantomEyesLayer`.
        Kind::Phantom { .. } => eyes(layer64(PHANTOM_EYES, &nodes), out),
        // `IronGolemCrackinessLayer`: cracks as its health falls, flashing red with it.
        Kind::IronGolem { cracks, .. } if cracks > 0 => {
            let level = ["low", "medium", "high"][cracks.min(3) as usize - 1];
            let texture = format!("{IRON_GOLEM}_crackiness_{level}");
            let quads = layer_tex(&texture, &nodes, [128.0; 2]);
            emit(&quads, l.pos, [1.0; 3], light, &mut out.cutout);
        }
        // `EnderEyesLayer`, and `CarriedBlockLayer`: the block's model held out in front,
        // half size and turned to show a corner.
        Kind::Enderman { carried, .. } => {
            eyes(layer(ENDERMAN_EYES, &nodes), out);
            if let Some(state) = carried.and_then(|s| models.states.get(s as usize)) {
                let m = root
                    * Mat4::from_translation(Vec3::new(0.0, 0.6875, -0.75))
                    * Mat4::from_rotation_x(20f32.to_radians())
                    * Mat4::from_rotation_y(45f32.to_radians())
                    * Mat4::from_translation(Vec3::new(0.25, 0.1875, 0.25))
                    * Mat4::from_scale(Vec3::new(-0.5, -0.5, 0.5))
                    * Mat4::from_rotation_y(90f32.to_radians());
                for (_, model) in state.parts.iter().filter_map(|p| p.first()) {
                    for q in &model.quads {
                        let pos = q.pos.map(|c| m.transform_point3(c));
                        let tint = state.hand_tint.filter(|_| q.tint >= 0);
                        let rgb = tint.unwrap_or([1.0; 3]);
                        emit_quad(pos, q.uv, l.pos, rgb, light, &mut out.cutout);
                    }
                }
            }
        }
        // `SlimeOuterLayer`: the see-through outer cube over it.
        Kind::Slime { .. } => {
            let quads = layer(SLIME, SLIME_OUTER);
            emit(&quads, l.pos, [1.0; 3], light, &mut out.particles);
        }
        // `VillagerProfessionLayer`: the type's clothes, with its hat unless the
        // profession's covers it (`Hat`), then the profession's and its level's badge.
        Kind::Villager { .. } | Kind::ZombieVillager { .. } => {
            // The model without its head (`createNoHatModel`, `createNoHatLayer`).
            let headless = || match l.kind {
                Kind::Villager { unhappy } => villager(l, unhappy, false),
                Kind::ZombieVillager { aggressive } => zombie_villager(l, aggressive, false),
                _ => unreachable!(),
            };
            let hat = |i: usize| {
                l.layers
                    .get(i)
                    .and_then(|t| models.villager_hats.get(t))
                    .copied()
                    .unwrap_or(0)
            };
            let profession = if l.layers.len() > 1 { hat(1) } else { 0 };
            let type_hat = profession == 0 || (profession == 1 && hat(0) != 2);
            for (i, texture) in l.layers.iter().enumerate() {
                let nodes = if i == 0 && !type_hat {
                    headless()
                } else {
                    nodes.clone()
                };
                let quads = layer64(texture, &nodes);
                emit(&quads, l.pos, [1.0; 3], light, &mut out.cutout);
            }
        }
        // `DrownedOuterLayer`: the same model a quarter pixel out.
        Kind::Drowned { aggressive } => {
            let grown = humanoid_parts(
                l,
                DROWNED_OUTER_PARTS,
                DROWNED_OUTER_LIMBS,
                Some(aggressive),
            );
            let quads = layer64(DROWNED_OUTER, &grown);
            emit(&quads, l.pos, [1.0; 3], light, &mut out.cutout);
        }
        // `SkeletonClothingLayer`: a humanoid's model a quarter pixel out, posed as the
        // skeleton is.
        Kind::Stray => {
            let clothes = humanoid_parts(l, STRAY_CLOTHES_PARTS, STRAY_CLOTHES_LIMBS, None);
            let quads = layer(STRAY_OVERLAY, &clothes);
            emit(&quads, l.pos, [1.0; 3], light, &mut out.cutout);
        }
        // `WolfCollarLayer`, which doesn't flash red with the rest.
        Kind::Wolf {
            collar: Some(dye), ..
        } => collar = Some((layer(WOLF_COLLAR, &nodes), dye)),
        Kind::Sheep { color, sheared } => {
            // `SheepWoolUndercoatLayer`: a coloured sheep's coat under its wool, on the
            // body model, which still shows once it's sheared. Babies have none.
            if color != 0 && !l.baby {
                let quads = layer(SHEEP_UNDERCOAT, &nodes);
                emit(&quads, l.pos, wool_color(color), light, &mut out.cutout);
            }
            // `SheepWoolLayer`: the fur model over the body, tinted with the wool's colour.
            if !sheared {
                let quads = layer(SHEEP_WOOL, &quadruped(l, &SHEEP_FUR));
                emit(&quads, l.pos, wool_color(color), light, &mut out.cutout);
            }
        }
        // `CatCollarLayer`: the body model a hundredth of a pixel out, flashing with it.
        Kind::Cat {
            pose,
            lie,
            lie_tail,
            relax,
            collar: Some(dye),
        } => {
            let c = DYE_RGB[dye as usize & 15];
            let rgb = [16, 8, 0].map(|s| (c >> s & 0xFF) as f32 / 255.0);
            let quads = if l.baby {
                // `BabyCatModel.COLLAR_TRANSFORMER`: the baby model 1% bigger about its feet.
                let nodes = baby_feline(l, pose, lie, lie_tail, relax);
                let feet = Vec3::Y * 1.501;
                let mut b = Builder {
                    quads: Vec::new(),
                    sprite: atlas.sprite(CAT_COLLAR_BABY),
                    tex_size: [32.0; 2],
                    tint: -1,
                };
                let grown = root
                    * Mat4::from_translation(feet)
                    * Mat4::from_scale(Vec3::splat(1.01))
                    * Mat4::from_translation(-feet);
                b.nodes(grown, &nodes);
                b.quads
            } else {
                layer(
                    CAT_COLLAR,
                    &feline(l, &FELINE_COLLAR, pose, lie, lie_tail, relax),
                )
            };
            emit(&quads, l.pos, rgb, light, &mut out.cutout);
        }
        _ => {}
    }
    // `getOverlayCoords`: the body and the layers drawn as it is (the sheep's coats) flash;
    // glowing eyes don't.
    if l.hurt {
        for v in &mut out.cutout[start..] {
            v.light[3] |= crate::mesh::RED;
        }
    }
    if let Some((quads, dye)) = collar {
        let c = DYE_RGB[dye as usize & 15];
        let rgb = [16, 8, 0].map(|s| (c >> s & 0xFF) as f32 / 255.0);
        emit(&quads, l.pos, rgb, light, &mut out.cutout);
    }
}

/// `ColorLerper.Type.SHEEP.getColor`: white wool is a light grey; other colours are their
/// dye's texture colour at three quarters brightness.
fn wool_color(dye: u8) -> [f32; 3] {
    if dye == 0 {
        return [0xE6 as f32 / 255.0; 3];
    }
    let c = DYE_RGB[dye as usize & 15];
    [16, 8, 0].map(|s| ((c >> s & 0xFF) as f32 * 0.75).floor() / 255.0)
}

/// Vertices for model quads placed relative to `origin`, tinted by `color`.
fn emit(quads: &[Quad], origin: Vec3, color: [f32; 3], light: [u8; 2], out: &mut Vec<Vertex>) {
    for q in quads {
        emit_quad(q.pos, q.uv, origin, color, light, out);
    }
}

/// One quad's vertices, wound so its front faces out, shaded by which way that is.
fn emit_quad(
    pos: [Vec3; 4],
    uv: [[f32; 2]; 4],
    origin: Vec3,
    color: [f32; 3],
    [sky, block]: [u8; 2],
    out: &mut Vec<Vertex>,
) {
    let normal = (pos[1] - pos[0]).cross(pos[2] - pos[0]).normalize_or_zero();
    let shade = entity_shade(normal);
    let rgb = color.map(|c| (c * shade * 255.0).round().clamp(0.0, 255.0) as u8);
    for (p, uv) in pos.iter().zip(uv) {
        out.push(Vertex {
            pos: (origin + *p).to_array(),
            uv,
            color: [rgb[0], rgb[1], rgb[2], 255],
            light: [sky.min(15) * 16, block.min(15) * 16, 0, 0],
        });
    }
}

/// `minecraft_mix_light` with `Lighting`'s two level lights: 40% ambient, plus 60% of each
/// light the face turns towards.
fn entity_shade(normal: Vec3) -> f32 {
    let l0 = Vec3::new(0.2, 1.0, -0.7).normalize();
    let l1 = Vec3::new(-0.2, 1.0, 0.7).normalize();
    (0.4 + 0.6 * (normal.dot(l0).max(0.0) + normal.dot(l1).max(0.0))).min(1.0)
}

const fn mirrored(c: Cube) -> Cube {
    Cube { mirror: true, ..c }
}

const fn grown(c: Cube, grow: f32) -> Cube {
    Cube { grow, ..c }
}

/// A part's pose: offset in pixels, then rotation in radians.
fn part(
    parent: Option<usize>,
    [x, y, z]: [f32; 3],
    [rx, ry, rz]: [f32; 3],
    cubes: &'static [Cube],
) -> Node {
    Node {
        parent,
        pose: [x, y, z, rx, ry, rz],
        scale: [1.0; 3],
        cubes,
    }
}

/// `HumanoidModel.createMesh`'s head and hat.
const HEAD: &[Cube] = &[cube([-4.0, -8.0, -4.0], [8.0; 3], [0.0, 0.0])];
const HAT: &[Cube] = &[grown(cube([-4.0, -8.0, -4.0], [8.0; 3], [32.0, 0.0]), 0.5)];
const BODY: &[Cube] = &[cube([-4.0, 0.0, -2.0], [8.0, 12.0, 4.0], [16.0, 16.0])];

/// A humanoid's arms and legs: right arm, left arm, right leg, left leg, and the legs'
/// offset from the middle.
#[derive(Clone, Copy)]
struct Limbs {
    cubes: [&'static [Cube]; 4],
    leg_x: f32,
}

/// `HumanoidModel.createMesh`'s, used by zombies.
const HUMANOID: Limbs = Limbs {
    cubes: [
        &[cube([-3.0, -2.0, -2.0], [4.0, 12.0, 4.0], [40.0, 16.0])],
        &[mirrored(cube(
            [-1.0, -2.0, -2.0],
            [4.0, 12.0, 4.0],
            [40.0, 16.0],
        ))],
        &[cube([-2.0, 0.0, -2.0], [4.0, 12.0, 4.0], [0.0, 16.0])],
        &[mirrored(cube(
            [-2.0, 0.0, -2.0],
            [4.0, 12.0, 4.0],
            [0.0, 16.0],
        ))],
    ],
    leg_x: 1.9,
};

/// `SkeletonModel.createDefaultSkeletonMesh`: two pixels thick.
const SKELETON_LIMBS: Limbs = Limbs {
    cubes: [
        &[cube([-1.0, -2.0, -1.0], [2.0, 12.0, 2.0], [40.0, 16.0])],
        &[mirrored(cube(
            [-1.0, -2.0, -1.0],
            [2.0, 12.0, 2.0],
            [40.0, 16.0],
        ))],
        &[cube([-1.0, 0.0, -1.0], [2.0, 12.0, 2.0], [0.0, 16.0])],
        &[mirrored(cube(
            [-1.0, 0.0, -1.0],
            [2.0, 12.0, 2.0],
            [0.0, 16.0],
        ))],
    ],
    leg_x: 2.0,
};

/// `HumanoidModel.setupAnim` with empty hands, standing: the head looks, arms and legs
/// swing with the walk, and the arms bob (`AnimationUtils.bobModelPart`). A zombie's arms
/// are held out instead (`AnimationUtils.animateZombieArms`, with no attack swinging).
/// Returns the head, hat, body, right arm, left arm, right leg and left leg.
fn humanoid(l: &Living, limbs: Limbs, zombie: Option<bool>) -> Vec<Node> {
    humanoid_parts(l, [HEAD, HAT, BODY], limbs, zombie)
}

/// [`humanoid`] with these head, hat and body cubes, as armour models are posed.
fn humanoid_parts(
    l: &Living,
    [head, hat, body]: [&'static [Cube]; 3],
    limbs: Limbs,
    zombie: Option<bool>,
) -> Vec<Node> {
    let (p, s) = (l.walk_pos * 0.6662, l.walk_speed);
    let mut right_arm = [(p + PI).cos() * 2.0 * s * 0.5, 0.0, 0.0];
    let mut left_arm = [p.cos() * 2.0 * s * 0.5, 0.0, 0.0];
    // `HumanoidModel.poseRightArm` and `poseLeftArm`: a player's arm holding an item
    // (`ArmPose.ITEM`) swings half as far, raised a little; one raising a shield
    // (`BLOCK`) holds it up across the body, following the head a little; and drawing a
    // bow (`BOW_AND_ARROW`) raises both arms to aim where the head looks. Mobs' arms stay
    // `EMPTY`.
    if matches!(l.kind, Kind::Player { .. }) {
        let (head_x, head_y) = (l.pitch.to_radians(), l.head_yaw.to_radians());
        let using = l.equipment.using.filter(|u| u.remaining > 0.0);
        let bow = using.filter(|u| u.anim == UseAnim::Bow);
        for (i, (arm, held)) in [&mut right_arm, &mut left_arm]
            .into_iter()
            .zip(&l.equipment.hands)
            .enumerate()
        {
            let sign = if i == 0 { -1.0 } else { 1.0 };
            if bow.is_some() || held.is_none() {
                continue;
            }
            if using.is_some_and(|u| u.hand == i && u.anim == UseAnim::Block) {
                arm[0] = arm[0] * 0.5 - 0.9424779 + head_x.clamp(-PI * 4.0 / 9.0, 0.43633232);
                arm[1] = sign * 30f32.to_radians() + head_y.clamp(-PI / 6.0, PI / 6.0);
            } else {
                arm[0] = arm[0] * 0.5 - PI / 10.0;
                arm[1] = 0.0;
            }
        }
        if let Some(u) = bow {
            // The other arm reaches further across to the string.
            let reach = if u.hand == 0 { [0.0, 0.4] } else { [-0.4, 0.0] };
            right_arm[1] = -0.1 + head_y + reach[0];
            left_arm[1] = 0.1 + head_y + reach[1];
            right_arm[0] = -PI / 2.0 + head_x;
            left_arm[0] = -PI / 2.0 + head_x;
        }
    }
    // `HumanoidModel.setupAttackAnimation`: the body twists with the swing, carrying the
    // shoulders round, and a whack swings the arm down and across.
    // A baby's own model has its shoulders closer in, and a swing carries them round
    // `ageScale` (half) as far as an adult's.
    let rig = baby_humanoid(l);
    let (shoulder, reach) = rig.map_or((5.0, 5.0), |r| (r[3], 2.5));
    let (mut body_twist, mut right_pivot, mut left_pivot) =
        (0.0, [-shoulder, 0.0], [shoulder, 0.0]);
    let swing = l.swing.filter(|s| s.progress > 0.0);
    if let Some(sw) = swing {
        let t = sw.progress;
        body_twist = (t.sqrt() * PI * 2.0).sin() * 0.2 * if sw.left { -1.0 } else { 1.0 };
        let (sin, cos) = body_twist.sin_cos();
        right_pivot = [-cos * reach, sin * reach];
        left_pivot = [cos * reach, -sin * reach];
        right_arm[1] += body_twist;
        left_arm[1] += body_twist;
        left_arm[0] += body_twist;
        // `SwingAnimationType.WHACK`; stabbing (spears) isn't posed yet.
        if sw.kind == 1 {
            let arm = if sw.left {
                &mut left_arm
            } else {
                &mut right_arm
            };
            let out_quart = 1.0 - (1.0 - t).powi(4);
            let a = (out_quart * PI).sin();
            let b = (t * PI).sin() * -(l.pitch.to_radians() - 0.7) * 0.75;
            arm[0] -= a * 1.2 + b;
            arm[1] += body_twist * 2.0;
            arm[2] += (t * PI).sin() * -0.4;
        }
    }
    // `AnimationUtils.animateZombieArms`: both arms held out, higher when aggressive (but
    // down for a baby holding something), swinging together.
    if let Some(aggressive) = zombie {
        let t = swing.map_or(0.0, |s| s.progress);
        let raise = !l.baby || l.equipment.hands[l.equipment.left_handed as usize].is_none();
        let drop = if raise {
            -PI / if aggressive { 1.5 } else { 2.25 }
        } else {
            0.0
        };
        let ay = if raise { 1.0 } else { -1.0 } * (t * PI).sin();
        let ax = ((1.0 - (1.0 - t) * (1.0 - t)) * PI).sin();
        let (x, y) = (drop + ay * 1.2 - ax * 0.4, 0.1 - ay * 0.6);
        let sign = if raise { 1.0 } else { -1.0 };
        right_arm = [x, -y * sign, 0.0];
        left_arm = [x, y * sign, 0.0];
    }
    // `HumanoidModel.setupAnim`'s crouch: the body leans forward and drops with the arms,
    // the head drops further, and the legs move back.
    let crouching = matches!(
        l.kind,
        Kind::Player {
            crouching: true,
            ..
        }
    );
    let (head_y, body_y, leg_z, body_x) = if crouching {
        right_arm[0] += 0.4;
        left_arm[0] += 0.4;
        (4.2, 3.2, 4.0, 0.5)
    } else {
        (0.0, 0.0, 0.0, 0.0)
    };
    // Where the head, body, arms and legs hang from: a baby's own model has a short body
    // under its head.
    let (head_y, [body_y, body_z], arm_y, leg_y) = match rig {
        Some([head, body_y, body_z, _, arm, leg]) => (head, [body_y, body_z], arm, leg),
        None => (head_y, [body_y, 0.0], 2.0 + body_y, 12.0),
    };
    // `AnimationUtils.bobModelPart`, last.
    for (arm, sign) in [(&mut right_arm, 1.0), (&mut left_arm, -1.0)] {
        arm[2] += sign * ((l.age * 0.09).cos() * 0.05 + 0.05);
        arm[0] += sign * (l.age * 0.067).sin() * 0.05;
    }
    let [ra, la, rl, ll] = limbs.cubes;
    let mut nodes = vec![
        part(
            None,
            [0.0, head_y, 0.0],
            [l.pitch.to_radians(), l.head_yaw.to_radians(), 0.0],
            head,
        ),
        part(Some(0), [0.0; 3], [0.0; 3], hat),
        part(None, [0.0, body_y, body_z], [body_x, body_twist, 0.0], body),
        part(None, [right_pivot[0], arm_y, right_pivot[1]], right_arm, ra),
        part(None, [left_pivot[0], arm_y, left_pivot[1]], left_arm, la),
        part(
            None,
            [-limbs.leg_x, leg_y, leg_z],
            [p.cos() * 1.4 * s, 0.005, 0.005],
            rl,
        ),
        part(
            None,
            [limbs.leg_x, leg_y, leg_z],
            [(p + PI).cos() * 1.4 * s, -0.005, -0.005],
            ll,
        ),
    ];
    if let Kind::Piglin { pose } = l.kind {
        piglin_arms(l, pose, &mut nodes);
    }
    nodes
}

/// `PiglinModel.setupAnim`'s arm poses after `HumanoidModel`'s, and
/// `setupAttackAnimation`'s weapon swung down from high (`AnimationUtils.swingWeaponDown`),
/// on [`humanoid_parts`]' head, hat, body and arms.
fn piglin_arms(l: &Living, pose: PiglinPose, nodes: &mut [Node]) {
    let right_main = !l.equipment.left_handed;
    let t = l.age;
    let swing = l.swing.map_or(0.0, |s| s.progress);
    let (main, off) = if right_main { (3, 4) } else { (4, 3) };
    match pose {
        PiglinPose::Dancing => {
            let d = t / 60.0;
            nodes[0].pose[0] += (d * 10.0).sin();
            nodes[0].pose[1] += (d * 40.0).sin() + 0.4;
            let z = (70.0 + (d * 40.0).cos() * 10.0f32).to_radians();
            nodes[3].pose[5] = z;
            nodes[4].pose[5] = -z;
            nodes[3].pose[1] += (d * 40.0).sin() * 0.5 - 0.5;
            nodes[4].pose[1] += (d * 40.0).sin() * 0.5 + 0.5;
            nodes[2].pose[1] += (d * 40.0).sin() * 0.35;
        }
        PiglinPose::Melee if swing > 0.0 => {
            // In place of `HumanoidModel`'s swing: no body twist.
            nodes[2].pose[4] = 0.0;
            for (i, x) in [(3, -5.0), (4, 5.0)] {
                nodes[i].pose[0] = x;
                nodes[i].pose[2] = 0.0;
            }
            let a2 = (swing * PI).sin();
            let a = ((1.0 - (1.0 - swing) * (1.0 - swing)) * PI).sin();
            nodes[3].pose[3..6].copy_from_slice(&[0.0, PI / 20.0, 0.0]);
            nodes[4].pose[3..6].copy_from_slice(&[0.0, -PI / 20.0, 0.0]);
            nodes[main].pose[3] = -1.8849558 + (t * 0.09).cos() * 0.15 + a2 * 2.2 - a * 0.4;
            nodes[off].pose[3] = (t * 0.19).cos() * 0.5 + a2 * 1.2 - a * 0.4;
            // `AnimationUtils.bobArms`.
            for (i, sign) in [(3, 1.0), (4, -1.0)] {
                nodes[i].pose[5] += sign * ((t * 0.09).cos() * 0.05 + 0.05);
                nodes[i].pose[3] += sign * (t * 0.067).sin() * 0.05;
            }
        }
        PiglinPose::Melee => nodes[main].pose[3] = -1.8,
        // `AnimationUtils.animateCrossbowCharge`.
        PiglinPose::CrossbowCharge(f) => {
            let sign = if right_main { 1.0 } else { -1.0 };
            nodes[main].pose[4] = -0.8 * sign;
            nodes[main].pose[3] = -0.97079635;
            nodes[off].pose[4] = (0.4 + f * 0.45) * sign;
            nodes[off].pose[3] = -0.97079635 + f * (-PI / 2.0 + 0.97079635);
        }
        // `AnimationUtils.animateCrossbowHold`, aimed where the head looks.
        PiglinPose::CrossbowHold => {
            let sign = if right_main { 1.0 } else { -1.0 };
            let (pitch, yaw) = (nodes[0].pose[3], nodes[0].pose[4]);
            nodes[main].pose[4] = -0.3 * sign + yaw;
            nodes[off].pose[4] = 0.6 * sign + yaw;
            nodes[main].pose[3] = -PI / 2.0 + pitch + 0.1;
            nodes[off].pose[3] = -1.5 + pitch;
        }
        PiglinPose::Admiring => {
            nodes[0].pose[3] = 0.5;
            nodes[0].pose[4] = 0.0;
            let (arm, yaw) = if right_main { (4, 0.5) } else { (3, -0.5) };
            nodes[arm].pose[4] = yaw;
            nodes[arm].pose[3] = -0.9;
        }
        PiglinPose::Default => {}
    }
}

/// `HumanoidModel.createArmorMeshSet`, as `LayerDefinitions` builds it for humanoids and
/// players (64 × 32 textures): helmets and boots are 1 pixel out from the body
/// (`OUTER_ARMOR_DEFORMATION`), chestplates too, and leggings half a pixel
/// (`INNER_ARMOR_DEFORMATION`); legs are a tenth of a pixel thinner. Head, chest, legs and
/// feet: each the head, hat and body cubes and the limbs.
const ARMOR: [([&[Cube]; 3], Limbs); 4] = {
    const fn box_(from: [f32; 3], size: [f32; 3], tex: [f32; 2], grow: f32) -> Cube {
        grown(cube(from, size, tex), grow)
    }
    const RIGHT_ARM: Cube = cube([-3.0, -2.0, -2.0], [4.0, 12.0, 4.0], [40.0, 16.0]);
    const LEFT_ARM: Cube = mirrored(cube([-1.0, -2.0, -2.0], [4.0, 12.0, 4.0], [40.0, 16.0]));
    const RIGHT_LEG: Cube = cube([-2.0, 0.0, -2.0], [4.0, 12.0, 4.0], [0.0, 16.0]);
    const LEFT_LEG: Cube = mirrored(RIGHT_LEG);
    const NONE: &[Cube] = &[];
    const HEAD: [&[Cube]; 3] = [
        &[box_([-4.0, -8.0, -4.0], [8.0; 3], [0.0, 0.0], 1.0)],
        &[box_([-4.0, -8.0, -4.0], [8.0; 3], [32.0, 0.0], 1.5)],
        NONE,
    ];
    const OUTER_BODY: &[Cube] = &[box_([-4.0, 0.0, -2.0], [8.0, 12.0, 4.0], [16.0, 16.0], 1.0)];
    const INNER_BODY: &[Cube] = &[box_([-4.0, 0.0, -2.0], [8.0, 12.0, 4.0], [16.0, 16.0], 0.5)];
    const CHEST_ARMS: [&[Cube]; 2] = [&[grown(RIGHT_ARM, 1.0)], &[grown(LEFT_ARM, 1.0)]];
    const LEGGINGS: [&[Cube]; 2] = [&[grown(RIGHT_LEG, 0.4)], &[grown(LEFT_LEG, 0.4)]];
    const BOOTS: [&[Cube]; 2] = [&[grown(RIGHT_LEG, 0.9)], &[grown(LEFT_LEG, 0.9)]];
    const fn limbs(cubes: [&'static [Cube]; 4]) -> Limbs {
        Limbs { cubes, leg_x: 1.9 }
    }
    [
        (HEAD, limbs([NONE; 4])),
        (
            [NONE, NONE, OUTER_BODY],
            limbs([CHEST_ARMS[0], CHEST_ARMS[1], NONE, NONE]),
        ),
        (
            [NONE, NONE, INNER_BODY],
            limbs([NONE, NONE, LEGGINGS[0], LEGGINGS[1]]),
        ),
        ([NONE; 3], limbs([NONE, NONE, BOOTS[0], BOOTS[1]])),
    ]
};

/// For a humanoid that can wear armour and hold items: whether it's a zombie (aggressive
/// or not) as `humanoid` poses it, its model scale, and how far its hands sit out from its
/// arms' pivots (`translateToHand`: slim players' half a pixel, skeletons' a pixel). None
/// for other mobs.
fn humanoid_kind(l: &Living) -> Option<(Option<bool>, f32, f32)> {
    match l.kind {
        Kind::Player { slim, .. } => Some((None, 0.9375, if slim { 0.5 } else { 0.0 })),
        Kind::Zombie { aggressive } | Kind::Drowned { aggressive } => {
            Some((Some(aggressive), 1.0, 0.0))
        }
        Kind::Husk { aggressive } => Some((Some(aggressive), 1.0625, 0.0)),
        Kind::ZombieVillager { aggressive } => Some((Some(aggressive), 1.0, 0.0)),
        Kind::Skeleton | Kind::Stray | Kind::Bogged { .. } => Some((None, 1.0, 1.0)),
        Kind::WitherSkeleton => Some((None, 1.2, 1.0)),
        Kind::Piglin { .. } => Some((None, 1.0, 0.0)),
        Kind::ZombifiedPiglin { aggressive } => Some((Some(aggressive), 1.0, 0.0)),
        _ => None,
    }
}

/// `HumanoidArmorLayer` and `ItemInHandLayer`: a humanoid's armour, chest, legs, feet then
/// head, each of its equipment asset's layers tinted by its dye, on armour models posed as
/// the body is; then the items in its hands.
fn equipment(models: &crate::bake::BlockModels, l: &Living, light: [u8; 2], out: &mut Vec<Vertex>) {
    let Some((zombie, model_scale, hand_x)) = humanoid_kind(l) else {
        return;
    };
    let root = living_root(l, model_scale);
    // Babies with their own model wear `createBabyArmorMesh`'s armour, not drawn yet.
    let slots: &[usize] = if baby_model(l) { &[] } else { &[1, 2, 3, 0] };
    for &slot in slots {
        let Some(armor) = &l.equipment.armor[slot] else {
            continue;
        };
        let Some(asset) = models.equipment.get(&armor.asset) else {
            continue;
        };
        // `usesInnerModel`: leggings have their own layer textures.
        let layers = if slot == 2 {
            &asset.leggings
        } else {
            &asset.humanoid
        };
        let (parts, armor_limbs) = &ARMOR[slot];
        let nodes = humanoid_parts(l, *parts, *armor_limbs, zombie);
        for (sprite, dyeable) in layers {
            // `getColorForLayer`: a dyeable layer takes the dye, or its undyed colour;
            // with neither it's not drawn.
            let color = match dyeable {
                Some(undyed) => armor.dye.map_or(*undyed, |d| d | 0xff00_0000),
                None => 0xffff_ffff,
            };
            if color == 0 {
                continue;
            }
            let mut b = Builder {
                quads: Vec::new(),
                sprite: models.atlas.sprite(sprite),
                tex_size: [64.0, 32.0],
                tint: -1,
            };
            b.nodes(root, &nodes);
            let rgb = [16, 8, 0].map(|s| (color >> s & 0xff) as f32 / 255.0);
            emit(&b.quads, l.pos, rgb, light, out);
        }
    }
    // `ItemInHandLayer.submitArmWithItem`: from each hand, turned to point the item
    // forward, then placed by its model's hand transform.
    let nodes = humanoid_parts(l, [&[]; 3], ARMOR[3].1, zombie);
    for (i, held) in l.equipment.hands.iter().enumerate() {
        let using = l.equipment.using;
        let Some(icon) = held
            .as_ref()
            .and_then(|item| held_icon(models, item, i, using))
        else {
            continue;
        };
        let left = i == 1;
        let sign = if left { -1.0 } else { 1.0 };
        // `useBabyOffset`: closer in for a baby's short arms.
        let hand_offset = if l.baby {
            Vec3::new(0.0, 1.0, -4.5)
        } else {
            Vec3::new(sign, 2.0, -10.0)
        };
        let [x, y, z, arm_x, arm_y, arm_z] = nodes[3 + i].pose;
        let hand = root
            * crate::entity_model::pose([x + sign * hand_x, y, z], [arm_x, arm_y, arm_z])
            * Mat4::from_rotation_x(-PI / 2.0)
            * Mat4::from_rotation_y(PI)
            * Mat4::from_translation(hand_offset / 16.0);
        let m = hand * item_transform(icon.hand(left), left);
        for (corners, uv, tint) in item_quads(&models.atlas, icon) {
            emit_quad(
                corners.map(|c| m.transform_point3(c)),
                uv,
                l.pos,
                tint,
                light,
                out,
            );
        }
    }
}

/// `ItemTransform.apply`, mirrored for the left hand, then the model centred, as
/// `ItemStackRenderState.submit` places an item.
fn item_transform(t: mc_assets::Transform, left: bool) -> Mat4 {
    let [rx, ry, rz] = t.rotation.map(f32::to_radians);
    let [tx, ty, tz] = t.translation;
    let (tx, ry, rz) = if left { (-tx, -ry, -rz) } else { (tx, ry, rz) };
    Mat4::from_translation(Vec3::new(tx, ty, tz) / 16.0)
        * Mat4::from_quat(Quat::from_euler(glam::EulerRot::XYZ, rx, ry, rz))
        * Mat4::from_scale(Vec3::from(t.scale))
        * Mat4::from_translation(Vec3::splat(-0.5))
}

/// What the player sees of their own hands (`FirstPersonHandsAndItemsRenderState`),
/// already interpolated to the frame.
#[derive(Clone, Debug, PartialEq)]
pub struct FirstPerson {
    /// The player's skin sprite, and whether it has slim arms.
    pub skin: String,
    pub slim: bool,
    /// Whether their main hand is their left.
    pub left_handed: bool,
    /// The items shown in the main and off hand, by name; they lag the real ones while
    /// the hand dips to swap them.
    pub items: [Option<String>; 2],
    /// How far each hand is lowered, 0 (up) to 1 (out of sight): `1 - handHeight`.
    pub lowered: [f32; 2],
    /// The swing in progress, if any: which hand (0 main, 1 off), its `SwingAnimationType`
    /// (whack 1), and how far through it is, 0 to 1.
    pub swing: Option<(usize, u8, f32)>,
    /// How far the view has turned past where the hands have caught up to, in degrees
    /// (`viewXRot - xBob` and `viewYRot - yBob`); the hands lag a tenth of it.
    pub sway: [f32; 2],
    /// The item being used, by hand, while it has ticks left.
    pub using: Option<Using>,
}

/// `GameRenderer.renderItemInHand` and `FirstPersonHandsAndItemsRenderer.submitHandsWithItems`:
/// the player's arm and the items they hold, as world-space quads around `camera.eye`,
/// lit with `light`, for [`crate::gpu::Renderer::set_hand`] to draw over the world.
pub fn first_person(
    models: &crate::bake::BlockModels,
    camera: &crate::gpu::Camera,
    hands: &FirstPerson,
    light: [u8; 2],
) -> Vec<Vertex> {
    let mut out = Vec::new();
    // The pose stack starts as the inverse view rotation (`Camera.rotation`), so the
    // quads end up in world space and the world's entity lights shade them.
    let view = Mat4::from_quat(camera.rotation());
    let root = view
        * Mat4::from_rotation_x((hands.sway[0] * 0.1).to_radians())
        * Mat4::from_rotation_y((hands.sway[1] * 0.1).to_radians());
    let deg = |d: f32| d.to_radians();
    for (hand, item) in hands.items.iter().enumerate() {
        // `evaluateWhichHandsToRender`: drawing a bow shows only the hand drawing it.
        if hands
            .using
            .is_some_and(|u| u.anim == UseAnim::Bow && u.hand != hand)
        {
            continue;
        }
        // `submitArmWithItem`: which arm holds this hand, and its swing.
        let left = (hand == 1) != hands.left_handed;
        let invert = if left { -1.0 } else { 1.0 };
        let swing = hands.swing.filter(|&(h, ..)| h == hand);
        // `attackHand` is the main hand when nothing swings.
        let attack = match hands.swing {
            Some((h, _, progress)) if h == hand => progress,
            _ => 0.0,
        };
        let lowered = hands.lowered[hand];
        let Some(item) = item else {
            // `renderPlayerArm`: only an empty main hand shows the arm.
            if hand != 0 {
                continue;
            }
            let sqrt = attack.sqrt();
            let m =
                root * Mat4::from_translation(Vec3::new(
                    invert * (-0.3 * (sqrt * PI).sin() + 0.64),
                    0.4 * (sqrt * PI * 2.0).sin() - 0.6 + lowered * -0.6,
                    -0.4 * (attack * PI).sin() - 0.72,
                )) * Mat4::from_rotation_y(deg(invert * 45.0))
                    * Mat4::from_rotation_y(deg(invert * (sqrt * PI).sin() * 70.0))
                    * Mat4::from_rotation_z(deg(invert * (attack * attack * PI).sin() * -20.0))
                    * Mat4::from_translation(Vec3::new(-invert, 3.6, 3.5))
                    * Mat4::from_rotation_z(deg(invert * 120.0))
                    * Mat4::from_rotation_x(deg(200.0))
                    * Mat4::from_rotation_y(deg(invert * -135.0))
                    * Mat4::from_translation(Vec3::new(invert * 5.6, 0.0, 0.0));
            // `AvatarRenderer.renderHand`: the arm at its resting offset, tilted a little
            // out, with its sleeve.
            let limbs = if hands.slim { PLAYER_SLIM } else { PLAYER_WIDE };
            let arm = part(
                None,
                [invert * -5.0, 2.0, 0.0],
                [0.0, 0.0, invert * 0.1],
                limbs.cubes[left as usize],
            );
            let mut b = Builder {
                quads: Vec::new(),
                sprite: models.atlas.sprite(&hands.skin),
                tex_size: [64.0, 64.0],
                tint: -1,
            };
            b.nodes(m, &[arm]);
            emit(&b.quads, camera.eye, [1.0; 3], light, &mut out);
            continue;
        };
        let using = hands.using.filter(|u| u.hand == hand);
        let Some(icon) = held_icon(models, item, hand, using) else {
            continue;
        };
        // `applyItemArmTransform`, then `swingArm` for a whack in this hand, or how the
        // item in use is held.
        let arm = Mat4::from_translation(Vec3::new(invert * 0.56, -0.52 + lowered * -0.6, -0.72));
        let mut m = match using {
            Some(u) => match u.anim {
                UseAnim::Eat | UseAnim::Drink => root * eat_transform(u, invert) * arm,
                UseAnim::Bow => root * arm * bow_transform(u, invert),
                UseAnim::Block if item != "minecraft:shield" => {
                    root * arm
                        * Mat4::from_translation(Vec3::new(invert * -0.14142136, 0.08, 0.14142136))
                        * Mat4::from_rotation_x(deg(-102.25))
                        * Mat4::from_rotation_y(deg(invert * 13.365))
                        * Mat4::from_rotation_z(deg(invert * 78.05))
                }
                UseAnim::Block | UseAnim::None => root * arm,
            },
            None => root * arm,
        };
        if using.is_none() && swing.is_some_and(|(_, kind, _)| kind == 1) {
            let sqrt = attack.sqrt();
            let xz = (sqrt * PI).sin();
            m = m
                * Mat4::from_translation(Vec3::new(
                    invert * -0.4 * xz,
                    0.2 * (sqrt * PI * 2.0).sin(),
                    -0.2 * (attack * PI).sin(),
                ))
                // `applyItemArmAttackTransform`.
                * Mat4::from_rotation_y(deg(invert * (45.0 + (attack * attack * PI).sin() * -20.0)))
                * Mat4::from_rotation_z(deg(invert * xz * -20.0))
                * Mat4::from_rotation_x(deg(xz * -80.0))
                * Mat4::from_rotation_y(deg(invert * -45.0));
        }
        let m = m * item_transform(icon.first_person_hand(left), left);
        for (corners, uv, tint) in item_quads(&models.atlas, icon) {
            emit_quad(
                corners.map(|c| m.transform_point3(c)),
                uv,
                camera.eye,
                tint,
                light,
                &mut out,
            );
        }
    }
    out
}

/// `FirstPersonHandsAndItemsRenderer.applyEatTransform`: the food bobs at the mouth, and
/// is lifted in at the start, a jiggle that falls off sharply at the end.
fn eat_transform(u: Using, invert: f32) -> Mat4 {
    let deg = |d: f32| d.to_radians();
    let time = u.remaining;
    let scaled = time / u.duration as f32;
    let mut m = Mat4::IDENTITY;
    if scaled < 0.8 {
        let bob = ((time / 4.0 * PI).cos() * 0.1).abs();
        m *= Mat4::from_translation(Vec3::new(0.0, bob, 0.0));
    }
    let jiggle = 1.0 - scaled.powf(27.0);
    m * Mat4::from_translation(Vec3::new(jiggle * 0.6 * invert, jiggle * -0.5, 0.0))
        * Mat4::from_rotation_y(deg(invert * jiggle * 90.0))
        * Mat4::from_rotation_x(deg(jiggle * 10.0))
        * Mat4::from_rotation_z(deg(invert * jiggle * 30.0))
}

/// `submitArmWithItem`'s `BOW` case: the bow turned across the view, pulled back and
/// stretched as it draws (`BowItem.getPowerForTime`), shaking once it's past a tenth.
fn bow_transform(u: Using, invert: f32) -> Mat4 {
    let deg = |d: f32| d.to_radians();
    let held = u.duration as f32 - u.remaining;
    let power = held / 20.0;
    let power = ((power * power + power * 2.0) / 3.0).min(1.0);
    let mut m = Mat4::from_translation(Vec3::new(invert * -0.2785682, 0.18344387, 0.15731531))
        * Mat4::from_rotation_x(deg(-13.935))
        * Mat4::from_rotation_y(deg(invert * 35.3))
        * Mat4::from_rotation_z(deg(invert * -9.785));
    if power > 0.1 {
        let shake = ((held - 0.1) * 1.3).sin() * (power - 0.1);
        m *= Mat4::from_translation(Vec3::new(0.0, shake * 0.004, 0.0));
    }
    m * Mat4::from_translation(Vec3::new(0.0, 0.0, power * 0.04))
        * Mat4::from_scale(Vec3::new(1.0, 1.0, 1.0 + power * 0.2))
        * Mat4::from_rotation_y(deg(-invert * 45.0))
}

/// `ScreenEffectRenderer.submitFire`: what a burning player sees, two cards of `fire_1`
/// half a block ahead, mirrored, low in the view and turned 10° in from each side, drawn
/// translucent and unlit over everything, the hand too. They're placed where `camera`
/// sees them as the hand's 70° view would, for [`crate::gpu::Renderer::set_fire_overlay`].
pub fn fire_overlay(atlas: &Atlas, camera: &crate::gpu::Camera) -> Vec<Vertex> {
    let [u0, v0, u1, v1] = atlas.sprite("minecraft:block/fire_1").rect;
    // The hand pass's view is `camera.view()` (the screen effects don't bob), so undo it.
    let to_world = camera.view().inverse();
    let mut out = Vec::new();
    for side in [1.0, -1.0] {
        let m = to_world
            * Mat4::from_translation(Vec3::new(side * 0.24, -0.3, 0.0))
            * Mat4::from_rotation_y(side * -PI / 18.0);
        // `buildSpriteQuad`: the sprite's right edge at the left.
        let corners = [
            ([-0.5, -0.5], [u1, v1]),
            ([0.5, -0.5], [u0, v1]),
            ([0.5, 0.5], [u0, v0]),
            ([-0.5, 0.5], [u1, v0]),
        ];
        for ([x, y], uv) in corners {
            out.push(Vertex {
                pos: m.transform_point3(Vec3::new(x, y, -0.5)).to_array(),
                uv,
                // `-436207617`: white at 0xE5 alpha.
                color: [255, 255, 255, 0xE5],
                light: [0, 0, 0, crate::mesh::UNLIT],
            });
        }
    }
    out
}

/// How far a player's outer layer (jacket, sleeves, trousers) sits out, in pixels.
const PLAYER_LAYER: f32 = 0.25;
/// `PlayerModel.createMesh`'s limbs: wide or slim arms, each with its sleeve.
const PLAYER_WIDE: Limbs = Limbs {
    cubes: [
        &[
            cube([-3.0, -2.0, -2.0], [4.0, 12.0, 4.0], [40.0, 16.0]),
            grown(
                cube([-3.0, -2.0, -2.0], [4.0, 12.0, 4.0], [40.0, 32.0]),
                PLAYER_LAYER,
            ),
        ],
        &[
            cube([-1.0, -2.0, -2.0], [4.0, 12.0, 4.0], [32.0, 48.0]),
            grown(
                cube([-1.0, -2.0, -2.0], [4.0, 12.0, 4.0], [48.0, 48.0]),
                PLAYER_LAYER,
            ),
        ],
        PLAYER_LEGS[0],
        PLAYER_LEGS[1],
    ],
    leg_x: 1.9,
};
const PLAYER_SLIM: Limbs = Limbs {
    cubes: [
        &[
            cube([-2.0, -2.0, -2.0], [3.0, 12.0, 4.0], [40.0, 16.0]),
            grown(
                cube([-2.0, -2.0, -2.0], [3.0, 12.0, 4.0], [40.0, 32.0]),
                PLAYER_LAYER,
            ),
        ],
        &[
            cube([-1.0, -2.0, -2.0], [3.0, 12.0, 4.0], [32.0, 48.0]),
            grown(
                cube([-1.0, -2.0, -2.0], [3.0, 12.0, 4.0], [48.0, 48.0]),
                PLAYER_LAYER,
            ),
        ],
        PLAYER_LEGS[0],
        PLAYER_LEGS[1],
    ],
    leg_x: 1.9,
};
const PLAYER_LEGS: [&[Cube]; 2] = [
    &[
        cube([-2.0, 0.0, -2.0], [4.0, 12.0, 4.0], [0.0, 16.0]),
        grown(
            cube([-2.0, 0.0, -2.0], [4.0, 12.0, 4.0], [0.0, 32.0]),
            PLAYER_LAYER,
        ),
    ],
    &[
        cube([-2.0, 0.0, -2.0], [4.0, 12.0, 4.0], [16.0, 48.0]),
        grown(
            cube([-2.0, 0.0, -2.0], [4.0, 12.0, 4.0], [0.0, 48.0]),
            PLAYER_LAYER,
        ),
    ],
];

/// `PlayerModel.createMesh`: a humanoid with its own left limbs' texture, wide or slim
/// arms, and the outer layer (jacket, sleeves, trousers) a quarter pixel out.
fn player(l: &Living, slim: bool) -> Vec<Node> {
    const JACKET: &[Cube] = &[grown(
        cube([-4.0, 0.0, -2.0], [8.0, 12.0, 4.0], [16.0, 32.0]),
        PLAYER_LAYER,
    )];
    let mut nodes = humanoid(l, if slim { PLAYER_SLIM } else { PLAYER_WIDE }, None);
    nodes.push(part(Some(2), [0.0; 3], [0.0; 3], JACKET));
    nodes
}

/// `CreeperModel`: a head and body on four short legs. Its fields swap the legs' names,
/// so the part called `right_hind_leg` swings as `leftHindLeg`, and so on.
fn creeper(l: &Living) -> Vec<Node> {
    const LEG: &[Cube] = &[cube([-2.0, 0.0, -2.0], [4.0, 6.0, 4.0], [0.0, 16.0])];
    let (p, s) = (l.walk_pos * 0.6662, l.walk_speed);
    let a = p.cos() * 1.4 * s;
    let b = (p + PI).cos() * 1.4 * s;
    vec![
        part(
            None,
            [0.0, 6.0, 0.0],
            [l.pitch.to_radians(), l.head_yaw.to_radians(), 0.0],
            HEAD,
        ),
        part(None, [0.0, 6.0, 0.0], [0.0; 3], BODY),
        part(None, [-2.0, 18.0, 4.0], [b, 0.0, 0.0], LEG),
        part(None, [2.0, 18.0, 4.0], [a, 0.0, 0.0], LEG),
        part(None, [-2.0, 18.0, -4.0], [a, 0.0, 0.0], LEG),
        part(None, [2.0, 18.0, -4.0], [b, 0.0, 0.0], LEG),
    ]
}

/// `SpiderModel`: head, neck and abdomen, and eight legs splayed out, each pair swinging
/// and lifting a quarter cycle apart.
fn spider(l: &Living) -> Vec<Node> {
    const RIGHT: &[Cube] = &[cube([-15.0, -1.0, -1.0], [16.0, 2.0, 2.0], [18.0, 0.0])];
    const LEFT: &[Cube] = &[mirrored(cube(
        [-1.0, -1.0, -1.0],
        [16.0, 2.0, 2.0],
        [18.0, 0.0],
    ))];
    let p = l.walk_pos * 0.6662;
    let s = l.walk_speed;
    // Hind, middle hind, middle front, front.
    let phase = [0.0, PI, PI / 2.0, PI * 3.0 / 2.0];
    let swing = phase.map(|o| -((p * 2.0 + o).cos() * 0.4) * s);
    let step = phase.map(|o| ((p + o).sin() * 0.4).abs() * s);
    let (q, e) = (PI / 4.0, PI / 8.0);
    let z = 0.58119464;
    // Each pair: z offset, its rest yaw and roll for the right leg (the left mirrors them).
    let legs = [(2.0, q, -q), (1.0, e, -z), (0.0, -e, -z), (-1.0, -q, -q)];
    const SPIDER_HEAD: &[Cube] = &[cube([-4.0, -4.0, -8.0], [8.0; 3], [32.0, 4.0])];
    const NECK: &[Cube] = &[cube([-3.0; 3], [6.0; 3], [0.0, 0.0])];
    const ABDOMEN: &[Cube] = &[cube([-5.0, -4.0, -6.0], [10.0, 8.0, 12.0], [0.0, 12.0])];
    let look = [l.pitch.to_radians(), l.head_yaw.to_radians(), 0.0];
    let mut nodes = vec![
        part(None, [0.0, 15.0, -3.0], look, SPIDER_HEAD),
        part(None, [0.0, 15.0, 0.0], [0.0; 3], NECK),
        part(None, [0.0, 15.0, 9.0], [0.0; 3], ABDOMEN),
    ];
    for (i, (z_offset, yaw, roll)) in legs.into_iter().enumerate() {
        nodes.push(part(
            None,
            [-4.0, 15.0, z_offset],
            [0.0, yaw + swing[i], roll + step[i]],
            RIGHT,
        ));
        nodes.push(part(
            None,
            [4.0, 15.0, z_offset],
            [0.0, -yaw - swing[i], -roll - step[i]],
            LEFT,
        ));
    }
    nodes
}

/// A `QuadrupedModel`: head, body (lying along z), and legs at their offsets: right hind,
/// left hind, right front, left front.
struct Quadruped {
    head: (&'static [Cube], [f32; 3]),
    body: (&'static [Cube], [f32; 3]),
    legs: [(&'static [Cube], [f32; 3]); 4],
}

/// `CowModel.createBaseCowModel`.
const COW: Quadruped = {
    const RIGHT: &[Cube] = &[cube([-2.0, 0.0, -2.0], [4.0, 12.0, 4.0], [0.0, 16.0])];
    const LEFT: &[Cube] = &[mirrored(cube(
        [-2.0, 0.0, -2.0],
        [4.0, 12.0, 4.0],
        [0.0, 16.0],
    ))];
    Quadruped {
        head: (
            &[
                cube([-4.0, -4.0, -6.0], [8.0, 8.0, 6.0], [0.0, 0.0]),
                cube([-3.0, 1.0, -7.0], [6.0, 3.0, 1.0], [1.0, 33.0]),
                cube([-5.0, -5.0, -5.0], [1.0, 3.0, 1.0], [22.0, 0.0]),
                cube([4.0, -5.0, -5.0], [1.0, 3.0, 1.0], [22.0, 0.0]),
            ],
            [0.0, 4.0, -8.0],
        ),
        body: (
            &[
                cube([-6.0, -10.0, -7.0], [12.0, 18.0, 10.0], [18.0, 4.0]),
                cube([-2.0, 2.0, -8.0], [4.0, 6.0, 1.0], [52.0, 0.0]),
            ],
            [0.0, 5.0, 2.0],
        ),
        legs: [
            (RIGHT, [-4.0, 12.0, 7.0]),
            (LEFT, [4.0, 12.0, 7.0]),
            (RIGHT, [-4.0, 12.0, -5.0]),
            (LEFT, [4.0, 12.0, -5.0]),
        ],
    }
};

/// `PigModel.createBasePigModel`: `QuadrupedModel.createBodyMesh(6)` with a snout, and its
/// left legs mirrored.
const PIG: Quadruped = {
    const RIGHT: &[Cube] = &[cube([-2.0, 0.0, -2.0], [4.0, 6.0, 4.0], [0.0, 16.0])];
    const LEFT: &[Cube] = &[mirrored(cube(
        [-2.0, 0.0, -2.0],
        [4.0, 6.0, 4.0],
        [0.0, 16.0],
    ))];
    Quadruped {
        head: (
            &[
                cube([-4.0, -4.0, -8.0], [8.0; 3], [0.0, 0.0]),
                cube([-2.0, 0.0, -9.0], [4.0, 3.0, 1.0], [16.0, 16.0]),
            ],
            [0.0, 12.0, -6.0],
        ),
        body: (
            &[cube([-5.0, -10.0, -7.0], [10.0, 16.0, 8.0], [28.0, 8.0])],
            [0.0, 11.0, 2.0],
        ),
        legs: [
            (RIGHT, [-3.0, 18.0, 7.0]),
            (LEFT, [3.0, 18.0, 7.0]),
            (RIGHT, [-3.0, 18.0, -5.0]),
            (LEFT, [3.0, 18.0, -5.0]),
        ],
    }
};

/// `SheepModel.createBodyLayer`: `createBodyMesh(12)` with its right legs mirrored.
const SHEEP_BODY: Quadruped = {
    const RIGHT: &[Cube] = &[mirrored(cube(
        [-2.0, 0.0, -2.0],
        [4.0, 12.0, 4.0],
        [0.0, 16.0],
    ))];
    const LEFT: &[Cube] = &[cube([-2.0, 0.0, -2.0], [4.0, 12.0, 4.0], [0.0, 16.0])];
    Quadruped {
        head: (
            &[cube([-3.0, -4.0, -6.0], [6.0, 6.0, 8.0], [0.0, 0.0])],
            [0.0, 6.0, -8.0],
        ),
        body: (
            &[cube([-4.0, -10.0, -7.0], [8.0, 16.0, 6.0], [28.0, 8.0])],
            [0.0, 5.0, 2.0],
        ),
        legs: [
            (RIGHT, [-3.0, 12.0, 7.0]),
            (LEFT, [3.0, 12.0, 7.0]),
            (RIGHT, [-3.0, 12.0, -5.0]),
            (LEFT, [3.0, 12.0, -5.0]),
        ],
    }
};

/// `SheepFurModel.createFurLayer`: the wool, grown out from the body.
const SHEEP_FUR: Quadruped = {
    const LEG: &[Cube] = &[grown(
        cube([-2.0, 0.0, -2.0], [4.0, 6.0, 4.0], [0.0, 16.0]),
        0.5,
    )];
    Quadruped {
        head: (
            &[grown(cube([-3.0, -4.0, -4.0], [6.0; 3], [0.0, 0.0]), 0.6)],
            [0.0, 6.0, -8.0],
        ),
        body: (
            &[grown(
                cube([-4.0, -10.0, -7.0], [8.0, 16.0, 6.0], [28.0, 8.0]),
                1.75,
            )],
            [0.0, 5.0, 2.0],
        ),
        legs: [
            (LEG, [-3.0, 12.0, 7.0]),
            (LEG, [3.0, 12.0, 7.0]),
            (LEG, [-3.0, 12.0, -5.0]),
            (LEG, [3.0, 12.0, -5.0]),
        ],
    }
};

/// `QuadrupedModel.setupAnim`: the head looks, and diagonal legs swing together.
fn quadruped(l: &Living, q: &Quadruped) -> Vec<Node> {
    let (p, s) = (l.walk_pos * 0.6662, l.walk_speed);
    let a = p.cos() * 1.4 * s;
    let b = (p + PI).cos() * 1.4 * s;
    let mut nodes = vec![
        part(
            None,
            q.head.1,
            [l.pitch.to_radians(), l.head_yaw.to_radians(), 0.0],
            q.head.0,
        ),
        part(None, q.body.1, [PI / 2.0, 0.0, 0.0], q.body.0),
    ];
    for ((cubes, at), swing) in q.legs.into_iter().zip([a, b, b, a]) {
        nodes.push(part(None, at, [swing, 0.0, 0.0], cubes));
    }
    nodes
}

/// `AdultChickenModel`: head with beak and wattle, body, legs that walk and wings that
/// flap (`ChickenModel.setupAnim`).
fn chicken(l: &Living) -> Vec<Node> {
    const CHICKEN_HEAD: &[Cube] = &[cube([-2.0, -6.0, -2.0], [4.0, 6.0, 3.0], [0.0, 0.0])];
    const BEAK: &[Cube] = &[cube([-2.0, -4.0, -4.0], [4.0, 2.0, 2.0], [14.0, 0.0])];
    const WATTLE: &[Cube] = &[cube([-1.0, -2.0, -3.0], [2.0; 3], [14.0, 4.0])];
    const CHICKEN_BODY: &[Cube] = &[cube([-3.0, -4.0, -3.0], [6.0, 8.0, 6.0], [0.0, 9.0])];
    const LEG: &[Cube] = &[cube([-1.0, 0.0, -3.0], [3.0, 5.0, 3.0], [26.0, 0.0])];
    const RIGHT_WING: &[Cube] = &[cube([0.0, 0.0, -3.0], [1.0, 4.0, 6.0], [24.0, 13.0])];
    const LEFT_WING: &[Cube] = &[cube([-1.0, 0.0, -3.0], [1.0, 4.0, 6.0], [24.0, 13.0])];
    let (p, s) = (l.walk_pos * 0.6662, l.walk_speed);
    let flap = (l.flap.sin() + 1.0) * l.flap_speed;
    let look = [l.pitch.to_radians(), l.head_yaw.to_radians(), 0.0];
    vec![
        part(None, [0.0, 15.0, -4.0], look, CHICKEN_HEAD),
        part(Some(0), [0.0; 3], [0.0; 3], BEAK),
        part(Some(0), [0.0; 3], [0.0; 3], WATTLE),
        part(None, [0.0, 16.0, 0.0], [PI / 2.0, 0.0, 0.0], CHICKEN_BODY),
        part(None, [-2.0, 19.0, 1.0], [p.cos() * 1.4 * s, 0.0, 0.0], LEG),
        part(
            None,
            [1.0, 19.0, 1.0],
            [(p + PI).cos() * 1.4 * s, 0.0, 0.0],
            LEG,
        ),
        part(None, [-4.0, 13.0, 0.0], [0.0, 0.0, flap], RIGHT_WING),
        part(None, [4.0, 13.0, 0.0], [0.0, 0.0, -flap], LEFT_WING),
    ]
}

/// `EndermanModel`: `HumanoidModel.createMesh` lifted 14 pixels, with long thin limbs; the
/// arms and legs swing half as far, no more than 0.4 radians, and hold a carried block out.
/// Screaming (`isCreepy`) drops its jaw: the head rises 5 pixels and its hat stays put.
fn enderman(l: &Living, creepy: bool, carrying: bool) -> Vec<Node> {
    const HEAD: &[Cube] = &[cube([-4.0, -8.0, -4.0], [8.0; 3], [0.0, 0.0])];
    const HAT: &[Cube] = &[grown(cube([-4.0, -8.0, -4.0], [8.0; 3], [0.0, 16.0]), -0.5)];
    const BODY: &[Cube] = &[cube([-4.0, 0.0, -2.0], [8.0, 12.0, 4.0], [32.0, 16.0])];
    const ARM: Cube = cube([-1.0, -2.0, -1.0], [2.0, 30.0, 2.0], [56.0, 0.0]);
    const LEG: Cube = cube([-1.0, 0.0, -1.0], [2.0, 30.0, 2.0], [56.0, 0.0]);
    const LIMBS: Limbs = Limbs {
        cubes: [&[ARM], &[mirrored(ARM)], &[LEG], &[mirrored(LEG)]],
        leg_x: 2.0,
    };
    let mut nodes = humanoid_parts(l, [HEAD, HAT, BODY], LIMBS, None);
    let set_y = |n: &mut Node, y: f32| n.pose[1] = y;
    set_y(&mut nodes[0], if creepy { -18.0 } else { -13.0 });
    set_y(&mut nodes[1], if creepy { 5.0 } else { 0.0 });
    set_y(&mut nodes[2], -14.0);
    for i in [3, 4] {
        set_y(&mut nodes[i], -12.0);
    }
    for i in [5, 6] {
        set_y(&mut nodes[i], -5.0);
    }
    for n in &mut nodes[3..7] {
        n.pose[3] = (n.pose[3] * 0.5).clamp(-0.4, 0.4);
    }
    if carrying {
        nodes[3].pose[3] = -0.5;
        nodes[4].pose[3] = -0.5;
        nodes[3].pose[5] = 0.05;
        nodes[4].pose[5] = -0.05;
    }
    nodes
}

/// `SlimeModel.createInnerBodyLayer`: the core, eyes and mouth inside.
const SLIME_INNER: [Node; 4] = [
    Node {
        parent: None,
        pose: [0.0; 6],
        scale: [1.0; 3],
        cubes: &[cube([-3.0, 17.0, -3.0], [6.0; 3], [0.0, 16.0])],
    },
    Node {
        parent: None,
        pose: [0.0; 6],
        scale: [1.0; 3],
        cubes: &[cube([-3.25, 18.0, -3.5], [2.0; 3], [32.0, 0.0])],
    },
    Node {
        parent: None,
        pose: [0.0; 6],
        scale: [1.0; 3],
        cubes: &[cube([1.25, 18.0, -3.5], [2.0; 3], [32.0, 4.0])],
    },
    Node {
        parent: None,
        pose: [0.0; 6],
        scale: [1.0; 3],
        cubes: &[cube([0.0, 21.0, -3.5], [1.0; 3], [32.0, 8.0])],
    },
];

/// `SlimeModel.createOuterBodyLayer`: the see-through cube around it.
const SLIME_OUTER: &[Node] = &[Node {
    parent: None,
    pose: [0.0; 6],
    scale: [1.0; 3],
    cubes: &[cube([-4.0, 16.0, -4.0], [8.0; 3], [0.0, 0.0])],
}];

/// `VillagerModel.createBodyModel`'s body, jacket, folded arms and legs, the parts a witch
/// shares; nodes 0 to 4 after `first` (the head's).
fn villager_body(l: &Living, first: usize) -> [Node; 5] {
    const BODY: &[Cube] = &[cube([-4.0, 0.0, -3.0], [8.0, 12.0, 6.0], [16.0, 20.0])];
    const JACKET: &[Cube] = &[grown(
        cube([-4.0, 0.0, -3.0], [8.0, 20.0, 6.0], [0.0, 38.0]),
        0.5,
    )];
    const ARMS: &[Cube] = &[
        cube([-8.0, -2.0, -2.0], [4.0, 8.0, 4.0], [44.0, 22.0]),
        mirrored(cube([4.0, -2.0, -2.0], [4.0, 8.0, 4.0], [44.0, 22.0])),
        cube([-4.0, 2.0, -2.0], [8.0, 4.0, 4.0], [40.0, 38.0]),
    ];
    const LEG: Cube = cube([-2.0, 0.0, -2.0], [4.0, 12.0, 4.0], [0.0, 22.0]);
    const RIGHT_LEG: &[Cube] = &[LEG];
    const LEFT_LEG: &[Cube] = &[mirrored(LEG)];
    let (p, s) = (l.walk_pos * 0.6662, l.walk_speed);
    [
        part(None, [0.0; 3], [0.0; 3], BODY),
        part(Some(first), [0.0; 3], [0.0; 3], JACKET),
        part(None, [0.0, 3.0, -1.0], [-0.75, 0.0, 0.0], ARMS),
        part(
            None,
            [-2.0, 12.0, 0.0],
            [p.cos() * 1.4 * s * 0.5, 0.0, 0.0],
            RIGHT_LEG,
        ),
        part(
            None,
            [2.0, 12.0, 0.0],
            [(p + PI).cos() * 1.4 * s * 0.5, 0.0, 0.0],
            LEFT_LEG,
        ),
    ]
}

/// `VillagerModel`: its head with a hat, its brim and nose, which shakes while it's
/// unhappy; without the head at all for `createNoHatModel`, which the clothes of a type
/// whose hat a profession's covers are drawn on.
fn villager(l: &Living, unhappy: bool, head: bool) -> Vec<Node> {
    const HEAD: &[Cube] = &[cube([-4.0, -10.0, -4.0], [8.0, 10.0, 8.0], [0.0, 0.0])];
    const HAT: &[Cube] = &[grown(
        cube([-4.0, -10.0, -4.0], [8.0, 10.0, 8.0], [32.0, 0.0]),
        0.51,
    )];
    const RIM: &[Cube] = &[cube([-8.0, -8.0, -6.0], [16.0, 16.0, 1.0], [30.0, 47.0])];
    const NOSE: &[Cube] = &[cube([-1.0, -1.0, -6.0], [2.0, 4.0, 2.0], [24.0, 0.0])];
    let look = if unhappy {
        [0.4, l.head_yaw.to_radians(), 0.3 * (0.45 * l.age).sin()]
    } else {
        [l.pitch.to_radians(), l.head_yaw.to_radians(), 0.0]
    };
    let mut nodes = vec![part(None, [0.0; 3], look, if head { HEAD } else { &[] })];
    nodes.extend(villager_body(l, 1));
    if head {
        nodes.push(part(Some(0), [0.0; 3], [0.0; 3], HAT));
        let hat = nodes.len() - 1;
        nodes.push(part(Some(hat), [0.0; 3], [-PI / 2.0, 0.0, 0.0], RIM));
        nodes.push(part(Some(0), [0.0, -2.0, 0.0], [0.0; 3], NOSE));
    }
    nodes
}

/// `WitchModel`: a villager whose hat is a tall crooked cone (keeping the villager hat's
/// brim, as `addOrReplaceChild` keeps the children of the part it replaces), with a mole
/// on its twitching nose (`ageInTicks` times a hundredth of the entity id's last digit),
/// which lifts while it holds something.
fn witch(l: &Living, id: i32, holding: bool) -> Vec<Node> {
    const HEAD: &[Cube] = &[cube([-4.0, -10.0, -4.0], [8.0, 10.0, 8.0], [0.0, 0.0])];
    const HAT: &[Cube] = &[cube([0.0; 3], [10.0, 2.0, 10.0], [0.0, 64.0])];
    const HAT2: &[Cube] = &[cube([0.0; 3], [7.0, 4.0, 7.0], [0.0, 76.0])];
    const HAT3: &[Cube] = &[cube([0.0; 3], [4.0; 3], [0.0, 87.0])];
    const HAT4: &[Cube] = &[grown(cube([0.0; 3], [1.0, 2.0, 1.0], [0.0, 95.0]), 0.25)];
    const RIM: &[Cube] = &[cube([-8.0, -8.0, -6.0], [16.0, 16.0, 1.0], [30.0, 47.0])];
    const NOSE: &[Cube] = &[cube([-1.0, -1.0, -6.0], [2.0, 4.0, 2.0], [24.0, 0.0])];
    const MOLE: &[Cube] = &[grown(cube([0.0, 3.0, -6.75], [1.0; 3], [0.0, 0.0]), -0.25)];
    let look = [l.pitch.to_radians(), l.head_yaw.to_radians(), 0.0];
    let mut nodes = vec![part(None, [0.0; 3], look, HEAD)];
    nodes.extend(villager_body(l, 1));
    let speed = 0.01 * id.rem_euclid(10) as f32;
    let (nose_at, nose_x) = if holding {
        ([0.0, 1.0, -1.5], -0.9)
    } else {
        (
            [0.0, -2.0, 0.0],
            (l.age * speed).sin() * 4.5f32.to_radians(),
        )
    };
    let nose_z = (l.age * speed).cos() * 2.5f32.to_radians();
    let at = nodes.len();
    nodes.extend([
        part(Some(0), [-5.0, -10.03125, -5.0], [0.0; 3], HAT),
        part(
            Some(at),
            [1.75, -4.0, 2.0],
            [-0.05235988, 0.0, 0.02617994],
            HAT2,
        ),
        part(
            Some(at + 1),
            [1.75, -4.0, 2.0],
            [-0.10471976, 0.0, 0.05235988],
            HAT3,
        ),
        part(
            Some(at + 2),
            [1.75, -2.0, 2.0],
            [-PI / 15.0, 0.0, 0.10471976],
            HAT4,
        ),
        part(Some(at), [0.0; 3], [-PI / 2.0, 0.0, 0.0], RIM),
        part(Some(0), nose_at, [nose_x, 0.0, nose_z], NOSE),
        part(Some(at + 5), [0.0, -2.0, 0.0], [0.0; 3], MOLE),
    ]);
    nodes
}

/// `ZombieVillagerModel.createBodyLayer`: `HumanoidModel` with a villager's long head and
/// nose, hat and brim, and robe; posed as a zombie. Without the head for
/// `createNoHatLayer`, which the clothes of a type whose hat a profession's covers are
/// drawn on.
fn zombie_villager(l: &Living, aggressive: bool, head: bool) -> Vec<Node> {
    const HEAD: &[Cube] = &[
        cube([-4.0, -10.0, -4.0], [8.0, 10.0, 8.0], [0.0, 0.0]),
        cube([-1.0, -3.0, -6.0], [2.0, 4.0, 2.0], [24.0, 0.0]),
    ];
    const HAT: &[Cube] = &[grown(
        cube([-4.0, -10.0, -4.0], [8.0, 10.0, 8.0], [32.0, 0.0]),
        0.5,
    )];
    const RIM: &[Cube] = &[cube([-8.0, -8.0, -6.0], [16.0, 16.0, 1.0], [30.0, 47.0])];
    const BODY: &[Cube] = &[
        cube([-4.0, 0.0, -3.0], [8.0, 12.0, 6.0], [16.0, 20.0]),
        grown(cube([-4.0, 0.0, -3.0], [8.0, 20.0, 6.0], [0.0, 38.0]), 0.05),
    ];
    const LEG: Cube = cube([-2.0, 0.0, -2.0], [4.0, 12.0, 4.0], [0.0, 22.0]);
    const LIMBS: Limbs = Limbs {
        cubes: [
            &[cube([-3.0, -2.0, -2.0], [4.0, 12.0, 4.0], [44.0, 22.0])],
            &[mirrored(cube(
                [-1.0, -2.0, -2.0],
                [4.0, 12.0, 4.0],
                [44.0, 22.0],
            ))],
            &[LEG],
            &[mirrored(LEG)],
        ],
        leg_x: 2.0,
    };
    if baby_model(l) {
        return baby_zombie_villager(l, aggressive, head);
    }
    let parts = if head {
        [HEAD, HAT, BODY]
    } else {
        [&[], &[], BODY]
    };
    let mut nodes = humanoid_parts(l, parts, LIMBS, Some(aggressive));
    if head {
        nodes.push(part(Some(1), [0.0; 3], [-PI / 2.0, 0.0, 0.0], RIM));
    }
    nodes
}

/// `BabyZombieVillagerModel`: a short humanoid with a big villager head and nose, its hat
/// and brim, and a robe; posed as a zombie. The hat, brim and nose hang unturned from the
/// head, so their offsets are folded into their cubes. Without the head for
/// `createNoHatLayer`.
fn baby_zombie_villager(l: &Living, aggressive: bool, head: bool) -> Vec<Node> {
    const HEAD: &[Cube] = &[
        cube([-4.0, -8.0, -3.5], [8.0, 8.0, 7.0], [0.0, 0.0]),
        cube([-1.0, -2.0, -4.5], [2.0, 2.0, 1.0], [23.0, 0.0]),
    ];
    const HAT: &[Cube] = &[
        grown(cube([-4.0, -8.0, -3.5], [8.0, 8.0, 7.0], [0.0, 31.0]), 0.3),
        cube([-7.0, -5.0, -6.0], [14.0, 1.0, 12.0], [0.0, 46.0]),
    ];
    const BODY: &[Cube] = &[
        cube([-2.0, -2.75, -1.5], [4.0, 5.0, 3.0], [0.0, 15.0]),
        grown(
            cube([-2.0, -2.75, -1.5], [4.0, 6.0, 3.0], [16.0, 22.0]),
            0.1,
        ),
    ];
    const LIMBS: Limbs = Limbs {
        cubes: [
            &[cube([-1.0, -0.5, -1.0], [2.0, 5.0, 2.0], [24.0, 15.0])],
            &[cube([-1.0, -0.5, -1.0], [2.0, 5.0, 2.0], [16.0, 15.0])],
            &[cube([-1.0, -0.5, -1.0], [2.0, 3.0, 2.0], [8.0, 23.0])],
            &[cube([-1.0, -0.5, -1.0], [2.0, 3.0, 2.0], [0.0, 23.0])],
        ],
        leg_x: 1.0,
    };
    let parts = if head {
        [HEAD, HAT, BODY]
    } else {
        [&[], &[], BODY]
    };
    humanoid_parts(l, parts, LIMBS, Some(aggressive))
}

/// `Mth.triangleWave`: -1 to 1 and back over each `period`.
fn triangle_wave(index: f32, period: f32) -> f32 {
    ((index % period - period * 0.5).abs() - period * 0.25) / (period * 0.25)
}

/// `IronGolemModel` (128 × 128): its arms swing against its legs as it walks, and both
/// come down together in an attack.
fn iron_golem(l: &Living, attack: f32) -> Vec<Node> {
    const HEAD: &[Cube] = &[
        cube([-4.0, -12.0, -5.5], [8.0, 10.0, 8.0], [0.0, 0.0]),
        cube([-1.0, -5.0, -7.5], [2.0, 4.0, 2.0], [24.0, 0.0]),
    ];
    const BODY: &[Cube] = &[
        cube([-9.0, -2.0, -6.0], [18.0, 12.0, 11.0], [0.0, 40.0]),
        grown(cube([-4.5, 10.0, -3.0], [9.0, 5.0, 6.0], [0.0, 70.0]), 0.5),
    ];
    const RIGHT_ARM: &[Cube] = &[cube([-13.0, -2.5, -3.0], [4.0, 30.0, 6.0], [60.0, 21.0])];
    const LEFT_ARM: &[Cube] = &[cube([9.0, -2.5, -3.0], [4.0, 30.0, 6.0], [60.0, 58.0])];
    const RIGHT_LEG: &[Cube] = &[cube([-3.5, -3.0, -3.0], [6.0, 16.0, 5.0], [37.0, 0.0])];
    const LEFT_LEG: &[Cube] = &[mirrored(cube(
        [-3.5, -3.0, -3.0],
        [6.0, 16.0, 5.0],
        [60.0, 0.0],
    ))];
    let (p, s) = (l.walk_pos, l.walk_speed);
    let wave = triangle_wave(p, 13.0);
    let arms = if attack > 0.0 {
        let a = -2.0 + 1.5 * triangle_wave(attack, 10.0);
        [a, a]
    } else {
        [(-0.2 + 1.5 * wave) * s, (-0.2 - 1.5 * wave) * s]
    };
    let look = [l.pitch.to_radians(), l.head_yaw.to_radians(), 0.0];
    vec![
        part(None, [0.0, -7.0, -2.0], look, HEAD),
        part(None, [0.0, -7.0, 0.0], [0.0; 3], BODY),
        part(None, [0.0, -7.0, 0.0], [arms[0], 0.0, 0.0], RIGHT_ARM),
        part(None, [0.0, -7.0, 0.0], [arms[1], 0.0, 0.0], LEFT_ARM),
        part(
            None,
            [-4.0, 11.0, 0.0],
            [-1.5 * wave * s, 0.0, 0.0],
            RIGHT_LEG,
        ),
        part(None, [5.0, 11.0, 0.0], [1.5 * wave * s, 0.0, 0.0], LEFT_LEG),
    ]
}

/// `SquidModel`: a body over eight tentacles in a ring, all lifted to the same angle.
fn squid(tentacles: f32) -> Vec<Node> {
    const BODY: &[Cube] = &[grown(
        cube([-6.0, -8.0, -6.0], [12.0, 16.0, 12.0], [0.0, 0.0]),
        0.02,
    )];
    const TENTACLE: &[Cube] = &[cube([-1.0, 0.0, -1.0], [2.0, 18.0, 2.0], [48.0, 0.0])];
    let mut nodes = vec![part(None, [0.0, 8.0, 0.0], [0.0; 3], BODY)];
    for i in 0..8 {
        let angle = i as f64 * std::f64::consts::PI * 2.0 / 8.0;
        let at = [angle.cos() as f32 * 5.0, 15.0, angle.sin() as f32 * 5.0];
        let yaw =
            (i as f64 * std::f64::consts::PI * -2.0 / 8.0 + std::f64::consts::FRAC_PI_2) as f32;
        nodes.push(part(None, at, [tentacles, yaw, 0.0], TENTACLE));
    }
    nodes
}

/// `BabySquidModel` (32 × 32): a small body over eight short tentacles in a ring, all
/// lifted to the same angle.
fn baby_squid(tentacles: f32) -> Vec<Node> {
    const BODY: &[Cube] = &[cube([-4.0, -5.0, -4.0], [8.0, 10.0, 8.0], [0.0, 0.0])];
    const TENTACLE: &[Cube] = &[cube([-1.0, -0.5, -1.0], [2.0, 6.0, 2.0], [0.0, 18.0])];
    let mut nodes = vec![part(None, [0.0, 13.0, 0.0], [0.0; 3], BODY)];
    for i in 0..8 {
        let angle = i as f64 * std::f64::consts::PI * 2.0 / 8.0;
        let at = [angle.cos() as f32 * 3.0, 18.5, angle.sin() as f32 * 3.0];
        let yaw =
            (i as f64 * std::f64::consts::PI * -2.0 / 8.0 + std::f64::consts::FRAC_PI_2) as f32;
        nodes.push(part(None, at, [tentacles, yaw, 0.0], TENTACLE));
    }
    nodes
}

/// `PhantomModel`: a body with a head and a two-part tail, and two-part wings that flap
/// with `flapTime`.
fn phantom(flap: f32) -> Vec<Node> {
    const BODY: &[Cube] = &[cube([-3.0, -2.0, -8.0], [5.0, 3.0, 9.0], [0.0, 8.0])];
    const TAIL_BASE: &[Cube] = &[cube([-2.0, 0.0, 0.0], [3.0, 2.0, 6.0], [3.0, 20.0])];
    const TAIL_TIP: &[Cube] = &[cube([-1.0, 0.0, 0.0], [1.0, 1.0, 6.0], [4.0, 29.0])];
    const LEFT_BASE: &[Cube] = &[cube([0.0; 3], [6.0, 2.0, 9.0], [23.0, 12.0])];
    const LEFT_TIP: &[Cube] = &[cube([0.0; 3], [13.0, 1.0, 9.0], [16.0, 24.0])];
    const RIGHT_BASE: &[Cube] = &[mirrored(cube(
        [-6.0, 0.0, 0.0],
        [6.0, 2.0, 9.0],
        [23.0, 12.0],
    ))];
    const RIGHT_TIP: &[Cube] = &[mirrored(cube(
        [-13.0, 0.0, 0.0],
        [13.0, 1.0, 9.0],
        [16.0, 24.0],
    ))];
    const HEAD: &[Cube] = &[cube([-4.0, -2.0, -5.0], [7.0, 3.0, 5.0], [0.0, 0.0])];
    let anim = (flap * 7.448451).to_radians();
    let wing = (anim.cos() * 16.0).to_radians();
    let tail = -(5.0 + (anim * 2.0).cos() * 5.0f32).to_radians();
    vec![
        part(None, [0.0; 3], [-0.1, 0.0, 0.0], BODY),
        part(Some(0), [0.0, -2.0, 1.0], [tail, 0.0, 0.0], TAIL_BASE),
        part(Some(1), [0.0, 0.5, 6.0], [tail, 0.0, 0.0], TAIL_TIP),
        part(Some(0), [2.0, -2.0, -8.0], [0.0, 0.0, wing], LEFT_BASE),
        part(Some(3), [6.0, 0.0, 0.0], [0.0, 0.0, wing], LEFT_TIP),
        part(Some(0), [-3.0, -2.0, -8.0], [0.0, 0.0, -wing], RIGHT_BASE),
        part(Some(5), [-6.0, 0.0, 0.0], [0.0, 0.0, -wing], RIGHT_TIP),
        part(Some(0), [0.0, 1.0, -7.0], [0.2, 0.0, 0.0], HEAD),
    ]
}

/// `BatModel`: a body with two-part wings and feet, and a head with ears, posed only by
/// its flying or resting animation; resting, its head turns with its look.
fn bat(l: &Living, resting: bool, fly: Option<f32>, rest: Option<f32>) -> Vec<Node> {
    const BODY: &[Cube] = &[cube([-1.5, 0.0, -1.0], [3.0, 5.0, 2.0], [0.0, 0.0])];
    const HEAD: &[Cube] = &[cube([-2.0, -3.0, -1.0], [4.0, 3.0, 2.0], [0.0, 7.0])];
    const RIGHT_EAR: &[Cube] = &[cube([-2.5, -4.0, 0.0], [3.0, 5.0, 0.0], [1.0, 15.0])];
    const LEFT_EAR: &[Cube] = &[cube([-0.1, -3.0, 0.0], [3.0, 5.0, 0.0], [8.0, 15.0])];
    const RIGHT_WING: &[Cube] = &[cube([-2.0, -2.0, 0.0], [2.0, 7.0, 0.0], [12.0, 0.0])];
    const RIGHT_TIP: &[Cube] = &[cube([-6.0, -2.0, 0.0], [6.0, 8.0, 0.0], [16.0, 0.0])];
    const LEFT_WING: &[Cube] = &[cube([0.0, -2.0, 0.0], [2.0, 7.0, 0.0], [12.0, 7.0])];
    const LEFT_TIP: &[Cube] = &[cube([0.0, -2.0, 0.0], [6.0, 8.0, 0.0], [16.0, 8.0])];
    const FEET: &[Cube] = &[cube([-1.5, 0.0, 0.0], [3.0, 2.0, 0.0], [16.0, 16.0])];
    const BONES: &[&str] = &[
        "body",
        "head",
        "right_ear",
        "left_ear",
        "right_wing",
        "right_wing_tip",
        "left_wing",
        "left_wing_tip",
        "feet",
    ];
    let head_yaw = if resting {
        l.head_yaw.to_radians()
    } else {
        0.0
    };
    let mut nodes = vec![
        part(None, [0.0, 17.0, 0.0], [0.0; 3], BODY),
        part(None, [0.0, 17.0, 0.0], [0.0, head_yaw, 0.0], HEAD),
        part(Some(1), [-1.5, -2.0, 0.0], [0.0; 3], RIGHT_EAR),
        part(Some(1), [1.1, -3.0, 0.0], [0.0; 3], LEFT_EAR),
        part(Some(0), [-1.5, 0.0, 0.0], [0.0; 3], RIGHT_WING),
        part(Some(4), [-2.0, 0.0, 0.0], [0.0; 3], RIGHT_TIP),
        part(Some(0), [1.5, 0.0, 0.0], [0.0; 3], LEFT_WING),
        part(Some(6), [2.0, 0.0, 0.0], [0.0; 3], LEFT_TIP),
        part(Some(0), [0.0, 5.0, 0.0], [0.0; 3], FEET),
    ];
    if let Some(t) = fly {
        crate::animations::BAT_FLYING.apply(t, BONES, &mut nodes);
    }
    if let Some(t) = rest {
        crate::animations::BAT_RESTING.apply(t, BONES, &mut nodes);
    }
    nodes
}

/// `RabbitModel.setupAnim`: unless it's tilting its head idly, its head follows its look
/// (replacing the head's own pitch); then its hop and its idle head tilt.
fn rabbit_anim(
    l: &Living,
    hop: Option<f32>,
    tilt: Option<f32>,
    animations: [&crate::animation::Animation; 2],
    bones: &[&str],
    nodes: &mut [Node],
) {
    if tilt.is_none() {
        let head = bones.iter().position(|b| *b == "head").expect("a head");
        nodes[head].pose[3] = l.pitch.to_radians();
        nodes[head].pose[4] = l.head_yaw.to_radians();
    }
    if let Some(t) = hop {
        animations[0].apply(t, bones, nodes);
    }
    if let Some(t) = tilt {
        animations[1].apply(t, bones, nodes);
    }
}

/// `AdultRabbitModel` (64 × 64): a sloping body with a tail, a head with long ears, front
/// legs, and haunches that stay on the ground.
// Vanilla's own rounded angles, such as 0.3927 for π/8.
#[allow(clippy::approx_constant)]
fn rabbit(l: &Living, hop: Option<f32>, tilt: Option<f32>) -> Vec<Node> {
    const BODY: &[Cube] = &[cube([-4.0, -6.0, -9.0], [8.0, 6.0, 10.0], [0.0, 0.0])];
    const TAIL: &[Cube] = &[cube(
        [-2.0, -3.0084, -1.0125],
        [4.0, 4.0, 4.0],
        [20.0, 16.0],
    )];
    const HEAD: &[Cube] = &[cube([-2.5, -3.0, -4.0], [5.0, 5.0, 5.0], [0.0, 16.0])];
    const LEFT_EAR: &[Cube] = &[cube([-1.0, -4.2929, -0.1213], [2.0, 5.0, 1.0], [32.0, 0.0])];
    const RIGHT_EAR: &[Cube] = &[cube([-1.0, -4.2929, -0.1213], [2.0, 5.0, 1.0], [26.0, 0.0])];
    const RIGHT_FRONT: &[Cube] = &[cube([-0.9, -1.0, -0.9], [2.0, 4.0, 2.0], [36.0, 18.0])];
    const LEFT_FRONT: &[Cube] = &[cube([-1.0, -1.0, -1.0], [2.0, 4.0, 2.0], [44.0, 18.0])];
    const RIGHT_HAUNCH: &[Cube] = &[cube([-1.0, 0.0, -5.0], [2.0, 1.0, 6.0], [20.0, 24.0])];
    const LEFT_HAUNCH: &[Cube] = &[cube([-1.0, 0.0, -5.0], [2.0, 1.0, 6.0], [36.0, 24.0])];
    const BONES: &[&str] = &[
        "body",
        "tail",
        "head",
        "left_ear",
        "right_ear",
        "frontlegs",
        "right_front_leg",
        "left_front_leg",
        "backlegs",
        "right_hind_leg",
        "right_haunch",
        "left_hind_leg",
        "left_haunch",
    ];
    let leg = 0.3927;
    let mut nodes = vec![
        part(None, [0.0, 23.0, 4.0], [-0.3927, 0.0, 0.0], BODY),
        part(Some(0), [0.0, -4.9916, 0.0125], [0.0; 3], TAIL),
        part(Some(0), [0.0, -5.2929, -8.1213], [0.3927, 0.0, 0.0], HEAD),
        part(Some(2), [1.5, -3.7071, -0.8787], [0.0; 3], LEFT_EAR),
        part(Some(2), [-1.5, -3.7071, -0.8787], [0.0; 3], RIGHT_EAR),
        part(Some(0), [0.0, -1.5349, -6.3108], [0.0; 3], &[]),
        part(
            Some(5),
            [-2.0, 1.9239, 0.3827],
            [leg, 0.0, 0.0],
            RIGHT_FRONT,
        ),
        part(Some(5), [2.0, 1.9239, 0.4827], [leg, 0.0, 0.0], LEFT_FRONT),
        part(None, [0.0, 23.0, 4.0], [0.0; 3], &[]),
        part(Some(8), [-3.0, 0.5, 0.0], [0.0; 3], &[]),
        part(Some(9), [0.0, -0.5, 0.0], [0.0, leg, 0.0], RIGHT_HAUNCH),
        part(Some(8), [3.0, 0.5, 0.0], [0.0; 3], &[]),
        part(Some(11), [0.0, -0.5, 0.0], [0.0, -leg, 0.0], LEFT_HAUNCH),
    ];
    let animations = [
        &crate::animations::RABBIT_HOP,
        &crate::animations::RABBIT_IDLE_HEAD_TILT,
    ];
    rabbit_anim(l, hop, tilt, animations, BONES, &mut nodes);
    nodes
}

/// `BabyRabbitModel` (32 × 32): a round body and tail, a big head with short ears, and
/// little legs.
// Vanilla's own rounded angles, such as 0.3927 for π/8.
#[allow(clippy::approx_constant)]
fn baby_rabbit(l: &Living, hop: Option<f32>, tilt: Option<f32>) -> Vec<Node> {
    const BODY: &[Cube] = &[cube([-2.0, -2.0, -3.0], [4.0, 3.0, 6.0], [0.0, 8.0])];
    const TAIL: &[Cube] = &[cube([-1.4, -2.0268, -1.0177], [3.0, 3.0, 3.0], [0.0, 21.0])];
    const HEAD: &[Cube] = &[cube([-2.5, -3.0, -3.0], [5.0, 4.0, 4.0], [0.0, 0.0])];
    const RIGHT_EAR: &[Cube] = &[cube([-1.0, -3.5, -0.5], [2.0, 4.0, 1.0], [18.0, 0.0])];
    const LEFT_EAR: &[Cube] = &[cube([-1.0, -3.5, -0.5], [2.0, 4.0, 1.0], [24.0, 0.0])];
    const LEFT_FRONT: &[Cube] = &[cube([-0.5, -1.5, -0.5], [1.0, 3.0, 1.0], [18.0, 8.0])];
    const RIGHT_FRONT: &[Cube] = &[cube([-0.5, -1.5, -0.5], [1.0, 3.0, 1.0], [14.0, 8.0])];
    const LEFT_HAUNCH: &[Cube] = &[cube([-2.0, -0.5, 0.0], [2.0, 1.0, 3.0], [10.0, 17.0])];
    const RIGHT_HAUNCH: &[Cube] = &[cube([-2.0, -0.5, 0.0], [2.0, 1.0, 3.0], [0.0, 17.0])];
    const BONES: &[&str] = &[
        "body",
        "body_r1",
        "tail",
        "tail_r1",
        "head",
        "right_ear",
        "left_ear",
        "frontlegs",
        "left_front_leg",
        "left_front_leg_r1",
        "right_front_leg",
        "right_front_leg_r1",
        "backlegs",
        "left_hind_leg",
        "left_haunch",
        "right_hind_leg",
        "right_haunch",
    ];
    let (leg, slope, turn) = (0.3927, -0.5236, 3.1416);
    let mut nodes = vec![
        part(None, [0.0, 23.0, 1.6], [0.0; 3], &[]),
        part(Some(0), [0.0, -2.0, -1.6], [slope, 0.0, 0.0], BODY),
        part(Some(0), [0.0, -2.2, 2.0], [0.0; 3], &[]),
        part(Some(2), [-0.1, 0.0, 0.0], [slope, 0.0, 0.0], TAIL),
        part(Some(0), [0.0, -5.0, -2.6], [0.0; 3], HEAD),
        part(Some(4), [-1.5, -3.5, -0.5], [0.0; 3], RIGHT_EAR),
        part(Some(4), [1.5, -3.5, -0.5], [0.0; 3], LEFT_EAR),
        part(Some(0), [0.0, -2.5, -2.6], [0.0; 3], &[]),
        part(Some(7), [1.0, 1.0, -0.5], [leg, 0.0, 0.0], &[]),
        part(Some(8), [0.0, 1.0, 0.0], [-leg, 0.0, 0.0], LEFT_FRONT),
        part(Some(7), [-1.0, 1.0, -0.5], [leg, 0.0, 0.0], &[]),
        part(Some(10), [0.0, 1.0, 0.0], [-leg, 0.0, 0.0], RIGHT_FRONT),
        part(None, [0.0, 23.0, 2.0], [0.0; 3], &[]),
        part(Some(12), [1.5, 0.5, 0.5], [0.0, turn, 0.0], &[]),
        part(Some(13), [1.0, 0.0, 0.5], [0.0, -0.7854, 0.0], LEFT_HAUNCH),
        part(Some(12), [-1.5, 0.5, 0.5], [0.0, turn, 0.0], &[]),
        part(Some(15), [0.5, 0.0, -0.9], [0.0, 0.7854, 0.0], RIGHT_HAUNCH),
    ];
    let animations = [
        &crate::animations::BABY_RABBIT_HOP,
        &crate::animations::BABY_RABBIT_IDLE_HEAD_TILT,
    ];
    rabbit_anim(l, hop, tilt, animations, BONES, &mut nodes);
    nodes
}

/// `ModelPart.visible = false`: the part and everything hung from it aren't drawn.
fn hide(nodes: &mut [Node], part: usize) {
    for i in part..nodes.len() {
        let mut at = Some(i);
        while let Some(j) = at {
            if j == part {
                nodes[i].cubes = &[];
                break;
            }
            at = nodes[j].parent;
        }
    }
}

/// `FrogModel`: a flat body and head with bulging eyes, a tongue, a throat that swells
/// as it croaks, and webbed hands and feet; posed only by its animations: walking or
/// swimming as it moves, and jumping, croaking, licking and treading water.
fn frog(l: &Living, anims: [Option<f32>; 4], swimming: bool) -> Vec<Node> {
    const BODY: &[Cube] = &[
        cube([-3.5, -2.0, -8.0], [7.0, 3.0, 9.0], [3.0, 1.0]),
        cube([-3.5, -1.0, -8.0], [7.0, 0.0, 9.0], [23.0, 22.0]),
    ];
    const HEAD: &[Cube] = &[
        cube([-3.5, -1.0, -7.0], [7.0, 0.0, 9.0], [23.0, 13.0]),
        cube([-3.5, -2.0, -7.0], [7.0, 3.0, 9.0], [0.0, 13.0]),
    ];
    const RIGHT_EYE: &[Cube] = &[cube([-1.5, -1.0, -1.5], [3.0, 2.0, 3.0], [0.0, 0.0])];
    const LEFT_EYE: &[Cube] = &[cube([-1.5, -1.0, -1.5], [3.0, 2.0, 3.0], [0.0, 5.0])];
    const CROAKING: &[Cube] = &[grown(
        cube([-3.5, -0.1, -2.9], [7.0, 2.0, 3.0], [26.0, 5.0]),
        -0.1,
    )];
    const TONGUE: &[Cube] = &[cube([-2.0, 0.0, -7.1], [4.0, 0.0, 7.0], [17.0, 13.0])];
    const LEFT_ARM: &[Cube] = &[cube([-1.0, 0.0, -1.0], [2.0, 3.0, 3.0], [0.0, 32.0])];
    const LEFT_HAND: &[Cube] = &[cube([-4.0, 0.01, -4.0], [8.0, 0.0, 8.0], [18.0, 40.0])];
    const RIGHT_ARM: &[Cube] = &[cube([-1.0, 0.0, -1.0], [2.0, 3.0, 3.0], [0.0, 38.0])];
    const RIGHT_HAND: &[Cube] = &[cube([-4.0, 0.01, -5.0], [8.0, 0.0, 8.0], [2.0, 40.0])];
    const LEFT_LEG: &[Cube] = &[cube([-1.0, 0.0, -2.0], [3.0, 3.0, 4.0], [14.0, 25.0])];
    const LEFT_FOOT: &[Cube] = &[cube([-4.0, 0.01, -4.0], [8.0, 0.0, 8.0], [2.0, 32.0])];
    const RIGHT_LEG: &[Cube] = &[cube([-2.0, 0.0, -2.0], [3.0, 3.0, 4.0], [0.0, 25.0])];
    const RIGHT_FOOT: &[Cube] = &[cube([-4.0, 0.01, -4.0], [8.0, 0.0, 8.0], [18.0, 32.0])];
    // The model's own root part is also named "root", but an animation's "root" is the
    // layer's root above it (`createPartLookup`), which none of the frog's move.
    const BONES: &[&str] = &[
        "",
        "body",
        "head",
        "eyes",
        "right_eye",
        "left_eye",
        "croaking_body",
        "tongue",
        "left_arm",
        "left_hand",
        "right_arm",
        "right_hand",
        "left_leg",
        "left_foot",
        "right_leg",
        "right_foot",
    ];
    let mut nodes = vec![
        part(None, [0.0, 24.0, 0.0], [0.0; 3], &[]),
        part(Some(0), [0.0, -2.0, 4.0], [0.0; 3], BODY),
        part(Some(1), [0.0, -2.0, -1.0], [0.0; 3], HEAD),
        part(Some(2), [-0.5, 0.0, 2.0], [0.0; 3], &[]),
        part(Some(3), [-1.5, -3.0, -6.5], [0.0; 3], RIGHT_EYE),
        part(Some(3), [2.5, -3.0, -6.5], [0.0; 3], LEFT_EYE),
        part(Some(1), [0.0, -1.0, -5.0], [0.0; 3], CROAKING),
        part(Some(1), [0.0, -1.01, 1.0], [0.0; 3], TONGUE),
        part(Some(1), [4.0, -1.0, -6.5], [0.0; 3], LEFT_ARM),
        part(Some(8), [0.0, 3.0, -1.0], [0.0; 3], LEFT_HAND),
        part(Some(1), [-4.0, -1.0, -6.5], [0.0; 3], RIGHT_ARM),
        part(Some(10), [0.0, 3.0, 0.0], [0.0; 3], RIGHT_HAND),
        part(Some(0), [3.5, -3.0, 4.0], [0.0; 3], LEFT_LEG),
        part(Some(12), [2.0, 3.0, 0.0], [0.0; 3], LEFT_FOOT),
        part(Some(0), [-3.5, -3.0, 4.0], [0.0; 3], RIGHT_LEG),
        part(Some(14), [-2.0, 3.0, 0.0], [0.0; 3], RIGHT_FOOT),
    ];
    use crate::animation::Animation;
    use crate::animations::{FROG_CROAK, FROG_IDLE_WATER, FROG_JUMP, FROG_SWIM, FROG_TONGUE};
    for (anim, t) in [&FROG_JUMP, &FROG_CROAK, &FROG_TONGUE]
        .into_iter()
        .zip(anims)
    {
        if let Some(t) = t {
            anim.apply(t, BONES, &mut nodes);
        }
    }
    let (walk, factor) = if swimming {
        (&FROG_SWIM, 1.0)
    } else {
        (&crate::animations::FROG_WALK, 1.5)
    };
    let (millis, k) = Animation::walk_millis(l.walk_pos, l.walk_speed, factor, 2.5);
    walk.apply_at(millis, k, BONES, &mut nodes);
    if let Some(t) = anims[3] {
        FROG_IDLE_WATER.apply(t, BONES, &mut nodes);
    }
    // Its throat shows only while it croaks.
    if anims[1].is_none() {
        hide(&mut nodes, 6);
    }
    nodes
}

/// `CamelModel.setupAnim`: its head follows its look, within 30° either side and 25° up
/// to 45° down; then its walk, sitting down, sitting, standing up, idle and dash.
fn camel_anim(
    l: &Living,
    anims: [Option<f32>; 5],
    animations: [&crate::animation::Animation; 6],
    bones: &[&str],
    nodes: &mut [Node],
) {
    let head = bones.iter().position(|b| *b == "head").expect("a head");
    nodes[head].pose[3] = l.pitch.clamp(-25.0, 45.0).to_radians();
    nodes[head].pose[4] = l.head_yaw.clamp(-30.0, 30.0).to_radians();
    let (millis, k) = crate::animation::Animation::walk_millis(l.walk_pos, l.walk_speed, 2.0, 2.5);
    animations[0].apply_at(millis, k, bones, nodes);
    for (anim, t) in animations[1..].iter().zip(anims) {
        if let Some(t) = t {
            anim.apply(t, bones, nodes);
        }
    }
}

/// `AdultCamelModel` (128 × 128): a long body with a hump and a tail, a long neck and
/// head with ears, and long legs.
fn camel(l: &Living, anims: [Option<f32>; 5]) -> Vec<Node> {
    const BODY: &[Cube] = &[cube([-7.5, -12.0, -23.5], [15.0, 12.0, 27.0], [0.0, 25.0])];
    const HUMP: &[Cube] = &[cube([-4.5, -5.0, -5.5], [9.0, 5.0, 11.0], [74.0, 0.0])];
    const TAIL: &[Cube] = &[cube([-1.5, 0.0, 0.0], [3.0, 14.0, 0.0], [122.0, 0.0])];
    const HEAD: &[Cube] = &[
        cube([-3.5, -7.0, -15.0], [7.0, 8.0, 19.0], [60.0, 24.0]),
        cube([-3.5, -21.0, -15.0], [7.0, 14.0, 7.0], [21.0, 0.0]),
        cube([-2.5, -21.0, -21.0], [5.0, 5.0, 6.0], [50.0, 0.0]),
    ];
    const LEFT_EAR: &[Cube] = &[cube([-0.5, 0.5, -1.0], [3.0, 1.0, 2.0], [45.0, 0.0])];
    const RIGHT_EAR: &[Cube] = &[cube([-2.5, 0.5, -1.0], [3.0, 1.0, 2.0], [67.0, 0.0])];
    const fn leg(u: f32, v: f32) -> [Cube; 1] {
        [cube([-2.5, 2.0, -2.5], [5.0, 21.0, 5.0], [u, v])]
    }
    const LEFT_HIND: &[Cube] = &leg(58.0, 16.0);
    const RIGHT_HIND: &[Cube] = &leg(94.0, 16.0);
    const LEFT_FRONT: &[Cube] = &leg(0.0, 0.0);
    const RIGHT_FRONT: &[Cube] = &leg(0.0, 26.0);
    const BONES: &[&str] = &[
        "root",
        "body",
        "hump",
        "tail",
        "head",
        "left_ear",
        "right_ear",
        "left_hind_leg",
        "right_hind_leg",
        "left_front_leg",
        "right_front_leg",
    ];
    let mut nodes = vec![
        part(None, [0.0; 3], [0.0; 3], &[]),
        part(Some(0), [0.0, 4.0, 9.5], [0.0; 3], BODY),
        part(Some(1), [0.0, -12.0, -10.0], [0.0; 3], HUMP),
        part(Some(1), [0.0, -9.0, 3.5], [0.0; 3], TAIL),
        part(Some(1), [0.0, -3.0, -19.5], [0.0; 3], HEAD),
        part(Some(4), [2.5, -21.0, -9.5], [0.0; 3], LEFT_EAR),
        part(Some(4), [-2.5, -21.0, -9.5], [0.0; 3], RIGHT_EAR),
        part(Some(0), [4.9, 1.0, 9.5], [0.0; 3], LEFT_HIND),
        part(Some(0), [-4.9, 1.0, 9.5], [0.0; 3], RIGHT_HIND),
        part(Some(0), [4.9, 1.0, -10.5], [0.0; 3], LEFT_FRONT),
        part(Some(0), [-4.9, 1.0, -10.5], [0.0; 3], RIGHT_FRONT),
    ];
    use crate::animations::*;
    let animations = [
        &CAMEL_WALK,
        &CAMEL_SIT,
        &CAMEL_SIT_POSE,
        &CAMEL_STANDUP,
        &CAMEL_IDLE,
        &CAMEL_DASH,
    ];
    camel_anim(l, anims, animations, BONES, &mut nodes);
    nodes
}

/// `BabyCamelModel` (64 × 64): a short body and neck on long thin legs.
fn baby_camel(l: &Living, anims: [Option<f32>; 5]) -> Vec<Node> {
    const BODY: &[Cube] = &[cube([-4.5, -4.0, -8.0], [9.0, 8.0, 16.0], [0.0, 14.0])];
    const TAIL: &[Cube] = &[cube([-1.5, -0.5, 0.0], [3.0, 9.0, 0.0], [50.0, 38.0])];
    const HEAD: &[Cube] = &[
        cube([-2.5, -3.0, -7.5], [5.0, 5.0, 7.0], [20.0, 0.0]),
        cube([-2.5, -12.0, -7.5], [5.0, 9.0, 5.0], [0.0, 0.0]),
        cube([-2.5, -12.0, -10.5], [5.0, 4.0, 3.0], [0.0, 14.0]),
    ];
    const RIGHT_EAR: &[Cube] = &[cube([-3.0, -0.5, -1.0], [3.0, 1.0, 2.0], [37.0, 0.0])];
    const LEFT_EAR: &[Cube] = &[cube([0.0, -0.5, -1.0], [3.0, 1.0, 2.0], [47.0, 0.0])];
    const fn leg(u: f32, v: f32) -> [Cube; 1] {
        [cube([-1.5, -0.5, -1.5], [3.0, 13.0, 3.0], [u, v])]
    }
    const RIGHT_FRONT: &[Cube] = &leg(36.0, 14.0);
    const LEFT_FRONT: &[Cube] = &leg(48.0, 14.0);
    const LEFT_HIND: &[Cube] = &leg(12.0, 38.0);
    const RIGHT_HIND: &[Cube] = &leg(0.0, 38.0);
    const BONES: &[&str] = &[
        "root",
        "body",
        "tail",
        "head",
        "right_ear",
        "left_ear",
        "right_front_leg",
        "left_front_leg",
        "left_hind_leg",
        "right_hind_leg",
    ];
    let mut nodes = vec![
        part(None, [0.0; 3], [0.0; 3], &[]),
        part(Some(0), [0.0, 7.0, 0.0], [0.0; 3], BODY),
        part(Some(1), [0.0, -1.5, 8.05], [0.0; 3], TAIL),
        part(Some(1), [0.0, 1.0, -7.5], [0.0; 3], HEAD),
        part(Some(3), [-2.5, -11.0, -4.0], [0.0; 3], RIGHT_EAR),
        part(Some(3), [2.5, -11.0, -4.0], [0.0; 3], LEFT_EAR),
        part(Some(0), [-3.0, 11.5, -5.5], [0.0; 3], RIGHT_FRONT),
        part(Some(0), [3.0, 11.5, -5.5], [0.0; 3], LEFT_FRONT),
        part(Some(0), [3.0, 11.5, 5.5], [0.0; 3], LEFT_HIND),
        part(Some(0), [-3.0, 11.5, 5.5], [0.0; 3], RIGHT_HIND),
    ];
    use crate::animations::*;
    let animations = [
        &CAMEL_BABY_WALK,
        &CAMEL_BABY_SIT,
        &CAMEL_BABY_SIT_POSE,
        &CAMEL_BABY_STANDUP,
        &CAMEL_BABY_IDLE,
        &CAMEL_BABY_DASH,
    ];
    camel_anim(l, anims, animations, BONES, &mut nodes);
    nodes
}

/// `ArmadilloModel.setupAnim` on `AdultArmadilloModel` (64 × 64) or `BabyArmadilloModel`:
/// hidden in its shell it's a ball (`cube`) with its front legs and head showing, its
/// body's own cubes, hind legs and tail hidden; otherwise its head follows its look
/// (22.5° up to 25° down, 32.5° either side) and it walks. Then it rolls out, rolls up
/// and peeks.
#[allow(clippy::approx_constant)]
fn armadillo(l: &Living, hiding: bool, anims: [Option<f32>; 3]) -> Vec<Node> {
    const BODY: &[Cube] = &[
        grown(
            cube([-4.0, -7.0, -10.0], [8.0, 8.0, 12.0], [0.0, 20.0]),
            0.3,
        ),
        cube([-4.0, -7.0, -10.0], [8.0, 8.0, 12.0], [0.0, 40.0]),
    ];
    const TAIL: &[Cube] = &[cube([-0.5, -0.0865, 0.0933], [1.0, 6.0, 1.0], [44.0, 53.0])];
    const HEAD: &[Cube] = &[cube([-1.5, -1.0, -1.0], [3.0, 5.0, 2.0], [43.0, 15.0])];
    const RIGHT_EAR: &[Cube] = &[cube([-2.0, -3.0, 0.0], [2.0, 5.0, 0.0], [43.0, 10.0])];
    const LEFT_EAR: &[Cube] = &[cube([0.0, -3.0, 0.0], [2.0, 5.0, 0.0], [47.0, 10.0])];
    const fn leg(u: f32, v: f32) -> [Cube; 1] {
        [cube([-1.0, 0.0, -1.0], [2.0, 3.0, 2.0], [u, v])]
    }
    const RIGHT_HIND: &[Cube] = &leg(51.0, 31.0);
    const LEFT_HIND: &[Cube] = &leg(42.0, 31.0);
    const RIGHT_FRONT: &[Cube] = &leg(51.0, 43.0);
    const LEFT_FRONT: &[Cube] = &leg(42.0, 43.0);
    const BALL: &[Cube] = &[cube([-5.0, -10.0, -6.0], [10.0; 3], [0.0, 0.0])];
    const BONES: &[&str] = &[
        "body",
        "tail",
        "head",
        "head_cube",
        "right_ear",
        "right_ear_cube",
        "left_ear",
        "left_ear_cube",
        "right_hind_leg",
        "left_hind_leg",
        "right_front_leg",
        "left_front_leg",
        "cube",
    ];
    const BABY_BODY: &[Cube] = &[
        grown(cube([-2.5, -2.0, -3.5], [5.0, 4.0, 7.0], [0.0, 0.0]), 0.3),
        cube([-2.5, -2.0, -3.0], [5.0, 4.0, 6.0], [0.0, 11.0]),
    ];
    const BABY_TAIL: &[Cube] = &[cube([-0.5, -0.5, -2.0], [1.0, 1.0, 4.0], [22.0, 11.0])];
    const BABY_HEAD: &[Cube] = &[cube([-1.0, -2.0, -4.0], [2.0, 2.0, 4.0], [20.0, 17.0])];
    const BABY_RIGHT_EAR: &[Cube] = &[mirrored(cube(
        [-1.8, -2.0, 0.0],
        [2.0, 3.0, 0.0],
        [28.0, 8.0],
    ))];
    const BABY_LEFT_EAR: &[Cube] = &[cube([-0.2, -2.0, 0.0], [2.0, 3.0, 0.0], [28.0, 8.0])];
    const BABY_LEG: &[Cube] = &[cube([-1.0, 0.0, -1.0], [2.0; 3], [20.0, 27.0])];
    const BABY_LEG_MIRRORED: &[Cube] = &[mirrored(cube([-1.0, 0.0, -1.0], [2.0; 3], [20.0, 27.0]))];
    const BABY_RIGHT_FRONT: &[Cube] = &[cube([-1.0, 0.0, -1.0], [2.0; 3], [20.0, 23.0])];
    const BABY_LEFT_FRONT: &[Cube] = &[mirrored(cube([-1.0, 0.0, -1.0], [2.0; 3], [24.0, 0.0]))];
    const BABY_BALL: &[Cube] = &[grown(cube([-3.0; 3], [6.0; 3], [0.0, 25.0]), 0.3)];
    // The baby's tail cube is named "right_ear_cube" in vanilla too.
    const BABY_BONES: &[&str] = &[
        "body",
        "tail",
        "right_ear_cube",
        "head",
        "head_cube",
        "right_ear",
        "left_ear",
        "right_hind_leg",
        "left_hind_leg",
        "right_front_leg",
        "left_front_leg",
        "cube",
    ];
    use crate::animations::*;
    let (mut nodes, bones, animations, [tail, hind, ball]) = if l.baby {
        let nodes = vec![
            part(None, [0.0, 20.0, 0.5], [0.0; 3], BABY_BODY),
            part(Some(0), [0.0, 0.0, 3.4], [0.0; 3], &[]),
            part(Some(1), [0.0, 1.5, 1.0], [-1.0472, 0.0, 0.0], BABY_TAIL),
            part(Some(0), [0.0, 0.0, -3.2], [0.0; 3], &[]),
            part(Some(3), [0.0; 3], [0.7417649, 0.0, 0.0], BABY_HEAD),
            part(
                Some(4),
                [-1.0, -2.0, -0.3],
                [-0.4363, -0.1134, 0.0524],
                BABY_RIGHT_EAR,
            ),
            part(
                Some(4),
                [1.0, -2.0, -0.3],
                [-0.4363, 0.1134, -0.0524],
                BABY_LEFT_EAR,
            ),
            part(None, [-1.5, 22.0, 2.5], [0.0; 3], BABY_LEG_MIRRORED),
            part(None, [1.5, 22.0, 2.5], [0.0; 3], BABY_LEG),
            part(None, [1.5, 22.0, -1.5], [0.0; 3], BABY_RIGHT_FRONT),
            part(None, [-1.5, 22.0, -1.5], [0.0; 3], BABY_LEFT_FRONT),
            part(None, [0.0, 20.7, 0.5], [0.0; 3], BABY_BALL),
        ];
        let animations = [
            &ARMADILLO_BABY_WALK,
            &ARMADILLO_BABY_ROLL_OUT,
            &ARMADILLO_BABY_ROLL_UP,
            &ARMADILLO_BABY_PEEK,
        ];
        (nodes, BABY_BONES, animations, [1, 7, 11])
    } else {
        let nodes = vec![
            part(None, [0.0, 21.0, 4.0], [0.0; 3], BODY),
            part(Some(0), [0.0, -3.0, 1.0], [0.5061, 0.0, 0.0], TAIL),
            part(Some(0), [0.0, -2.0, -11.0], [0.0; 3], &[]),
            part(Some(2), [0.0; 3], [-0.3927, 0.0, 0.0], HEAD),
            part(Some(2), [-1.0, -1.0, 0.0], [0.0; 3], &[]),
            part(
                Some(4),
                [-0.5, 0.0, -0.6],
                [0.1886, -0.3864, -0.0718],
                RIGHT_EAR,
            ),
            part(Some(2), [1.0, -2.0, 0.0], [0.0; 3], &[]),
            part(
                Some(6),
                [0.5, 1.0, -0.6],
                [0.1886, 0.3864, 0.0718],
                LEFT_EAR,
            ),
            part(None, [-2.0, 21.0, 4.0], [0.0; 3], RIGHT_HIND),
            part(None, [2.0, 21.0, 4.0], [0.0; 3], LEFT_HIND),
            part(None, [-2.0, 21.0, -4.0], [0.0; 3], RIGHT_FRONT),
            part(None, [2.0, 21.0, -4.0], [0.0; 3], LEFT_FRONT),
            part(None, [0.0, 24.0, 0.0], [0.0; 3], BALL),
        ];
        let animations = [
            &ARMADILLO_WALK,
            &ARMADILLO_ROLL_OUT,
            &ARMADILLO_ROLL_UP,
            &ARMADILLO_PEEK,
        ];
        (nodes, BONES, animations, [1, 8, 12])
    };
    if hiding {
        // `body.skipDraw`: its head still shows.
        nodes[0].cubes = &[];
        hide(&mut nodes, hind);
        hide(&mut nodes, hind + 1);
        hide(&mut nodes, tail);
    } else {
        hide(&mut nodes, ball);
        let head = bones.iter().position(|b| *b == "head").expect("a head");
        nodes[head].pose[3] = l.pitch.clamp(-22.5, 25.0).to_radians();
        nodes[head].pose[4] = l.head_yaw.clamp(-32.5, 32.5).to_radians();
        let (millis, k) =
            crate::animation::Animation::walk_millis(l.walk_pos, l.walk_speed, 16.5, 2.5);
        animations[0].apply_at(millis, k, bones, &mut nodes);
    }
    for (anim, t) in animations[1..].iter().zip(anims) {
        if let Some(t) = t {
            anim.apply(t, bones, &mut nodes);
        }
    }
    nodes
}

/// `AdultFelineModel.createBodyMesh`'s cubes, grown by its deformation: head (with nose
/// and ears), body, the two tail parts, a hind leg and a front leg.
struct Feline {
    head: [Cube; 4],
    body: [Cube; 1],
    tail1: [Cube; 1],
    tail2: [Cube; 1],
    hind: [Cube; 1],
    front: [Cube; 1],
}

const fn feline_cubes(g: f32) -> Feline {
    Feline {
        head: [
            grown(cube([-2.5, -2.0, -3.0], [5.0, 4.0, 5.0], [0.0, 0.0]), g),
            grown(cube([-1.5, -0.001, -4.0], [3.0, 2.0, 2.0], [0.0, 24.0]), g),
            grown(cube([-2.0, -3.0, 0.0], [1.0, 1.0, 2.0], [0.0, 10.0]), g),
            grown(cube([1.0, -3.0, 0.0], [1.0, 1.0, 2.0], [6.0, 10.0]), g),
        ],
        body: [grown(
            cube([-2.0, 3.0, -8.0], [4.0, 16.0, 6.0], [20.0, 0.0]),
            g,
        )],
        tail1: [grown(
            cube([-0.5, 0.0, 0.0], [1.0, 8.0, 1.0], [0.0, 15.0]),
            g,
        )],
        // Its own deformation, whatever the model's.
        tail2: [grown(
            cube([-0.5, 0.0, 0.0], [1.0, 8.0, 1.0], [4.0, 15.0]),
            -0.02,
        )],
        hind: [grown(
            cube([-1.0, 0.0, 1.0], [2.0, 6.0, 2.0], [8.0, 13.0]),
            g,
        )],
        front: [grown(
            cube([-1.0, 0.0, 0.0], [2.0, 10.0, 2.0], [40.0, 0.0]),
            g,
        )],
    }
}

static FELINE: Feline = feline_cubes(0.0);
/// `AdultCatModel.COLLAR_DEFORMATION`.
static FELINE_COLLAR: Feline = feline_cubes(0.01);

/// `AdultFelineModel.setupAnim`: walking, crouching, sprinting or sitting, then lying down
/// on its side (`lie`, `lie_tail`) and lifting its head (`relax`).
fn feline(
    l: &Living,
    f: &'static Feline,
    pose: FelinePose,
    lie: f32,
    lie_tail: f32,
    relax: f32,
) -> Vec<Node> {
    let (p, s) = (l.walk_pos, l.walk_speed);
    let mut head = ([0.0, 15.0, -9.0], [0.0; 3]);
    let mut body = ([0.0, 12.0, -10.0], [PI / 2.0, 0.0, 0.0]);
    let mut tail1 = ([0.0, 15.0, 8.0], [0.9, 0.0, 0.0]);
    let mut tail2 = ([0.0, 20.0, 14.0], [0.0; 3]);
    let mut left_hind = ([1.1, 18.0, 5.0], [0.0; 3]);
    let mut right_hind = ([-1.1, 18.0, 5.0], [0.0; 3]);
    let mut left_front = ([1.2, 14.1, -5.0], [0.0; 3]);
    let mut right_front = ([-1.2, 14.1, -5.0], [0.0; 3]);
    match pose {
        FelinePose::Crouching => {
            body.0[1] += 1.0;
            head.0[1] += 2.0;
            tail1.0[1] += 1.0;
            tail2.0[1] -= 4.0;
            tail2.0[2] += 2.0;
            tail1.1[0] = PI / 2.0;
            tail2.1[0] = PI / 2.0;
        }
        FelinePose::Sprinting => {
            tail2.0[1] = tail1.0[1];
            tail2.0[2] += 2.0;
            tail1.1[0] = PI / 2.0;
            tail2.1[0] = PI / 2.0;
        }
        _ => {}
    }
    head.1 = [l.pitch.to_radians(), l.head_yaw.to_radians(), 0.0];
    if pose == FelinePose::Sitting {
        body.1[0] = PI / 4.0;
        body.0[1] -= 4.0;
        body.0[2] += 5.0;
        head.0[1] -= 3.3;
        head.0[2] += 1.0;
        tail1.0[1] += 8.0;
        tail1.0[2] -= 2.0;
        tail2.0[1] += 2.0;
        tail2.0[2] -= 0.8;
        tail1.1[0] = 1.7278761;
        tail2.1[0] = 2.670354;
        for leg in [&mut left_front, &mut right_front] {
            leg.1[0] = -PI / 20.0;
            leg.0[1] += 2.0;
            leg.0[2] -= 2.0;
        }
        for leg in [&mut left_hind, &mut right_hind] {
            leg.1[0] = -PI / 2.0;
            leg.0[1] += 3.0;
            leg.0[2] -= 4.0;
        }
    } else {
        body.1[0] = PI / 2.0;
        let q = p * 0.6662;
        if pose == FelinePose::Sprinting {
            left_hind.1[0] = q.cos() * s;
            right_hind.1[0] = (q + 0.3).cos() * s;
            left_front.1[0] = (q + PI + 0.3).cos() * s;
            right_front.1[0] = (q + PI).cos() * s;
            tail2.1[0] = 1.7278761 + PI / 10.0 * p.cos() * s;
        } else {
            left_hind.1[0] = q.cos() * s;
            right_hind.1[0] = (q + PI).cos() * s;
            left_front.1[0] = (q + PI).cos() * s;
            right_front.1[0] = q.cos() * s;
            let wag = if pose == FelinePose::Crouching {
                0.47123894
            } else {
                PI / 4.0
            };
            tail2.1[0] = 1.7278761 + wag * p.cos() * s;
        }
    }
    // `Mth.rotLerp`, which these small angles never wrap.
    let lerp = |t: f32, a: f32, b: f32| a + t * (b - a);
    if lie > 0.0 {
        head.1[2] = lerp(lie, head.1[2], -1.2707963);
        head.1[1] = lerp(lie, head.1[1], 1.2707963);
        left_front.1[0] = -1.2707963;
        right_front.1[0] = -0.47079635;
        right_front.1[2] = -0.2;
        right_front.0[0] += 1.0;
        left_hind.1[0] = -0.4;
        right_hind.1[0] = 0.5;
        right_hind.1[2] = -0.5;
        right_hind.0[0] += 0.8;
        right_hind.0[1] += 2.0;
        tail1.1[0] = lerp(lie_tail, tail1.1[0], 0.8);
        tail2.1[0] = lerp(lie_tail, tail2.1[0], -0.4);
    }
    if relax > 0.0 {
        head.1[0] = lerp(relax, head.1[0], -0.58177644);
    }
    vec![
        part(None, head.0, head.1, &f.head),
        part(None, body.0, body.1, &f.body),
        part(None, tail1.0, tail1.1, &f.tail1),
        part(None, tail2.0, tail2.1, &f.tail2),
        part(None, left_hind.0, left_hind.1, &f.hind),
        part(None, right_hind.0, right_hind.1, &f.hind),
        part(None, left_front.0, left_front.1, &f.front),
        part(None, right_front.0, right_front.1, &f.front),
    ]
}

/// `BabyFelineModel` (32 × 32), for baby cats and ocelots, posed by its own `setupAnim`:
/// walking, crouching, sprinting or sitting, then lying down on its side (`lie`,
/// `lie_tail`) and lifting its head (`relax`). Its second tail part is empty.
fn baby_feline(l: &Living, pose: FelinePose, lie: f32, lie_tail: f32, relax: f32) -> Vec<Node> {
    const HEAD: &[Cube] = &[
        cube([-2.5, -3.0, -2.875], [5.0, 4.0, 4.0], [0.0, 0.0]),
        cube([-2.0, -4.0, -0.875], [1.0, 1.0, 2.0], [18.0, 0.0]),
        cube([1.0, -4.0, -0.875], [1.0, 1.0, 2.0], [24.0, 0.0]),
        cube([-1.5, -1.0, -3.875], [3.0, 2.0, 1.0], [18.0, 3.0]),
    ];
    const BODY: &[Cube] = &[cube([-2.0, -1.5, -3.5], [4.0, 3.0, 7.0], [0.0, 8.0])];
    const TAIL: &[Cube] = &[cube([-0.5, -0.107, 0.0849], [1.0, 1.0, 5.0], [0.0, 18.0])];
    const fn leg(u: f32, v: f32) -> [Cube; 1] {
        [cube([-0.5, 0.0, -1.0], [1.0, 2.0, 2.0], [u, v])]
    }
    // Left front, right front, left hind, right hind.
    const LEGS: [[Cube; 1]; 4] = [
        leg(18.0, 18.0),
        leg(12.0, 18.0),
        leg(18.0, 22.0),
        leg(12.0, 22.0),
    ];
    let age = 0.5;
    let (p, s) = (l.walk_pos, l.walk_speed);
    let mut head = ([0.0, 20.0, -3.125], [0.0; 3]);
    let mut body = ([0.0, 20.5, 0.5], [0.0; 3]);
    let mut tail = ([0.0, 19.107, 3.9151], [-0.567232, 0.0, 0.0]);
    let mut legs = [
        ([1.0, 22.0, -1.5], [0.0; 3]),
        ([-1.0, 22.0, -1.5], [0.0; 3]),
        ([1.0, 22.0, 2.5], [0.0; 3]),
        ([-1.0, 22.0, 2.5], [0.0; 3]),
    ];
    match pose {
        FelinePose::Crouching => {
            body.0[1] += age;
            head.0[1] += 2.0 * age;
            tail.0[1] += age;
            tail.1[0] = PI / 2.0;
        }
        FelinePose::Sprinting => tail.1[0] = PI / 2.0,
        _ => {}
    }
    head.1 = [l.pitch.to_radians(), l.head_yaw.to_radians(), 0.0];
    if pose == FelinePose::Sitting {
        body.1[0] -= 0.43633232;
        body.0[1] += 1.0;
        head.0[2] += 0.75;
        tail.1[0] += 0.5454154;
        tail.0[1] += 4.0;
        tail.0[2] -= 0.9;
        legs[2].0[2] -= 0.9;
        legs[3].0[2] -= 0.9;
    } else {
        let q = p * 0.6662;
        let (right_hind, left_front) = if pose == FelinePose::Sprinting {
            ((q + 0.3).cos(), (q + PI + 0.3).cos())
        } else {
            ((q + PI).cos(), (q + PI).cos())
        };
        legs[2].1[0] = q.cos() * s;
        legs[3].1[0] = right_hind * s;
        legs[0].1[0] = left_front * s;
        legs[1].1[0] = if pose == FelinePose::Sprinting {
            (q + PI).cos()
        } else {
            q.cos()
        } * s;
    }
    // `Mth.rotLerp`, which these small angles never wrap.
    let lerp = |t: f32, a: f32, b: f32| a + t * (b - a);
    if lie > 0.0 {
        body.0[0] += 1.0;
        head.1[0] = lerp(lie, head.1[0], PI / 18.0);
        head.1[2] = lerp(lie, head.1[2], -PI * 5.0 / 12.0);
        head.0[0] += 1.0;
        head.0[1] += 0.75;
        head.0[2] -= 0.5;
        legs[1].1[0] = -PI / 4.0;
        legs[1].0[0] += 3.5;
        legs[1].0[1] -= 0.5;
        legs[0].1[0] = -PI / 2.0;
        legs[0].0[0] += 1.0;
        legs[0].0[1] -= 1.0;
        legs[0].0[2] -= 2.0;
        legs[3].1 = [PI * 2.0 / 9.0, PI / 9.0, -PI / 9.0];
        legs[3].0[0] += 2.5;
        legs[3].0[1] -= 0.25;
        legs[3].0[2] += 0.5;
        legs[2].0[0] += 1.0;
        legs[2].0[2] -= 1.0;
        // Added to, as vanilla does, not set.
        tail.1[0] += lerp(lie_tail, tail.1[0], -PI / 6.0);
        tail.1[1] += lerp(lie_tail, tail.1[1], 0.0);
        tail.1[2] += lerp(lie_tail, tail.1[2], -PI / 18.0);
        tail.0[0] += 1.0;
        tail.0[1] += 0.5;
        tail.0[2] -= 0.25;
    }
    if relax > 0.0 {
        head.1[0] = lerp(relax, head.1[0], -0.58177644);
    }
    let mut nodes = vec![
        part(None, head.0, head.1, HEAD),
        part(None, body.0, body.1, BODY),
        part(None, tail.0, tail.1, TAIL),
    ];
    for (i, (at, rot)) in legs.into_iter().enumerate() {
        nodes.push(part(None, at, rot, &LEGS[i]));
    }
    nodes
}

/// `AdultFoxModel` (48 × 32) posed by `FoxModel.setupAnim` from its `flags`: walking, then
/// crouching (wiggling, lowered by `crouch`), asleep curled on its side, or sitting, and
/// lifted again as it pounces; its head tilts by `roll` while it's interested.
fn fox(l: &Living, flags: u8, crouch: f32, roll: f32) -> Vec<Node> {
    const HEAD: &[Cube] = &[
        cube([-3.0, -2.0, -5.0], [8.0, 6.0, 6.0], [1.0, 5.0]),
        cube([-3.0, -4.0, -4.0], [2.0, 2.0, 1.0], [8.0, 1.0]),
        cube([3.0, -4.0, -4.0], [2.0, 2.0, 1.0], [15.0, 1.0]),
        cube([-1.0, 2.01, -8.0], [4.0, 2.0, 3.0], [6.0, 18.0]),
    ];
    const BODY: &[Cube] = &[cube([-3.0, 3.999, -3.5], [6.0, 11.0, 6.0], [24.0, 15.0])];
    const LEFT: &[Cube] = &[grown(
        cube([2.0, 0.5, -1.0], [2.0, 6.0, 2.0], [4.0, 24.0]),
        0.001,
    )];
    const RIGHT: &[Cube] = &[grown(
        cube([2.0, 0.5, -1.0], [2.0, 6.0, 2.0], [13.0, 24.0]),
        0.001,
    )];
    const TAIL: &[Cube] = &[cube([2.0, 0.0, -1.0], [4.0, 9.0, 5.0], [30.0, 0.0])];
    let (sitting, crouching, pouncing) = (flags & 1 != 0, flags & 4 != 0, flags & 16 != 0);
    let (sleeping, faceplanted) = (flags & 32 != 0, flags & 64 != 0);
    let (p, s) = (l.walk_pos * 0.6662, l.walk_speed);
    let mut head = ([-1.0, 16.5, -3.0], [0.0, 0.0, roll]);
    let mut body = ([0.0, 16.0, -6.0], [PI / 2.0, 0.0, 0.0]);
    let mut tail = ([-4.0, 15.0, -1.0], [-0.05235988, 0.0, 0.0]);
    // Right hind, left hind, right front, left front.
    let mut legs = [
        ([-5.0, 17.5, 7.0], [p.cos() * 1.4 * s, 0.0, 0.0], RIGHT),
        (
            [-1.0, 17.5, 7.0],
            [(p + PI).cos() * 1.4 * s, 0.0, 0.0],
            LEFT,
        ),
        (
            [-5.0, 17.5, 0.0],
            [(p + PI).cos() * 1.4 * s, 0.0, 0.0],
            RIGHT,
        ),
        ([-1.0, 17.5, 0.0], [p.cos() * 1.4 * s, 0.0, 0.0], LEFT),
    ];
    if crouching {
        body.1[0] += 0.10471976;
        head.0[1] += crouch;
        let wiggle = l.age.cos() * 0.05;
        body.1[1] = wiggle;
        legs[0].1[2] = wiggle;
        legs[1].1[2] = wiggle;
        legs[2].1[2] = wiggle / 2.0;
        legs[3].1[2] = wiggle / 2.0;
        body.0[1] += crouch;
    } else if sleeping {
        for leg in &mut legs {
            leg.2 = &[];
        }
        body.1[2] = -PI / 2.0;
        body.0[1] += 5.0;
        tail.1[0] = -PI * 5.0 / 6.0;
        head.0[0] += 2.0;
        head.0[1] += 2.99;
        head.1[1] = -PI * 2.0 / 3.0;
        head.1[2] = 0.0;
    } else if sitting {
        head.1[0] = 0.0;
        head.1[1] = 0.0;
        body.1[0] = PI / 6.0;
        body.0[1] -= 7.0;
        body.0[2] += 3.0;
        tail.1[0] = PI / 4.0;
        head.0[1] -= 6.5;
        head.0[2] += 2.75;
        legs[2].1[0] = -PI / 12.0;
        legs[3].1[0] = -PI / 12.0;
        for leg in &mut legs[..2] {
            leg.1[0] = -PI * 5.0 / 12.0;
            leg.0[1] += 4.0;
            leg.0[2] -= 0.25;
        }
        tail.0[2] -= 1.0;
    }
    if pouncing {
        body.0[1] -= crouch / 2.0;
        head.0[1] -= crouch / 2.0;
    }
    if !sleeping && !faceplanted && !crouching {
        head.1[0] = l.pitch.to_radians();
        head.1[1] = l.head_yaw.to_radians();
    }
    if sleeping {
        head.1 = [0.0, -PI * 2.0 / 3.0, (l.age * 0.027).cos() / 22.0];
    }
    let mut nodes = vec![
        part(None, head.0, head.1, HEAD),
        part(None, body.0, body.1, BODY),
        part(Some(1), tail.0, tail.1, TAIL),
    ];
    for (at, rot, cubes) in legs {
        nodes.push(part(None, at, rot, cubes));
    }
    nodes
}

/// `BabyFoxModel` (32 × 32) posed by `FoxModel.setupAnim`: it walks by
/// `FoxBabyAnimation.FOX_BABY_WALK` (legs stepping 35° and stretched, head lowered, tail
/// raised, as far as it's moving), then crouches, sleeps curled on its side
/// or sits as its own overrides pose it.
fn baby_fox(l: &Living, flags: u8, crouch: f32, roll: f32) -> Vec<Node> {
    const HEAD: &[Cube] = &[
        cube([-3.0, -2.125, -5.125], [6.0, 5.0, 5.0], [0.0, 0.0]),
        cube([-1.0, 0.875, -7.125], [2.0; 3], [18.0, 20.0]),
        cube([-3.0, -4.125, -4.125], [2.0, 2.0, 1.0], [22.0, 8.0]),
        cube([1.0, -4.125, -4.125], [2.0, 2.0, 1.0], [22.0, 11.0]),
    ];
    const BODY: &[Cube] = &[cube([-2.5, -2.0, -3.0], [5.0, 4.0, 6.0], [0.0, 10.0])];
    const TAIL: &[Cube] = &[cube([-1.5, -1.48, -1.0], [3.0, 3.0, 6.0], [0.0, 20.0])];
    const RIGHT: &[Cube] = &[cube([-1.0, 0.0, -1.0], [2.0; 3], [22.0, 4.0])];
    const LEFT: &[Cube] = &[cube([-1.0, 0.0, -1.0], [2.0; 3], [22.0, 0.0])];
    let (sitting, crouching) = (flags & 1 != 0, flags & 4 != 0);
    let (sleeping, faceplanted) = (flags & 32 != 0, flags & 64 != 0);
    let mut head = ([0.0, 18.125, 0.125], [0.0, 0.0, roll]);
    let mut body = ([0.0, 20.0, 2.0], [0.0; 3]);
    let mut tail = ([0.0, -0.5, 3.0], [0.0; 3]);
    // Right hind, left hind, right front, left front: pivot, turn, cubes and scale.
    let mut legs = [
        ([-1.5, 22.0, 4.0], [0.0; 3], RIGHT, [1.0; 3]),
        ([1.5, 22.0, 4.0], [0.0; 3], LEFT, [1.0; 3]),
        ([-1.5, 22.0, 0.0], [0.0; 3], RIGHT, [1.0; 3]),
        ([1.5, 22.0, 0.0], [0.0; 3], LEFT, [1.0; 3]),
    ];
    // `applyWalk(walkAnimationPos, walkAnimationSpeed, 1, 2.5)`: added to each part's pose.
    {
        use crate::animation::{Animation, Target};
        let anim = &crate::animations::FOX_BABY_WALK;
        let (millis, k) = Animation::walk_millis(l.walk_pos, l.walk_speed, 1.0, 2.5);
        let add = |(at, rot): &mut ([f32; 3], [f32; 3]), bone: &str| {
            let p = anim.sample(millis, k, bone, Target::Position);
            let r = anim.sample(millis, k, bone, Target::Rotation);
            for i in 0..3 {
                at[i] += p[i];
                rot[i] += r[i];
            }
        };
        add(&mut head, "head");
        add(&mut body, "body");
        add(&mut tail, "tail");
        let names = [
            "right_hind_leg",
            "left_hind_leg",
            "right_front_leg",
            "left_front_leg",
        ];
        for (leg, bone) in legs.iter_mut().zip(names) {
            let mut pose = (leg.0, leg.1);
            add(&mut pose, bone);
            (leg.0, leg.1) = pose;
            let s = anim.sample(millis, k, bone, Target::Scale);
            for i in 0..3 {
                leg.3[i] += s[i];
            }
        }
    }
    if crouching {
        body.1[0] += 0.10471976;
        head.0[1] += crouch * 0.5;
        let wiggle = l.age.cos() * 0.05;
        body.1[1] = wiggle;
        legs[0].1[2] = wiggle;
        legs[1].1[2] = wiggle;
        legs[2].1[2] = wiggle / 2.0;
        legs[3].1[2] = wiggle / 2.0;
        body.0[1] += crouch / 6.0;
    } else if sleeping {
        for leg in &mut legs {
            leg.2 = &[];
        }
        body.1[2] = -PI / 2.0;
        body.1[0] = -PI / 18.0;
        body.0[0] -= 1.0;
        body.0[1] += 1.0;
        body.0[2] -= 1.0;
        tail.1[0] = -2.1816616;
        tail.0[0] -= 0.7;
        tail.0[1] += 0.9;
        tail.0[2] += 0.6;
        head.0[0] -= 2.0;
        head.0[1] += 2.8;
        head.0[2] -= 4.0;
        head.1[1] = -PI * 2.0 / 3.0;
        head.1[2] = 0.0;
    } else if sitting {
        head.1[0] = 0.0;
        head.1[1] = 0.0;
        body.1[0] = -0.959931;
        body.0[2] -= 4.5 * 0.5;
        body.0[1] += 3.0 * 0.5;
        tail.0[1] -= 0.6;
        tail.0[2] -= 2.0 * 0.5;
        tail.1[0] = 0.95993114;
        head.0[1] -= 0.75;
        for (i, x) in [(2, 0.01), (3, -0.01)] {
            legs[i].1[0] = -PI / 12.0;
            legs[i].0[2] -= 1.0;
            legs[i].0[0] += x;
        }
        for (i, x) in [(0, 0.01), (1, -0.01)] {
            legs[i].0[2] -= 3.75;
            legs[i].0[0] += x;
        }
    }
    if !sleeping && !faceplanted && !crouching {
        head.1[0] = l.pitch.to_radians();
        head.1[1] = l.head_yaw.to_radians();
    }
    if sleeping {
        head.1 = [0.0, -PI * 2.0 / 3.0, (l.age * 0.027).cos() / 22.0];
    }
    let mut nodes = vec![
        part(None, head.0, head.1, HEAD),
        part(None, body.0, body.1, BODY),
        part(Some(1), tail.0, tail.1, TAIL),
    ];
    for (at, rot, cubes, scale) in legs {
        let mut leg = part(None, at, rot, cubes);
        leg.scale = scale;
        nodes.push(leg);
    }
    nodes
}

/// `BoggedModel`: [`humanoid`] with skeleton limbs and mushrooms, flat cards crossed in
/// pairs, on its head.
fn bogged(l: &Living, sheared: bool) -> Vec<Node> {
    const RED: &[Cube] = &[cube([-3.0, -3.0, 0.0], [6.0, 4.0, 0.0], [50.0, 16.0])];
    const BROWN: &[Cube] = &[cube([-3.0, -3.0, 0.0], [6.0, 4.0, 0.0], [50.0, 22.0])];
    const BROWN_LOW: &[Cube] = &[cube([-3.0, -4.0, 0.0], [6.0, 4.0, 0.0], [50.0, 28.0])];
    let mut nodes = humanoid(l, SKELETON_LIMBS, None);
    if !sheared {
        let q = PI / 4.0;
        nodes.extend([
            part(Some(0), [3.0, -8.0, 3.0], [0.0, q, 0.0], RED),
            part(Some(0), [3.0, -8.0, 3.0], [0.0, 3.0 * q, 0.0], RED),
            part(Some(0), [-3.0, -8.0, -3.0], [0.0, q, 0.0], BROWN),
            part(Some(0), [-3.0, -8.0, -3.0], [0.0, 3.0 * q, 0.0], BROWN),
            part(Some(0), [-2.0, -1.0, 4.0], [-PI / 2.0, 0.0, q], BROWN_LOW),
            part(
                Some(0),
                [-2.0, -1.0, 4.0],
                [-PI / 2.0, 0.0, 3.0 * q],
                BROWN_LOW,
            ),
        ]);
    }
    nodes
}

/// `SnowGolemModel`: three snowballs and stick arms, cubes shrunk half a pixel; its upper
/// body and arms turn a quarter as far as its head. The head is node 0.
fn snow_golem(l: &Living) -> Vec<Node> {
    const HEAD: &[Cube] = &[grown(cube([-4.0, -8.0, -4.0], [8.0; 3], [0.0, 0.0]), -0.5)];
    const ARM: &[Cube] = &[grown(
        cube([-1.0, 0.0, -1.0], [12.0, 2.0, 2.0], [32.0, 0.0]),
        -0.5,
    )];
    const UPPER: &[Cube] = &[grown(
        cube([-5.0, -10.0, -5.0], [10.0; 3], [0.0, 16.0]),
        -0.5,
    )];
    const LOWER: &[Cube] = &[grown(
        cube([-6.0, -12.0, -6.0], [12.0; 3], [0.0, 36.0]),
        -0.5,
    )];
    let yaw = l.head_yaw.to_radians();
    let body = yaw * 0.25;
    let (sin, cos) = body.sin_cos();
    vec![
        part(
            None,
            [0.0, 4.0, 0.0],
            [l.pitch.to_radians(), yaw, 0.0],
            HEAD,
        ),
        part(None, [cos * 5.0, 6.0, -sin * 5.0], [0.0, body, 1.0], ARM),
        part(
            None,
            [-cos * 5.0, 6.0, sin * 5.0],
            [0.0, body + PI, -1.0],
            ARM,
        ),
        part(None, [0.0, 13.0, 0.0], [0.0, body, 0.0], UPPER),
        part(None, [0.0, 24.0, 0.0], [0.0; 3], LOWER),
    ]
}

/// `MagmaCubeModel`: eight slices around a core, which spread apart as it leaps.
fn magma_cube(squish: f32) -> Vec<Node> {
    const fn slice(i: usize) -> [Cube; 1] {
        let (u, v) = match i {
            0 => (0.0, 0.0),
            1..=3 => (0.0, 9.0 * i as f32),
            _ => (32.0, 9.0 * i as f32 - 36.0),
        };
        [cube([-4.0, 16.0 + i as f32, -4.0], [8.0, 1.0, 8.0], [u, v])]
    }
    const SLICES: [[Cube; 1]; 8] = [
        slice(0),
        slice(1),
        slice(2),
        slice(3),
        slice(4),
        slice(5),
        slice(6),
        slice(7),
    ];
    const CORE: &[Cube] = &[cube([-2.0, 18.0, -2.0], [4.0; 3], [24.0, 40.0])];
    let squish = squish.max(0.0);
    let mut nodes: Vec<Node> = SLICES
        .iter()
        .enumerate()
        .map(|(i, c)| {
            part(
                None,
                [0.0, -(4.0 - i as f32) * squish * 1.7, 0.0],
                [0.0; 3],
                c,
            )
        })
        .collect();
    nodes.push(part(None, [0.0; 3], [0.0; 3], CORE));
    nodes
}

/// `BlazeModel`: a head and three rings of four rods, turning different ways at
/// different speeds and bobbing.
fn blaze(l: &Living) -> Vec<Node> {
    const HEAD: &[Cube] = &[cube([-4.0; 3], [8.0; 3], [0.0, 0.0])];
    const ROD: &[Cube] = &[cube([0.0; 3], [2.0, 8.0, 2.0], [0.0, 16.0])];
    let t = l.age;
    let look = [l.pitch.to_radians(), l.head_yaw.to_radians(), 0.0];
    let mut nodes = vec![part(None, [0.0; 3], look, HEAD)];
    // Each ring: its first rod's angle, its radius, and its height, bobbing as
    // `cos((i * step + age) * speed)` for rod `i`.
    let rings = [
        (t * PI * -0.1, 9.0, -2.0, 2.0, 0.25),
        (PI / 4.0 + t * PI * 0.03, 7.0, 2.0, 2.0, 0.25),
        (0.47123894 + t * PI * -0.05, 5.0, 11.0, 1.5, 0.5),
    ];
    for (ring, (start, radius, height, step, speed)) in rings.into_iter().enumerate() {
        for k in 0..4 {
            let i = (ring * 4 + k) as f32;
            let angle = start + k as f32;
            let y = height + ((i * step + t) * speed).cos();
            let at = [angle.cos() * radius, y, angle.sin() * radius];
            nodes.push(part(None, at, [0.0; 3], ROD));
        }
    }
    nodes
}

/// `GhastModel`: a body over nine tentacles, whose lengths come from a `Random` seeded
/// with 1660, swaying.
fn ghast(l: &Living) -> Vec<Node> {
    const BODY: &[Cube] = &[cube([-8.0; 3], [16.0; 3], [0.0, 0.0])];
    const fn tentacle(len: f32) -> [Cube; 1] {
        [cube([-1.0, 0.0, -1.0], [2.0, len, 2.0], [0.0, 0.0])]
    }
    const TENTACLES: [[Cube; 1]; 9] = [
        tentacle(8.0),
        tentacle(13.0),
        tentacle(9.0),
        tentacle(11.0),
        tentacle(11.0),
        tentacle(10.0),
        tentacle(12.0),
        tentacle(9.0),
        tentacle(12.0),
    ];
    let mut nodes = vec![part(None, [0.0, 17.6, 0.0], [0.0; 3], BODY)];
    for (i, cubes) in TENTACLES.iter().enumerate() {
        let x = ((i % 3) as f32 - (i / 3 % 2) as f32 * 0.5 + 0.25) / 2.0 * 2.0 - 1.0;
        let z = (i / 3) as f32 / 2.0 * 2.0 - 1.0;
        let swing = 0.2 * (l.age * 0.3 + i as f32).sin() + 0.4;
        nodes.push(part(
            None,
            [x * 5.0, 24.6, z * 5.0],
            [swing, 0.0, 0.0],
            cubes,
        ));
    }
    nodes
}

/// A segmented crawler's body (`SilverfishModel`, `EndermiteModel`): each segment's
/// size and texture offset, from head to tail.
fn segments(sizes: &[[f32; 3]], texs: &[[f32; 2]]) -> (Vec<Cube>, Vec<f32>) {
    let mut placement = -3.5;
    let mut cubes = Vec::new();
    let mut zs = Vec::new();
    for (i, (&[w, h, d], &tex)) in sizes.iter().zip(texs).enumerate() {
        cubes.push(cube([w * -0.5, 0.0, d * -0.5], [w, h, d], tex));
        zs.push(placement);
        if let Some(next) = sizes.get(i + 1) {
            placement += (d + next[2]) * 0.5;
        }
    }
    (cubes, zs)
}

/// One segment's wriggle: its yaw and sideways shift (`SilverfishModel.setupAnim`).
fn wriggle(age: f32, i: usize, turn: f32, shift: f32) -> (f32, f32) {
    let phase = age * 0.9 + i as f32 * 0.15 * PI;
    let off = (i as f32 - 2.0).abs();
    (
        phase.cos() * PI * turn * (1.0 + off),
        phase.sin() * PI * shift * off,
    )
}

/// `SilverfishModel`: seven segments, three with spines over them, wriggling.
fn silverfish(l: &Living) -> Vec<Node> {
    use std::sync::LazyLock;
    const SIZES: [[f32; 3]; 7] = [
        [3.0, 2.0, 2.0],
        [4.0, 3.0, 2.0],
        [6.0, 4.0, 3.0],
        [3.0, 3.0, 3.0],
        [2.0, 2.0, 3.0],
        [2.0, 1.0, 2.0],
        [1.0, 1.0, 2.0],
    ];
    const TEXS: [[f32; 2]; 7] = [
        [0.0, 0.0],
        [0.0, 4.0],
        [0.0, 9.0],
        [0.0, 16.0],
        [0.0, 22.0],
        [11.0, 0.0],
        [13.0, 4.0],
    ];
    static BODY: LazyLock<(Vec<Cube>, Vec<f32>)> = LazyLock::new(|| segments(&SIZES, &TEXS));
    static LAYERS: LazyLock<[[Cube; 1]; 3]> = LazyLock::new(|| {
        [
            [cube(
                [-5.0, 0.0, SIZES[2][2] * -0.5],
                [10.0, 8.0, SIZES[2][2]],
                [20.0, 0.0],
            )],
            [cube(
                [-3.0, 0.0, SIZES[4][2] * -0.5],
                [6.0, 4.0, SIZES[4][2]],
                [20.0, 11.0],
            )],
            [cube(
                [-3.0, 0.0, SIZES[4][2] * -0.5],
                [6.0, 5.0, SIZES[1][2]],
                [20.0, 18.0],
            )],
        ]
    });
    let (cubes, zs) = &*BODY;
    let w: Vec<(f32, f32)> = (0..7).map(|i| wriggle(l.age, i, 0.05, 0.2)).collect();
    let mut nodes: Vec<Node> = (0..7)
        .map(|i| {
            let at = [w[i].1, 24.0 - SIZES[i][1], zs[i]];
            part(
                None,
                at,
                [0.0, w[i].0, 0.0],
                std::slice::from_ref(&cubes[i]),
            )
        })
        .collect();
    let layers = &*LAYERS;
    nodes.push(part(
        None,
        [0.0, 16.0, zs[2]],
        [0.0, w[2].0, 0.0],
        &layers[0],
    ));
    nodes.push(part(
        None,
        [w[4].1, 20.0, zs[4]],
        [0.0, w[4].0, 0.0],
        &layers[1],
    ));
    nodes.push(part(
        None,
        [w[1].1, 19.0, zs[1]],
        [0.0, w[1].0, 0.0],
        &layers[2],
    ));
    nodes
}

/// `EndermiteModel`: four segments wriggling, less than a silverfish.
fn endermite(l: &Living) -> Vec<Node> {
    use std::sync::LazyLock;
    const SIZES: [[f32; 3]; 4] = [
        [4.0, 3.0, 2.0],
        [6.0, 4.0, 5.0],
        [3.0, 3.0, 1.0],
        [1.0, 2.0, 1.0],
    ];
    const TEXS: [[f32; 2]; 4] = [[0.0, 0.0], [0.0, 5.0], [0.0, 14.0], [0.0, 18.0]];
    static BODY: LazyLock<(Vec<Cube>, Vec<f32>)> = LazyLock::new(|| segments(&SIZES, &TEXS));
    let (cubes, zs) = &*BODY;
    (0..4)
        .map(|i| {
            let (yaw, x) = wriggle(l.age, i, 0.01, 0.1);
            let at = [x, 24.0 - SIZES[i][1], zs[i]];
            part(None, at, [0.0, yaw, 0.0], std::slice::from_ref(&cubes[i]))
        })
        .collect()
}

/// `PolarBearModel` (128 × 64): a `QuadrupedModel` that rears up on its hind legs as
/// `stand` goes to 1.
fn polar_bear(l: &Living, stand: f32) -> Vec<Node> {
    const HEAD: &[Cube] = &[
        cube([-3.5, -3.0, -3.0], [7.0; 3], [0.0, 0.0]),
        cube([-2.5, 1.0, -6.0], [5.0, 3.0, 3.0], [0.0, 44.0]),
        cube([-4.5, -4.0, -1.0], [2.0, 2.0, 1.0], [26.0, 0.0]),
        mirrored(cube([2.5, -4.0, -1.0], [2.0, 2.0, 1.0], [26.0, 0.0])),
    ];
    const BODY: &[Cube] = &[
        cube([-5.0, -13.0, -7.0], [14.0, 14.0, 11.0], [0.0, 19.0]),
        cube([-4.0, -25.0, -7.0], [12.0, 12.0, 10.0], [39.0, 0.0]),
    ];
    const HIND: &[Cube] = &[cube([-2.0, 0.0, -2.0], [4.0, 10.0, 8.0], [50.0, 22.0])];
    const FRONT: &[Cube] = &[cube([-2.0, 0.0, -2.0], [4.0, 10.0, 6.0], [50.0, 40.0])];
    let (p, s) = (l.walk_pos * 0.6662, l.walk_speed);
    let (a, b) = (p.cos() * 1.4 * s, (p + PI).cos() * 1.4 * s);
    let st = stand * stand;
    let age = if l.baby { 0.5 } else { 1.0 };
    let front_y = 14.0 - st * age * 20.0;
    let front_z = -8.0 + st * age * 4.0;
    let front_x = -st * PI * 0.45;
    vec![
        part(
            None,
            [0.0, 10.0 - st * 24.0, -16.0 + st * 13.0],
            [
                l.pitch.to_radians() + st * PI * 0.15,
                l.head_yaw.to_radians(),
                0.0,
            ],
            HEAD,
        ),
        part(
            None,
            [-2.0, 9.0 + st * age * 2.0, 12.0],
            [PI / 2.0 - st * PI * 0.35, 0.0, 0.0],
            BODY,
        ),
        part(None, [-4.5, 14.0, 6.0], [a, 0.0, 0.0], HIND),
        part(None, [4.5, 14.0, 6.0], [b, 0.0, 0.0], HIND),
        part(
            None,
            [-3.5, front_y, front_z],
            [b + front_x, 0.0, 0.0],
            FRONT,
        ),
        part(
            None,
            [3.5, front_y, front_z],
            [a + front_x, 0.0, 0.0],
            FRONT,
        ),
    ]
}

/// `BabyPolarBearModel` (64 × 64), posed by `PolarBearModel.setupAnim` as the adult is: its
/// body and front legs rise half as far (`ageScale`), its head the full way.
fn baby_polar_bear(l: &Living, stand: f32) -> Vec<Node> {
    const HEAD: &[Cube] = &[
        cube([-3.0, -2.625, -4.25], [6.0, 5.0, 4.0], [0.0, 0.0]),
        cube([-2.0, 0.375, -6.25], [4.0, 2.0, 2.0], [20.0, 3.0]),
        cube([-4.0, -3.625, -2.75], [2.0, 2.0, 1.0], [20.0, 0.0]),
        cube([2.0, -3.625, -2.75], [2.0, 2.0, 1.0], [26.0, 0.0]),
    ];
    const BODY: &[Cube] = &[cube([-4.0, -3.5, -6.0], [8.0, 7.0, 12.0], [0.0, 9.0])];
    const fn leg(u: f32, v: f32) -> [Cube; 1] {
        [cube([-1.5, -0.5, -1.5], [3.0; 3], [u, v])]
    }
    const LEGS: [[Cube; 1]; 4] = [
        leg(0.0, 34.0),
        leg(12.0, 34.0),
        leg(0.0, 28.0),
        leg(12.0, 28.0),
    ];
    let (p, s) = (l.walk_pos * 0.6662, l.walk_speed);
    let (a, b) = (p.cos() * 1.4 * s, (p + PI).cos() * 1.4 * s);
    let st = stand * stand;
    let front_y = 21.5 - st * 0.5 * 20.0;
    let front_z = -4.5 + st * 0.5 * 4.0;
    let front_x = -st * PI * 0.45;
    vec![
        part(
            None,
            [0.0, 18.625 - st * 24.0, -5.75 + st * 13.0],
            [
                l.pitch.to_radians() + st * PI * 0.15,
                l.head_yaw.to_radians(),
                0.0,
            ],
            HEAD,
        ),
        part(
            None,
            [0.0, 17.5 + st * 0.5 * 2.0, 0.0],
            [-st * PI * 0.35, 0.0, 0.0],
            BODY,
        ),
        part(None, [-2.5, 21.5, 4.5], [a, 0.0, 0.0], &LEGS[0]),
        part(None, [2.5, 21.5, 4.5], [b, 0.0, 0.0], &LEGS[1]),
        part(
            None,
            [-2.5, front_y, front_z],
            [b + front_x, 0.0, 0.0],
            &LEGS[2],
        ),
        part(
            None,
            [2.5, front_y, front_z],
            [a + front_x, 0.0, 0.0],
            &LEGS[3],
        ),
    ]
}

/// `BabyGoatModel` (64 × 64): a small `QuadrupedModel` with stubby horns it may have lost
/// and little ears; its head stays bowed at 22.5° while it isn't ramming
/// (`BabyGoatModel.setupAnim`). Its `HeadMain` cube is the head's again, so it's left out.
fn baby_goat(l: &Living, horns: [bool; 2]) -> Vec<Node> {
    const HEAD: &[Cube] = &[cube([-2.0, -3.8126, -5.1548], [4.0, 4.0, 6.0], [0.0, 0.0])];
    const RIGHT_HORN: &[Cube] = &[mirrored(cube(
        [0.0, -4.5, 0.0],
        [1.0, 2.0, 1.0],
        [24.0, 0.0],
    ))];
    const LEFT_HORN: &[Cube] = &[mirrored(cube(
        [2.0, -4.5, 0.0],
        [1.0, 2.0, 1.0],
        [24.0, 0.0],
    ))];
    const RIGHT_EAR: &[Cube] = &[mirrored(cube(
        [-2.0, -0.5, -0.5],
        [2.0, 1.0, 1.0],
        [0.0, 12.0],
    ))];
    const LEFT_EAR: &[Cube] = &[cube([0.0, -0.5, -0.5], [2.0, 1.0, 1.0], [0.0, 12.0])];
    const BODY: &[Cube] = &[
        cube([-3.0, -2.3, -4.5], [6.0, 5.0, 9.0], [0.0, 10.0]),
        cube([-2.5, -2.2, -4.0], [5.0, 4.0, 8.0], [0.0, 24.0]),
    ];
    const fn leg(u: f32, v: f32) -> [Cube; 1] {
        [cube([-1.0, -0.5, -1.0], [2.0, 5.0, 2.0], [u, v])]
    }
    // Right hind, left hind, right front, left front.
    const LEGS: [[Cube; 1]; 4] = [
        leg(21.0, 12.0),
        leg(29.0, 12.0),
        leg(21.0, 5.0),
        leg(29.0, 5.0),
    ];
    let (p, s) = (l.walk_pos * 0.6662, l.walk_speed);
    let (a, b) = (p.cos() * 1.4 * s, (p + PI).cos() * 1.4 * s);
    let horn = |has: bool, cubes: &'static [Cube]| if has { cubes } else { &[] };
    let tilt = [-PI / 8.0, 0.0, 0.0];
    vec![
        part(
            None,
            [0.0, 15.5, -3.0],
            [PI / 8.0, l.head_yaw.to_radians(), 0.0],
            HEAD,
        ),
        part(
            Some(0),
            [-1.5, -1.5, -1.0],
            tilt,
            horn(horns[1], RIGHT_HORN),
        ),
        part(Some(0), [-1.5, -1.5, -1.0], tilt, horn(horns[0], LEFT_HORN)),
        part(
            Some(0),
            [-1.7, -2.3126, 0.1452],
            [0.0, -PI / 6.0, 0.0],
            RIGHT_EAR,
        ),
        part(
            Some(0),
            [1.7, -2.3126, 0.1452],
            [0.0, PI / 6.0, 0.0],
            LEFT_EAR,
        ),
        part(None, [0.0, 17.8, 0.0], [0.0; 3], BODY),
        part(None, [-1.5, 19.5, 3.0], [a, 0.0, 0.0], &LEGS[0]),
        part(None, [1.5, 19.5, 3.0], [b, 0.0, 0.0], &LEGS[1]),
        part(None, [-1.5, 19.5, -2.0], [b, 0.0, 0.0], &LEGS[2]),
        part(None, [1.5, 19.5, -2.0], [a, 0.0, 0.0], &LEGS[3]),
    ]
}

/// `GoatModel`: a `QuadrupedModel` with ears, a goatee, a nose and horns it may have lost.
fn goat(l: &Living, horns: [bool; 2]) -> Vec<Node> {
    const HEAD: &[Cube] = &[
        cube([-6.0, -11.0, -10.0], [3.0, 2.0, 1.0], [2.0, 61.0]),
        mirrored(cube([2.0, -11.0, -10.0], [3.0, 2.0, 1.0], [2.0, 61.0])),
        cube([-0.5, -3.0, -14.0], [0.0, 7.0, 5.0], [23.0, 52.0]),
    ];
    const LEFT_HORN: &[Cube] = &[cube([-0.01, -16.0, -10.0], [2.0, 7.0, 2.0], [12.0, 55.0])];
    const RIGHT_HORN: &[Cube] = &[cube([-2.99, -16.0, -10.0], [2.0, 7.0, 2.0], [12.0, 55.0])];
    const NOSE: &[Cube] = &[cube([-3.0, -4.0, -8.0], [5.0, 7.0, 10.0], [34.0, 46.0])];
    const BODY: &[Cube] = &[
        cube([-4.0, -17.0, -7.0], [9.0, 11.0, 16.0], [1.0, 1.0]),
        cube([-5.0, -18.0, -8.0], [11.0, 14.0, 11.0], [0.0, 28.0]),
    ];
    const LEFT_HIND: &[Cube] = &[cube([0.0, 4.0, 0.0], [3.0, 6.0, 3.0], [36.0, 29.0])];
    const RIGHT_HIND: &[Cube] = &[cube([0.0, 4.0, 0.0], [3.0, 6.0, 3.0], [49.0, 29.0])];
    const LEFT_FRONT: &[Cube] = &[cube([0.0; 3], [3.0, 10.0, 3.0], [49.0, 2.0])];
    const RIGHT_FRONT: &[Cube] = &[cube([0.0; 3], [3.0, 10.0, 3.0], [35.0, 2.0])];
    let (p, s) = (l.walk_pos * 0.6662, l.walk_speed);
    let (a, b) = (p.cos() * 1.4 * s, (p + PI).cos() * 1.4 * s);
    let look = [l.pitch.to_radians(), l.head_yaw.to_radians(), 0.0];
    let horn = |has: bool, cubes: &'static [Cube]| if has { cubes } else { &[] };
    vec![
        part(None, [1.0, 14.0, 0.0], look, HEAD),
        part(Some(0), [0.0; 3], [0.0; 3], horn(horns[0], LEFT_HORN)),
        part(Some(0), [0.0; 3], [0.0; 3], horn(horns[1], RIGHT_HORN)),
        part(Some(0), [0.0, -8.0, -8.0], [0.9599, 0.0, 0.0], NOSE),
        part(None, [0.0, 24.0, 0.0], [0.0; 3], BODY),
        part(None, [1.0, 14.0, 4.0], [b, 0.0, 0.0], LEFT_HIND),
        part(None, [-3.0, 14.0, 4.0], [a, 0.0, 0.0], RIGHT_HIND),
        part(None, [1.0, 14.0, -6.0], [a, 0.0, 0.0], LEFT_FRONT),
        part(None, [-3.0, 14.0, -6.0], [b, 0.0, 0.0], RIGHT_FRONT),
    ]
}

/// `AdultPiglinModel`: `PlayerModel`'s wide limbs and jacket under a piglin's head with
/// its snout and tusks, and ears that flap as it walks (`AbstractPiglinModel.setupAnim`),
/// or twitch to the beat while it dances. Posed as a zombie for a zombified piglin.
fn piglin(l: &Living, zombie: Option<bool>) -> Vec<Node> {
    const HEAD: &[Cube] = &[
        cube([-5.0, -8.0, -4.0], [10.0, 8.0, 8.0], [0.0, 0.0]),
        cube([-2.0, -4.0, -5.0], [4.0, 4.0, 1.0], [31.0, 1.0]),
        cube([2.0, -2.0, -5.0], [1.0, 2.0, 1.0], [2.0, 4.0]),
        cube([-3.0, -2.0, -5.0], [1.0, 2.0, 1.0], [2.0, 0.0]),
    ];
    const JACKET: &[Cube] = &[grown(
        cube([-4.0, 0.0, -2.0], [8.0, 12.0, 4.0], [16.0, 32.0]),
        PLAYER_LAYER,
    )];
    const LEFT_EAR: &[Cube] = &[cube([0.0, 0.0, -2.0], [1.0, 5.0, 4.0], [51.0, 6.0])];
    const RIGHT_EAR: &[Cube] = &[cube([-1.0, 0.0, -2.0], [1.0, 5.0, 4.0], [39.0, 6.0])];
    if baby_model(l) {
        return baby_piglin(l, zombie);
    }
    let mut nodes = humanoid_parts(l, [HEAD, &[], BODY], PLAYER_WIDE, zombie);
    let angle = 30f32.to_radians();
    let f = l.age * 0.1 + l.walk_pos * 0.5;
    let amp = 0.08 + l.walk_speed * 0.4;
    let (mut left, mut right) = (-angle - (f * 1.2).cos() * amp, angle + f.cos() * amp);
    if l.kind
        == (Kind::Piglin {
            pose: PiglinPose::Dancing,
        })
    {
        let d = l.age / 60.0;
        right = PI / 6.0 + (d * 30.0).sin() * 10f32.to_radians();
        left = -PI / 6.0 - (d * 30.0).cos() * 10f32.to_radians();
    }
    nodes.push(part(Some(2), [0.0; 3], [0.0; 3], JACKET));
    nodes.push(part(Some(0), [4.5, -6.0, 0.0], [0.0, 0.0, left], LEFT_EAR));
    nodes.push(part(
        Some(0),
        [-4.5, -6.0, 0.0],
        [0.0, 0.0, right],
        RIGHT_EAR,
    ));
    nodes
}

/// `BabyPiglinModel` (and `BabyZombifiedPiglinModel`, posed as a zombie): a big head with a snout over a short body and stubby limbs, and
/// ears that hang nearly straight down (5°, `getDefaultEarAngleInDegrees`) on a pivot of
/// their own, flapping as an adult's do.
fn baby_piglin(l: &Living, zombie: Option<bool>) -> Vec<Node> {
    const HEAD: &[Cube] = &[
        cube([-1.5, -3.0, -4.5], [3.0, 3.0, 1.0], [21.0, 30.0]),
        cube([-4.5, -6.0, -3.5], [9.0, 6.0, 7.0], [0.0, 0.0]),
    ];
    const BODY: &[Cube] = &[cube([-3.0, -3.0, -1.0], [6.0, 5.0, 3.0], [0.0, 13.0])];
    const LIMBS: Limbs = Limbs {
        cubes: [
            &[cube([-1.0, 0.0, -1.5], [2.0, 5.0, 3.0], [10.0, 30.0])],
            &[cube([-1.0, 0.0, -1.5], [2.0, 5.0, 3.0], [28.0, 13.0])],
            &[cube([-1.5, 0.0, -1.5], [3.0, 4.0, 3.0], [22.0, 23.0])],
            &[cube([-1.5, 0.0, -1.5], [3.0, 4.0, 3.0], [10.0, 23.0])],
        ],
        leg_x: 1.5,
    };
    const LEFT_EAR: &[Cube] = &[cube([-0.5, -3.0, -2.0], [1.0, 6.0, 4.0], [0.0, 21.0])];
    const RIGHT_EAR: &[Cube] = &[cube([-0.5, -3.0, -2.0], [1.0, 6.0, 4.0], [18.0, 13.0])];
    let mut nodes = humanoid_parts(l, [HEAD, &[], BODY], LIMBS, zombie);
    let angle = 5f32.to_radians();
    let f = l.age * 0.1 + l.walk_pos * 0.5;
    let amp = 0.08 + l.walk_speed * 0.4;
    let (mut left, mut right) = (-angle - (f * 1.2).cos() * amp, angle + f.cos() * amp);
    if l.kind
        == (Kind::Piglin {
            pose: PiglinPose::Dancing,
        })
    {
        let d = l.age / 60.0;
        right = PI / 6.0 + (d * 30.0).sin() * 10f32.to_radians();
        left = -PI / 6.0 - (d * 30.0).cos() * 10f32.to_radians();
    }
    let ears = nodes.len();
    nodes.push(part(Some(0), [4.2, -4.0, 0.0], [0.0, 0.0, left], &[]));
    nodes.push(part(Some(0), [-4.2, -4.0, 0.0], [0.0, 0.0, right], &[]));
    nodes.push(part(
        Some(ears),
        [1.0, 1.75, 0.0],
        [0.0, 0.0, -0.6109],
        LEFT_EAR,
    ));
    nodes.push(part(
        Some(ears + 1),
        [-1.0, 1.75, 0.0],
        [0.0, 0.0, 0.6109],
        RIGHT_EAR,
    ));
    nodes
}

/// `HoglinModel` (128 × 64): its head hangs low and swings up in a headbutt, its ears
/// flap and its legs walk.
fn hoglin(l: &Living, attack: i32) -> Vec<Node> {
    const BODY: &[Cube] = &[cube([-8.0, -7.0, -13.0], [16.0, 14.0, 26.0], [1.0, 1.0])];
    const MANE: &[Cube] = &[grown(
        cube([0.0, 0.0, -9.0], [0.0, 10.0, 19.0], [90.0, 33.0]),
        0.001,
    )];
    const HEAD: &[Cube] = &[cube([-7.0, -3.0, -19.0], [14.0, 6.0, 19.0], [61.0, 1.0])];
    const RIGHT_EAR: &[Cube] = &[cube([-6.0, -1.0, -2.0], [6.0, 1.0, 4.0], [1.0, 1.0])];
    const LEFT_EAR: &[Cube] = &[cube([0.0, -1.0, -2.0], [6.0, 1.0, 4.0], [1.0, 6.0])];
    const RIGHT_HORN: &[Cube] = &[cube([-1.0, -11.0, -1.0], [2.0, 11.0, 2.0], [10.0, 13.0])];
    const LEFT_HORN: &[Cube] = &[cube([-1.0, -11.0, -1.0], [2.0, 11.0, 2.0], [1.0, 13.0])];
    const RIGHT_FRONT: &[Cube] = &[cube([-3.0, 0.0, -3.0], [6.0, 14.0, 6.0], [66.0, 42.0])];
    const LEFT_FRONT: &[Cube] = &[cube([-3.0, 0.0, -3.0], [6.0, 14.0, 6.0], [41.0, 42.0])];
    const RIGHT_HIND: &[Cube] = &[cube([-2.5, 0.0, -2.5], [5.0, 11.0, 5.0], [21.0, 45.0])];
    const LEFT_HIND: &[Cube] = &[cube([-2.5, 0.0, -2.5], [5.0, 11.0, 5.0], [0.0, 45.0])];
    let (p, s) = (l.walk_pos, l.walk_speed);
    let headbutt = 1.0 - (10 - 2 * attack).abs() as f32 / 10.0;
    let head_x = 0.87266463 + headbutt * (-PI / 9.0 - 0.87266463);
    let ear = PI * 2.0 / 9.0 + s * p.sin();
    let (a, b) = (p.cos() * 1.2 * s, (p + PI).cos() * 1.2 * s);
    vec![
        part(None, [0.0, 7.0, 0.0], [0.0; 3], BODY),
        part(Some(0), [0.0, -14.0, -7.0], [0.0; 3], MANE),
        part(
            None,
            [0.0, 2.0, -12.0],
            [head_x, l.head_yaw.to_radians(), 0.0],
            HEAD,
        ),
        part(Some(2), [-6.0, -2.0, -3.0], [0.0, 0.0, -ear], RIGHT_EAR),
        part(Some(2), [6.0, -2.0, -3.0], [0.0, 0.0, ear], LEFT_EAR),
        part(Some(2), [-7.0, 2.0, -12.0], [0.0; 3], RIGHT_HORN),
        part(Some(2), [7.0, 2.0, -12.0], [0.0; 3], LEFT_HORN),
        part(None, [-4.0, 10.0, -8.5], [a, 0.0, 0.0], RIGHT_FRONT),
        part(None, [4.0, 10.0, -8.5], [b, 0.0, 0.0], LEFT_FRONT),
        part(None, [-5.0, 13.0, 10.0], [b, 0.0, 0.0], RIGHT_HIND),
        part(None, [5.0, 13.0, 10.0], [a, 0.0, 0.0], LEFT_HIND),
    ]
}

/// `BabyHoglinModel` (64 × 64), for baby hoglins and zoglins: a long low head with
/// stubby tusks, a short body with its mane on top, and short legs; its headbutt also
/// lowers the head 2.5 pixels (`animateHeadbutt`). Posed as [`hoglin`] is.
fn baby_hoglin(l: &Living, attack: i32) -> Vec<Node> {
    const HEAD: &[Cube] = &[
        cube([-5.0, -2.2605, -10.547], [10.0, 4.0, 12.0], [0.0, 0.0]),
        cube([-7.0, -4.0981, -8.4879], [2.0, 5.0, 2.0], [44.0, 29.0]),
        cube([5.0, -4.0981, -8.4879], [2.0, 5.0, 2.0], [52.0, 29.0]),
    ];
    const BODY: &[Cube] = &[
        grown(
            cube([-4.0, -14.0, -7.0], [8.0, 8.0, 14.0], [0.0, 16.0]),
            0.02,
        ),
        grown(
            cube([0.0, -18.0, -8.0], [0.0, 6.0, 11.0], [24.0, 39.0]),
            0.02,
        ),
    ];
    const RIGHT_EAR: &[Cube] = &[cube([-5.1, -0.5, -2.0], [6.0, 1.0, 4.0], [32.0, 5.0])];
    const LEFT_EAR: &[Cube] = &[mirrored(cube(
        [-0.9, -0.5, -2.0],
        [6.0, 1.0, 4.0],
        [32.0, 0.0],
    ))];
    const fn leg(u: f32, v: f32) -> [Cube; 1] {
        [cube([-1.5, 0.0, -1.5], [3.0, 6.0, 3.0], [u, v])]
    }
    const LEGS: [[Cube; 1]; 4] = [
        leg(0.0, 38.0),
        leg(12.0, 38.0),
        leg(0.0, 47.0),
        leg(12.0, 47.0),
    ];
    let (p, s) = (l.walk_pos, l.walk_speed);
    let headbutt = 1.0 - (10 - 2 * attack).abs() as f32 / 10.0;
    let head_x = 0.87266463 + headbutt * (-PI / 9.0 - 0.87266463);
    let ear = PI * 2.0 / 9.0 + s * p.sin();
    let (a, b) = (p.cos() * 1.2 * s, (p + PI).cos() * 1.2 * s);
    vec![
        part(
            None,
            [0.0, 13.0 + headbutt * 2.5, -7.0],
            [head_x, l.head_yaw.to_radians(), 0.0],
            HEAD,
        ),
        part(None, [0.0, 24.0, 0.0], [0.0; 3], BODY),
        part(Some(0), [-5.0, -1.0, -1.5], [0.0, 0.0, -ear], RIGHT_EAR),
        part(Some(0), [5.0, -1.0, -1.5], [0.0, 0.0, ear], LEFT_EAR),
        part(None, [-2.5, 18.0, -4.5], [a, 0.0, 0.0], &LEGS[0]),
        part(None, [2.5, 18.0, -4.5], [b, 0.0, 0.0], &LEGS[1]),
        part(None, [-2.5, 18.0, 4.5], [b, 0.0, 0.0], &LEGS[2]),
        part(None, [2.5, 18.0, 4.5], [a, 0.0, 0.0], &LEGS[3]),
    ]
}

/// `BabyStriderModel` (32 × 32): a small body on two short legs, looking where its head
/// does, with three bristles across its top, front to back, that ripple forward and back
/// (`StriderModel.animateBristle` on their `xRot`). Bobs and sways as the adult does.
fn baby_strider(l: &Living) -> Vec<Node> {
    const BODY: &[Cube] = &[cube([-3.5, -3.75, -4.0], [7.0, 7.0, 8.0], [0.0, 0.0])];
    const RIGHT_LEG: &[Cube] = &[cube([-1.0, 0.0, -1.0], [2.0, 4.0, 2.0], [0.0, 24.0])];
    const LEFT_LEG: &[Cube] = &[cube([-1.0, 0.0, -1.0], [2.0, 4.0, 2.0], [8.0, 24.0])];
    const fn bristle(v: f32) -> [Cube; 1] {
        [cube([-3.5, -2.5, 0.0], [7.0, 3.0, 0.0], [0.0, v])]
    }
    // Front, middle, back (`bristle2`, `bristle1`, `bristle0`).
    const BRISTLES: [[Cube; 1]; 3] = [bristle(15.0), bristle(18.0), bristle(21.0)];
    let (p, s, t) = (l.walk_pos, l.walk_speed.min(0.25), l.age);
    let q = p * 1.5 * 0.5;
    let flow = (p * 1.5 + PI).cos() * s;
    let ripple = [
        flow * 0.6 + 0.1 * (t * 0.4).sin(),
        flow * 1.2 + 0.1 * (t * 0.2).sin(),
        flow * 1.3 + 0.05 * (t * -0.4).sin(),
    ];
    let mut nodes = vec![
        part(
            None,
            [0.0, 17.25 - (p * 1.5).cos() * 2.0 * s, 0.0],
            [
                l.pitch.to_radians(),
                l.head_yaw.to_radians(),
                0.1 * (p * 1.5).sin() * 4.0 * s,
            ],
            BODY,
        ),
        part(
            None,
            [-1.5, 20.0 + 2.0 * q.sin() * 2.0 * s, 0.0],
            [
                (q + PI).sin() * 2.0 * s,
                0.0,
                PI / 18.0 * (q + PI).cos() * s,
            ],
            RIGHT_LEG,
        ),
        part(
            None,
            [1.5, 20.0 + 2.0 * (q + PI).sin() * 2.0 * s, 0.0],
            [q.sin() * 2.0 * s, 0.0, PI / 18.0 * q.cos() * s],
            LEFT_LEG,
        ),
    ];
    for (i, z) in [-2.0, 0.0, 2.0].into_iter().enumerate() {
        nodes.push(part(
            Some(0),
            [0.0, -4.25, z],
            [ripple[i], 0.0, 0.0],
            &BRISTLES[i],
        ));
    }
    nodes
}

/// `AdultStriderModel` (64 × 128): a body on two long legs that bob and sway as it
/// walks, looking where its head does, with three flat bristles a side that ripple
/// (`StriderModel.animateBristle`).
fn strider(l: &Living) -> Vec<Node> {
    const RIGHT_LEG: &[Cube] = &[cube([-2.0, 0.0, -2.0], [4.0, 16.0, 4.0], [0.0, 32.0])];
    const LEFT_LEG: &[Cube] = &[cube([-2.0, 0.0, -2.0], [4.0, 16.0, 4.0], [0.0, 55.0])];
    const BODY: &[Cube] = &[cube([-8.0, -6.0, -8.0], [16.0; 3], [0.0, 0.0])];
    const fn bristle(left: bool, v: f32) -> [Cube; 1] {
        if left {
            [cube([0.0; 3], [12.0, 0.0, 16.0], [16.0, v])]
        } else {
            [mirrored(cube(
                [-12.0, 0.0, 0.0],
                [12.0, 0.0, 16.0],
                [16.0, v],
            ))]
        }
    }
    const RIGHT: [[Cube; 1]; 3] = [
        bristle(false, 33.0),
        bristle(false, 49.0),
        bristle(false, 65.0),
    ];
    const LEFT: [[Cube; 1]; 3] = [
        bristle(true, 33.0),
        bristle(true, 49.0),
        bristle(true, 65.0),
    ];
    let (p, s, t) = (l.walk_pos, l.walk_speed.min(0.25), l.age);
    let q = p * 1.5 * 0.5;
    let flow = (p * 1.5 + PI).cos() * s;
    // Top, middle, bottom: how far each ripples with the walk, and its idle wave.
    let ripple = [
        flow * 0.6 + 0.1 * (t * 0.4).sin(),
        flow * 1.2 + 0.1 * (t * 0.2).sin(),
        flow * 1.3 + 0.05 * (t * -0.4).sin(),
    ];
    let body = [
        l.pitch.to_radians(),
        l.head_yaw.to_radians(),
        0.1 * (p * 1.5).sin() * 4.0 * s,
    ];
    let mut nodes = vec![
        part(
            None,
            [-4.0, 8.0 + 2.0 * q.sin() * 2.0 * s, 0.0],
            [
                (q + PI).sin() * 2.0 * s,
                0.0,
                PI / 18.0 * (q + PI).cos() * s,
            ],
            RIGHT_LEG,
        ),
        part(
            None,
            [4.0, 8.0 + 2.0 * (q + PI).sin() * 2.0 * s, 0.0],
            [q.sin() * 2.0 * s, 0.0, PI / 18.0 * q.cos() * s],
            LEFT_LEG,
        ),
        part(
            None,
            [0.0, 2.0 - 2.0 * (p * 1.5).cos() * 2.0 * s, 0.0],
            body,
            BODY,
        ),
    ];
    // Top, middle, bottom: where each sits and its rest angle.
    let right = [(-5.0, -0.87266463), (-1.0, -1.134464), (4.0, -1.2217305)];
    let left = [(-6.0, 0.87266463), (-2.0, 1.134464), (3.0, 1.2217305)];
    for i in 0..3 {
        let (y, z) = right[i];
        nodes.push(part(
            Some(2),
            [-8.0, y, -8.0],
            [0.0, 0.0, z + ripple[i]],
            &RIGHT[i],
        ));
        let (y, z) = left[i];
        nodes.push(part(
            Some(2),
            [8.0, y, -8.0],
            [0.0, 0.0, z + ripple[i]],
            &LEFT[i],
        ));
    }
    nodes
}

/// `AdultWolfModel` posed by `WolfModel.setupAnim`: diagonal legs walk together, the tail
/// wags with the walk (not while angry) at the height `getTailAngle` gives, and a sitting
/// wolf (`setSittingPose`) folds its hind legs under it.
fn wolf(l: &Living, angry: bool, sitting: bool, tail: f32) -> Vec<Node> {
    const HEAD: &[Cube] = &[
        cube([-2.0, -3.0, -2.0], [6.0, 6.0, 4.0], [0.0, 0.0]),
        cube([-2.0, -5.0, 0.0], [2.0, 2.0, 1.0], [16.0, 14.0]),
        cube([2.0, -5.0, 0.0], [2.0, 2.0, 1.0], [16.0, 14.0]),
        cube([-0.5, -0.001, -5.0], [3.0, 3.0, 4.0], [0.0, 10.0]),
    ];
    const BODY: &[Cube] = &[cube([-3.0, -2.0, -3.0], [6.0, 9.0, 6.0], [18.0, 14.0])];
    const UPPER: &[Cube] = &[cube([-3.0; 3], [8.0, 6.0, 7.0], [21.0, 0.0])];
    const LEFT: &[Cube] = &[cube([0.0, 0.0, -1.0], [2.0, 8.0, 2.0], [0.0, 18.0])];
    const RIGHT: &[Cube] = &[mirrored(LEFT[0])];
    const TAIL: &[Cube] = &[cube([0.0, 0.0, -1.0], [2.0, 8.0, 2.0], [9.0, 18.0])];
    let (p, s) = (l.walk_pos * 0.6662, l.walk_speed);
    let (a, b) = (p.cos() * 1.4 * s, (p + PI).cos() * 1.4 * s);
    let wag = if angry { 0.0 } else { a };
    let look = [l.pitch.to_radians(), l.head_yaw.to_radians(), 0.0];
    let mut body = ([0.0, 14.0, 2.0], PI / 2.0);
    let mut upper = ([-1.0, 14.0, -3.0], PI / 2.0);
    let mut tail_at = [-1.0, 12.0, 8.0];
    let mut legs = [
        ([-2.5, 16.0, 7.0], a, RIGHT),
        ([0.5, 16.0, 7.0], b, LEFT),
        ([-2.5, 16.0, -4.0], b, RIGHT),
        ([0.5, 16.0, -4.0], a, LEFT),
    ];
    if sitting {
        body = ([0.0, 18.0, 0.0], PI / 4.0);
        upper = ([-1.0, 16.0, -3.0], PI * 2.0 / 5.0);
        tail_at = [-1.0, 21.0, 6.0];
        for leg in &mut legs[..2] {
            leg.0[1] += 6.7;
            leg.0[2] -= 5.0;
            leg.1 = PI * 3.0 / 2.0;
        }
        for (leg, dx) in legs[2..].iter_mut().zip([0.01, -0.01]) {
            leg.0[0] += dx;
            leg.0[1] += 1.0;
            leg.1 = 5.811947;
        }
    }
    let mut nodes = vec![
        part(None, [-1.0, 13.5, -7.0], look, &[]),
        part(Some(0), [0.0; 3], [0.0; 3], HEAD),
        part(None, body.0, [body.1, 0.0, 0.0], BODY),
        part(None, upper.0, [upper.1, 0.0, 0.0], UPPER),
    ];
    for (at, swing, cubes) in legs {
        nodes.push(part(None, at, [swing, 0.0, 0.0], cubes));
    }
    nodes.push(part(None, tail_at, [tail, wag, 0.0], &[]));
    nodes.push(part(Some(8), [0.0; 3], [0.0; 3], TAIL));
    nodes
}

/// `DrownedModel.createBodyLayer`: a humanoid whose left arm and leg have their own
/// texture (64 × 64).
const DROWNED_LIMBS: Limbs = Limbs {
    cubes: [
        &[cube([-3.0, -2.0, -2.0], [4.0, 12.0, 4.0], [40.0, 16.0])],
        &[cube([-1.0, -2.0, -2.0], [4.0, 12.0, 4.0], [32.0, 48.0])],
        &[cube([-2.0, 0.0, -2.0], [4.0, 12.0, 4.0], [0.0, 16.0])],
        &[cube([-2.0, 0.0, -2.0], [4.0, 12.0, 4.0], [16.0, 48.0])],
    ],
    leg_x: 1.9,
};
/// The same, a quarter pixel out (`DROWNED_OUTER_LAYER`).
const DROWNED_OUTER_PARTS: [&[Cube]; 3] = [
    &[grown(cube([-4.0, -8.0, -4.0], [8.0; 3], [0.0, 0.0]), 0.25)],
    &[grown(cube([-4.0, -8.0, -4.0], [8.0; 3], [32.0, 0.0]), 0.75)],
    &[grown(
        cube([-4.0, 0.0, -2.0], [8.0, 12.0, 4.0], [16.0, 16.0]),
        0.25,
    )],
];
const DROWNED_OUTER_LIMBS: Limbs = Limbs {
    cubes: [
        &[grown(DROWNED_LIMBS.cubes[0][0], 0.25)],
        &[grown(DROWNED_LIMBS.cubes[1][0], 0.25)],
        &[grown(DROWNED_LIMBS.cubes[2][0], 0.25)],
        &[grown(DROWNED_LIMBS.cubes[3][0], 0.25)],
    ],
    leg_x: 1.9,
};

/// `STRAY_OUTER_LAYER`: `HumanoidModel.createMesh` a quarter pixel out (64 × 32), posed as
/// the skeleton is, legs two pixels out.
const STRAY_CLOTHES_PARTS: [&[Cube]; 3] = DROWNED_OUTER_PARTS;
const STRAY_CLOTHES_LIMBS: Limbs = Limbs {
    cubes: [
        &[grown(HUMANOID.cubes[0][0], 0.25)],
        &[grown(HUMANOID.cubes[1][0], 0.25)],
        &[grown(HUMANOID.cubes[2][0], 0.25)],
        &[grown(HUMANOID.cubes[3][0], 0.25)],
    ],
    leg_x: 2.0,
};

/// `ItemEntityRenderer.submit` for one item: bobbing above the ground, its lowest point a
/// pixel up, spinning, in the model's `ground` transform. Flat items are their sprite's
/// front and back, a pixel apart, as `item/generated` (whose `ground` they all share)
/// places them; vanilla also fills in the pixel-thick edges.
/// Corners, uv and colour of one of an item's quads, in the item's own 0..1 space.
type ItemQuad = ([Vec3; 4], [[f32; 2]; 4], [f32; 3]);

/// `ItemModelGenerator`: a sprite as a card from z 7.5 to 8.5 pixels, its front and back
/// the whole sprite, and a side face along each edge between an opaque pixel and a clear
/// one (or the sprite's edge), textured with that pixel. Sides facing left and right are
/// shaded as east and west.
fn extrude(atlas: &Atlas, rect: [f32; 4], tint: [f32; 3], lift: f32, out: &mut Vec<ItemQuad>) {
    let [u0, v0, u1, v1] = rect;
    let (front, back) = (8.5 / 16.0 + lift, 7.5 / 16.0 - lift);
    let v = Vec3::new;
    out.push((
        [
            v(0.0, 0.0, front),
            v(1.0, 0.0, front),
            v(1.0, 1.0, front),
            v(0.0, 1.0, front),
        ],
        [[u0, v1], [u1, v1], [u1, v0], [u0, v0]],
        tint,
    ));
    out.push((
        [
            v(1.0, 0.0, back),
            v(0.0, 0.0, back),
            v(0.0, 1.0, back),
            v(1.0, 1.0, back),
        ],
        [[u1, v1], [u0, v1], [u0, v0], [u1, v0]],
        tint,
    ));
    let sprite = crate::atlas::Sprite {
        rect,
        layer: crate::atlas::Layer::Cutout,
    };
    let [w, h] = atlas.sprite_size(sprite);
    let clear = |x: i32, y: i32| {
        x < 0
            || y < 0
            || x >= w as i32
            || y >= h as i32
            || atlas.alpha(sprite, x as u32, y as u32) == 0
    };
    let (sx, sy) = (1.0 / w as f32, 1.0 / h as f32);
    for y in 0..h as i32 {
        for x in 0..w as i32 {
            if clear(x, y) {
                continue;
            }
            // The pixel's middle, shrunk by a tenth of a pixel (`UV_SHRINK`).
            let pu = |f: f32| u0 + (u1 - u0) * (x as f32 + f) * sx;
            let pv = |f: f32| v0 + (v1 - v0) * (y as f32 + f) * sy;
            let uv = [
                [pu(0.1), pv(0.1)],
                [pu(0.9), pv(0.1)],
                [pu(0.9), pv(0.9)],
                [pu(0.1), pv(0.9)],
            ];
            // Left, right, top and bottom of the pixel in item space, y up.
            let (l, r) = (x as f32 * sx, (x + 1) as f32 * sx);
            let (t, b) = (1.0 - y as f32 * sy, 1.0 - (y + 1) as f32 * sy);
            // (neighbour that must be clear, the edge's two ends, the face's normal)
            let sides = [
                ((x, y - 1), [v(l, t, 0.0), v(r, t, 0.0)], Vec3::Y),
                ((x, y + 1), [v(l, b, 0.0), v(r, b, 0.0)], Vec3::NEG_Y),
                ((x - 1, y), [v(l, t, 0.0), v(l, b, 0.0)], Vec3::X),
                ((x + 1, y), [v(r, t, 0.0), v(r, b, 0.0)], Vec3::NEG_X),
            ];
            for ((nx, ny), [a, c], normal) in sides {
                if !clear(nx, ny) {
                    continue;
                }
                let (z0, z1) = (Vec3::Z * (7.5 / 16.0), Vec3::Z * (8.5 / 16.0));
                let mut pos = [a + z0, a + z1, c + z1, c + z0];
                if (pos[1] - pos[0]).cross(pos[2] - pos[0]).dot(normal) < 0.0 {
                    pos.swap(1, 3);
                }
                out.push((pos, uv, tint));
            }
        }
    }
}

/// An item's quads: a flat item's sprite layers extruded a pixel thick, as
/// `ItemModelGenerator.bakeExtrudedSprite` does, or its model's.
fn item_quads(atlas: &Atlas, icon: &ItemIcon) -> Vec<ItemQuad> {
    let mut quads = Vec::new();
    match icon {
        ItemIcon::Flat { layers, .. } => {
            for (i, &(rect, tint)) in layers.iter().enumerate() {
                // Later layers sit a hair out, as the GUI stacks them, so they don't fight.
                let lift = i as f32 * 0.001;
                extrude(atlas, rect, tint, lift, &mut quads);
            }
        }
        ItemIcon::Model { model, tints, .. } => {
            for q in &model.quads {
                let tint = usize::try_from(q.tint)
                    .ok()
                    .and_then(|i| tints.get(i).copied())
                    .unwrap_or([1.0; 3]);
                quads.push((q.pos, q.uv, tint));
            }
        }
    }
    quads
}

/// The item's `ground` transform (`ItemTransform.apply`), then the model centred: a flat
/// item's is `item/generated`'s.
fn ground_transform(icon: &ItemIcon) -> Mat4 {
    let (translation, rotation, scale) = match icon {
        ItemIcon::Flat { .. } => (Vec3::new(0.0, 2.0, 0.0), Quat::IDENTITY, Vec3::splat(0.5)),
        ItemIcon::Model { ground, .. } => {
            let [rx, ry, rz] = ground.rotation.map(f32::to_radians);
            (
                Vec3::from(ground.translation),
                Quat::from_euler(glam::EulerRot::XYZ, rx, ry, rz),
                Vec3::from(ground.scale),
            )
        }
    };
    Mat4::from_translation(translation / 16.0)
        * Mat4::from_quat(rotation)
        * Mat4::from_scale(scale)
        * Mat4::from_translation(Vec3::splat(-0.5))
}

/// `ArrowRenderer.submit`: `ArrowModel` turned to fly along `yaw` and climb at `pitch`,
/// quivering about its tip's axis as it sticks (`setupAnim`). Its planes are drawn only
/// from the side facing the camera (`RenderTypes.entityCutoutCull`).
fn arrow(
    atlas: &Atlas,
    a: &Arrow,
    camera: &crate::gpu::Camera,
    light: [u8; 2],
    out: &mut Vec<Vertex>,
) {
    // `ArrowModel.createBodyLayer`: two crossed planes 16 pixels long and the fletching's
    // square at the back, the whole mesh scaled by 0.9 (`mesh.transformed`); the planes'
    // texture is 0.8 as tall (`addBox`'s texture scale).
    const BACK: &[Cube] = &[cube([0.0, -2.5, -2.5], [0.0, 5.0, 5.0], [0.0, 0.0])];
    const CROSS: &[Cube] = &[cube([-12.0, -2.0, 0.0], [16.0, 4.0, 0.0], [0.0, 0.0])];
    let shake = if a.shake > 0.0 {
        -(a.shake * 3.0).sin() * a.shake
    } else {
        0.0
    };
    let root = Mat4::from_rotation_y((a.yaw - 90.0).to_radians())
        * Mat4::from_rotation_z(a.pitch.to_radians())
        * Mat4::from_rotation_z(shake.to_radians())
        * Mat4::from_scale(Vec3::splat(0.9));
    let sprite = atlas.sprite(a.texture);
    let node = |cubes| Node {
        parent: None,
        pose: [0.0; 6],
        scale: [1.0; 3],
        cubes,
    };
    let mut back = Builder {
        quads: Vec::new(),
        sprite,
        tex_size: [32.0, 32.0],
        tint: -1,
    };
    back.nodes(
        root * Mat4::from_translation(Vec3::new(-11.0 / 16.0, 0.0, 0.0))
            * Mat4::from_rotation_x(PI / 4.0)
            * Mat4::from_scale(Vec3::splat(0.8)),
        &[node(BACK)],
    );
    let mut cross = Builder {
        quads: Vec::new(),
        sprite,
        tex_size: [32.0, 32.0 * 0.8],
        tint: -1,
    };
    for turn in [PI / 4.0, PI * 3.0 / 4.0] {
        cross.nodes(root * Mat4::from_rotation_x(turn), &[node(CROSS)]);
    }
    let to_camera = camera.eye - a.pos;
    let facing: Vec<Quad> = back
        .quads
        .into_iter()
        .chain(cross.quads)
        .filter(|q| {
            let normal = (q.pos[1] - q.pos[0]).cross(q.pos[2] - q.pos[0]);
            normal.dot(to_camera - q.pos[0]) > 0.0
        })
        .collect();
    emit(&facing, a.pos, [1.0; 3], light, out);
}

fn dropped(atlas: &Atlas, icon: &ItemIcon, d: &Dropped, light: [u8; 2], out: &mut Vec<Vertex>) {
    let quads = item_quads(atlas, icon);
    let transform = ground_transform(icon);
    let min_y = quads
        .iter()
        .flat_map(|(corners, _, _)| corners.iter().map(|&c| transform.transform_point3(c).y))
        .fold(f32::MAX, f32::min);
    if min_y == f32::MAX {
        return;
    }
    let bob = (d.age / 10.0 + d.bob_offset).sin() * 0.1 + 0.1;
    let spin = d.age / 20.0 + d.bob_offset;
    let root = Mat4::from_translation(Vec3::new(0.0, bob - min_y + 1.0 / 16.0, 0.0))
        * Mat4::from_rotation_y(spin)
        * transform;
    for (corners, uv, tint) in quads {
        let pos = corners.map(|c| root.transform_point3(c));
        emit_quad(pos, uv, d.pos, tint, light, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn standing(kind: Kind) -> Living {
        Living {
            kind,
            texture: String::new(),
            layers: Vec::new(),
            pos: Vec3::ZERO,
            body_yaw: 0.0,
            head_yaw: 0.0,
            pitch: 0.0,
            walk_pos: 0.0,
            walk_speed: 0.0,
            age: 0.0,
            flap: 0.0,
            flap_speed: 0.0,
            baby: false,
            name_tag: None,
            equipment: Equipment::default(),
            swing: None,
            burning: None,
            hurt: false,
            death_time: 0.0,
        }
    }

    /// The corners of a model's quads, placed as the renderer places them.
    fn corners(l: &Living, nodes: &[Node], tex_size: [f32; 2], model_scale: f32) -> Vec<Vec3> {
        let atlas_sprite = crate::atlas::Sprite {
            rect: [0.0, 0.0, 1.0, 1.0],
            layer: crate::atlas::Layer::Cutout,
        };
        let mut b = Builder {
            quads: Vec::new(),
            sprite: atlas_sprite,
            tex_size,
            tint: -1,
        };
        b.nodes(living_root(l, model_scale), nodes);
        b.quads.iter().flat_map(|q| q.pos).collect()
    }

    /// Standing models stand where vanilla's do: `LivingEntityRenderer` lowers the model
    /// 1.501 blocks after flipping it, so the 24-pixel-high feet sit 0.001 above the ground
    /// and a part 8.5 pixels above the origin (a hat's top) is at 1.501 + 8.5/16, scaled by
    /// 15/16 for players. A mob turned to yaw 0 faces south, +z.
    #[test]
    fn models_stand_where_vanillas_do() {
        let extent = |points: &[Vec3]| {
            let min = points.iter().fold(Vec3::MAX, |a, &b| a.min(b));
            let max = points.iter().fold(Vec3::MIN, |a, &b| a.max(b));
            (min, max)
        };
        let close = |a: f32, b: f32| (a - b).abs() < 1e-4;
        // Humanoid legs splay by 0.005 radians, which lowers their corners a little.
        let feet = |a: f32| (a - 0.001).abs() < 1e-3;
        let zombie = standing(Kind::Zombie { aggressive: false });
        let nodes = humanoid(&zombie, HUMANOID, Some(false));
        let (min, max) = extent(&corners(&zombie, &nodes, [64.0; 2], 1.0));
        assert!(
            feet(min.y) && close(max.y, 1.501 + 8.5 / 16.0),
            "{min} {max}"
        );

        let steve = standing(Kind::Player {
            slim: false,
            crouching: false,
        });
        let (min, max) = extent(&corners(&steve, &player(&steve, false), [64.0; 2], 0.9375));
        // The trousers layer reaches a quarter pixel below the feet.
        assert!(
            (min.y - (1.501 - 24.25 / 16.0) * 0.9375).abs() < 1e-3,
            "{min}"
        );
        assert!(close(max.y, (1.501 + 8.5 / 16.0) * 0.9375), "{max}");
        // Crouching drops the head 4.2 pixels (`HumanoidModel.setupAnim`).
        let crouching = standing(Kind::Player {
            slim: false,
            crouching: true,
        });
        let (_, max) = extent(&corners(
            &crouching,
            &player(&crouching, false),
            [64.0; 2],
            0.9375,
        ));
        assert!(close(max.y, (1.501 + 4.3 / 16.0) * 0.9375), "{max}");

        // A cow's horns top it, a pixel above its head at y 4 - 5 pixels.
        let cow = standing(Kind::Cow);
        let nodes = quadruped(&cow, &COW);
        let (min, max) = extent(&corners(&cow, &nodes, [64.0; 2], 1.0));
        assert!(
            close(min.y, 0.001) && close(max.y, 1.501 + 1.0 / 16.0),
            "{min} {max}"
        );
        // Its head, 8 to 15 pixels in front of the origin, is towards +z.
        let (head_min, head_max) = extent(&corners(&cow, &nodes[..1], [64.0; 2], 1.0));
        assert!(
            close(head_min.z, 0.5) && close(head_max.z, 15.0 / 16.0),
            "{head_min} {head_max}"
        );
    }

    /// A zombie's flames (0.6 by 1.95) are six cards, 0.84 blocks to a side, a 0.45-block
    /// step (scaled) apart, each 0.9 the width of the one below; seen from a camera looking
    /// south, they face it, front out, and the first stands 0.3 (less 0.02 per whole card
    /// height) towards it.
    #[test]
    fn flames_stack_up_facing_the_camera() {
        let fire = [[0.0, 0.0, 0.5, 1.0], [0.5, 0.0, 1.0, 1.0]];
        let mut out = Vec::new();
        flames(fire, Vec3::ZERO, [0.6, 1.95], 0.0, [15, 15], &mut out);
        let quads: Vec<&[Vertex]> = out.chunks(4).collect();
        assert_eq!(quads.len(), 6);
        let s = 0.6 * 1.4;
        for (i, q) in quads.iter().enumerate() {
            let p = q.iter().map(|v| Vec3::from(v.pos)).collect::<Vec<_>>();
            // The camera looks along +z, so a card facing it faces -z.
            let normal = (p[1] - p[0]).cross(p[2] - p[0]).normalize();
            assert!((normal - Vec3::NEG_Z).length() < 1e-5, "{i}: {normal}");
            let width = (p[1] - p[0]).length();
            assert!(
                (width - s * 0.9f32.powi(i as i32)).abs() < 1e-5,
                "{i}: {width}"
            );
            assert!(
                (p[0].y - s * 0.45 * i as f32).abs() < 1e-5,
                "{i}: {}",
                p[0].y
            );
            assert!((p[2].y - p[0].y - s * 1.4).abs() < 1e-5);
            let z = -(0.3 - 2.0 * 0.02 - 0.03 * i as f32) * s;
            assert!((p[0].z - z).abs() < 1e-5, "{i}: {} vs {z}", p[0].z);
            // `fire_0` then `fire_1` mirrored, then the pair unmirrored, and so on.
            let u = [q[0].uv[0], q[1].uv[0]];
            let expected = [[0.0, 0.5], [0.5, 1.0], [0.5, 0.0], [1.0, 0.5]][i % 4];
            assert_eq!(u, expected, "{i}");
        }
        assert!(quads.iter().all(|q| q.iter().all(|v| v.color == [255; 4])));
    }

    /// A dying mob tips over sideways, √((t − 1) / 20 × 1.6) of the way at t ticks, so it's
    /// 56.9° over at 6 and flat from 13.5; a spider rolls on to its back instead.
    #[test]
    fn dying_mobs_fall_over() {
        let up = |kind, death_time| {
            let l = Living {
                death_time,
                ..standing(kind)
            };
            // Model space is y down: its up is -y.
            let up = living_root(&l, 1.0)
                .transform_vector3(Vec3::NEG_Y)
                .normalize();
            (up.angle_between(Vec3::Y).to_degrees(), up)
        };
        let pig = Kind::Pig;
        assert!(up(pig, 0.0).0 < 1e-3);
        let expected = (5.0f32 / 20.0 * 1.6).sqrt() * 90.0;
        assert!(
            (up(pig, 6.0).0 - expected).abs() < 1e-3,
            "{:?}",
            up(pig, 6.0)
        );
        let (angle, flat) = up(pig, 13.5);
        assert!((angle - 90.0).abs() < 1e-3 && flat.x > 0.99, "{flat}");
        assert!((up(pig, 20.0).0 - 90.0).abs() < 1e-3);
        assert!((up(Kind::Spider, 20.0).0 - 180.0).abs() < 1e-3);
    }

    /// Wool colours as `ColorLerper` makes them: white is E6E6E6, others are their dye's
    /// colour at three quarters.
    #[test]
    fn wool_colours_match_vanilla() {
        assert_eq!(wool_color(0), [230.0 / 255.0; 3]);
        // Red, B02E26: 0xB0 * 0.75 = 132, 0x2E * 0.75 = 34.5 floored, 0x26 * 0.75 = 28.5.
        assert_eq!(wool_color(14), [132.0 / 255.0, 34.0 / 255.0, 28.0 / 255.0]);
    }
}
