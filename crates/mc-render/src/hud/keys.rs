//! Vanilla's key mappings (`Options.keyMappings`): each mapping's name, default key,
//! category and order, the keys as `InputConstants` names and shows them, and the order
//! the Key Binds screen lists them in (`KeyMapping.compareTo`).

use std::collections::HashMap;

/// `InputConstants.Key`: a keyboard key by its USB HID scancode (0 being none), or a mouse
/// button by its number (left 1, middle 2, right 3).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Key {
    Keyboard(i32),
    Mouse(i32),
}

/// `InputConstants.UNKNOWN`: not bound.
pub const UNBOUND: Key = Key::Keyboard(0);

/// `InputConstants.Type`'s `addKey` names for the keyboard, by scancode.
const KEYBOARD: &[(i32, &str)] = &[
    (0, "unknown"),
    (4, "a"),
    (5, "b"),
    (6, "c"),
    (7, "d"),
    (8, "e"),
    (9, "f"),
    (10, "g"),
    (11, "h"),
    (12, "i"),
    (13, "j"),
    (14, "k"),
    (15, "l"),
    (16, "m"),
    (17, "n"),
    (18, "o"),
    (19, "p"),
    (20, "q"),
    (21, "r"),
    (22, "s"),
    (23, "t"),
    (24, "u"),
    (25, "v"),
    (26, "w"),
    (27, "x"),
    (28, "y"),
    (29, "z"),
    (30, "1"),
    (31, "2"),
    (32, "3"),
    (33, "4"),
    (34, "5"),
    (35, "6"),
    (36, "7"),
    (37, "8"),
    (38, "9"),
    (39, "0"),
    (40, "enter"),
    (41, "escape"),
    (42, "backspace"),
    (43, "tab"),
    (44, "space"),
    (45, "minus"),
    (46, "equal"),
    (47, "left.bracket"),
    (48, "right.bracket"),
    (49, "backslash"),
    (50, "world.2"),
    (51, "semicolon"),
    (52, "apostrophe"),
    (53, "grave.accent"),
    (54, "comma"),
    (55, "period"),
    (56, "slash"),
    (57, "caps.lock"),
    (58, "f1"),
    (59, "f2"),
    (60, "f3"),
    (61, "f4"),
    (62, "f5"),
    (63, "f6"),
    (64, "f7"),
    (65, "f8"),
    (66, "f9"),
    (67, "f10"),
    (68, "f11"),
    (69, "f12"),
    (70, "print.screen"),
    (71, "scroll.lock"),
    (72, "pause"),
    (73, "insert"),
    (74, "home"),
    (75, "page.up"),
    (76, "delete"),
    (77, "end"),
    (78, "page.down"),
    (79, "right"),
    (80, "left"),
    (81, "down"),
    (82, "up"),
    (83, "num.lock"),
    (84, "keypad.divide"),
    (85, "keypad.multiply"),
    (86, "keypad.subtract"),
    (87, "keypad.add"),
    (88, "keypad.enter"),
    (89, "keypad.1"),
    (90, "keypad.2"),
    (91, "keypad.3"),
    (92, "keypad.4"),
    (93, "keypad.5"),
    (94, "keypad.6"),
    (95, "keypad.7"),
    (96, "keypad.8"),
    (97, "keypad.9"),
    (98, "keypad.0"),
    (99, "keypad.period"),
    (100, "world.1"),
    (101, "application"),
    (102, "power"),
    (103, "keypad.equal"),
    (104, "f13"),
    (105, "f14"),
    (106, "f15"),
    (107, "f16"),
    (108, "f17"),
    (109, "f18"),
    (110, "f19"),
    (111, "f20"),
    (112, "f21"),
    (113, "f22"),
    (114, "f23"),
    (115, "f24"),
    (116, "execute"),
    (117, "help"),
    (118, "menu"),
    (119, "select"),
    (120, "stop"),
    (121, "again"),
    (122, "undo"),
    (123, "cut"),
    (124, "copy"),
    (125, "paste"),
    (126, "find"),
    (127, "mute"),
    (128, "volume.up"),
    (129, "volume.down"),
    (133, "keypad.comma"),
    (134, "keypad.equals.as400"),
    (135, "international1"),
    (136, "international2"),
    (137, "international3"),
    (138, "international4"),
    (139, "international5"),
    (140, "international6"),
    (141, "international7"),
    (142, "international8"),
    (143, "international9"),
    (144, "lang1"),
    (145, "lang2"),
    (146, "lang3"),
    (147, "lang4"),
    (148, "lang5"),
    (149, "lang6"),
    (150, "lang7"),
    (151, "lang8"),
    (152, "lang9"),
    (153, "alternate.erase"),
    (154, "sys.req"),
    (155, "cancel"),
    (156, "clear"),
    (157, "prior"),
    (158, "enter2"),
    (159, "separator"),
    (160, "out"),
    (161, "oper"),
    (162, "clear.again"),
    (163, "crsel"),
    (164, "exsel"),
    (176, "keypad.00"),
    (177, "keypad.000"),
    (178, "thousands.separator"),
    (179, "decimal.separator"),
    (180, "currency.unit"),
    (181, "currency.subunit"),
    (182, "keypad.left.parenthesis"),
    (183, "keypad.right.parenthesis"),
    (184, "keypad.left.brace"),
    (185, "keypad.right.brace"),
    (186, "keypad.tab"),
    (187, "keypad.backspace"),
    (188, "keypad.a"),
    (189, "keypad.b"),
    (190, "keypad.c"),
    (191, "keypad.d"),
    (192, "keypad.e"),
    (193, "keypad.f"),
    (194, "keypad.xor"),
    (195, "keypad.power"),
    (196, "keypad.percent"),
    (197, "keypad.less"),
    (198, "keypad.greater"),
    (199, "keypad.ampersand"),
    (200, "keypad.double.ampersand"),
    (201, "keypad.vertical.bar"),
    (202, "keypad.double.vertical.bar"),
    (203, "keypad.colon"),
    (204, "keypad.hash"),
    (205, "keypad.space"),
    (206, "keypad.at"),
    (207, "keypad.exclamation"),
    (208, "keypad.memory.store"),
    (209, "keypad.memory.recall"),
    (210, "keypad.memory.clear"),
    (211, "keypad.memory.add"),
    (212, "keypad.memory.subtract"),
    (213, "keypad.memory.multiply"),
    (214, "keypad.memory.divide"),
    (215, "keypad.plus.minus"),
    (216, "keypad.clear"),
    (217, "keypad.clear.entry"),
    (218, "keypad.binary"),
    (219, "keypad.octal"),
    (220, "keypad.decimal"),
    (221, "keypad.hexadecimal"),
    (224, "left.control"),
    (225, "left.shift"),
    (226, "left.alt"),
    (227, "left.win"),
    (228, "right.control"),
    (229, "right.shift"),
    (230, "right.alt"),
    (231, "right.win"),
    (257, "mode"),
    (258, "sleep"),
    (259, "wake"),
    (260, "channel.up"),
    (261, "channel.down"),
    (262, "media.play"),
    (263, "media.pause"),
    (264, "media.record"),
    (265, "media.fast.forward"),
    (266, "media.rewind"),
    (267, "media.next.track"),
    (268, "media.previous.track"),
    (269, "media.stop"),
    (270, "media.eject"),
    (271, "media.play.pause"),
    (272, "media.select"),
    (273, "ac.new"),
    (274, "ac.open"),
    (275, "ac.close"),
    (276, "ac.exit"),
    (277, "ac.save"),
    (278, "ac.print"),
    (279, "ac.properties"),
    (280, "ac.search"),
    (281, "ac.home"),
    (282, "ac.back"),
    (283, "ac.forward"),
    (284, "ac.stop"),
    (285, "ac.refresh"),
    (286, "ac.bookmarks"),
    (287, "soft.left"),
    (288, "soft.right"),
    (289, "call"),
    (290, "end.call"),
];

/// The mouse buttons with names of their own; others are `key.mouse.<n>`.
const MOUSE: [(i32, &str); 3] = [(1, "left"), (2, "middle"), (3, "right")];

/// The keys the system names with one character on a US layout (`SDL_GetKeyName`), which
/// show that, upper-cased, instead of their translation.
const ONE_CHARACTER: &[(&str, &str)] = &[
    ("minus", "-"),
    ("equal", "="),
    ("left.bracket", "["),
    ("right.bracket", "]"),
    ("backslash", "\\"),
    ("semicolon", ";"),
    ("apostrophe", "'"),
    ("grave.accent", "`"),
    ("comma", ","),
    ("period", "."),
    ("slash", "/"),
];

impl Key {
    /// `InputConstants.Key.getName`, as `options.txt` saves it.
    pub fn name(self) -> String {
        match self {
            Key::Keyboard(v) => match KEYBOARD.iter().find(|(k, _)| *k == v) {
                Some((_, name)) => format!("key.keyboard.{name}"),
                None => format!("key.keyboard.{v}"),
            },
            Key::Mouse(v) => match MOUSE.iter().find(|(k, _)| *k == v) {
                Some((_, name)) => format!("key.mouse.{name}"),
                None => format!("key.mouse.{v}"),
            },
        }
    }

    /// `InputConstants.getKey(String)`: the key a saved name stands for.
    pub fn parse(name: &str) -> Option<Key> {
        if let Some(rest) = name.strip_prefix("key.keyboard.") {
            let named = KEYBOARD.iter().find(|(_, n)| *n == rest).map(|(v, _)| *v);
            named.or_else(|| rest.parse().ok()).map(Key::Keyboard)
        } else if let Some(rest) = name.strip_prefix("key.mouse.") {
            let named = MOUSE.iter().find(|(_, n)| *n == rest).map(|(v, _)| *v);
            named.or_else(|| rest.parse().ok()).map(Key::Mouse)
        } else {
            None
        }
    }

    /// `InputConstants.Key.getDisplayName`: a key the system names with one character shows
    /// it upper-cased, others their translation; a mouse button without one is "Button n".
    pub fn display(self, lang: &HashMap<String, String>) -> String {
        let name = self.name();
        let translated = |key: &str| lang.get(key).cloned();
        match self {
            Key::Keyboard(_) => {
                let short = name.strip_prefix("key.keyboard.").unwrap_or_default();
                if short.len() == 1 {
                    return short.to_uppercase();
                }
                if let Some((_, c)) = ONE_CHARACTER.iter().find(|(n, _)| *n == short) {
                    return (*c).to_owned();
                }
                translated(&name).unwrap_or(name)
            }
            Key::Mouse(v) => translated(&name).unwrap_or_else(|| {
                let template = translated("key.mouse").unwrap_or_else(|| "Button %1$s".into());
                template.replace("%1$s", &v.to_string())
            }),
        }
    }
}

/// `KeyMapping.Category`, in its `SORT_ORDER`.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Category {
    Movement,
    Misc,
    Multiplayer,
    Gameplay,
    Inventory,
    Creative,
    Spectator,
    Debug,
}

impl Category {
    /// `KeyMapping.Category.label`'s translation key.
    pub fn label(self) -> &'static str {
        match self {
            Category::Movement => "key.category.minecraft.movement",
            Category::Misc => "key.category.minecraft.misc",
            Category::Multiplayer => "key.category.minecraft.multiplayer",
            Category::Gameplay => "key.category.minecraft.gameplay",
            Category::Inventory => "key.category.minecraft.inventory",
            Category::Creative => "key.category.minecraft.creative",
            Category::Spectator => "key.category.minecraft.spectator",
            Category::Debug => "key.category.minecraft.debug",
        }
    }
}

/// A `KeyMapping`: its name (the translation key), default key, category and order.
pub struct Mapping {
    pub name: &'static str,
    pub default: Key,
    pub category: Category,
    pub order: i32,
}

const fn key(name: &'static str, scancode: i32, category: Category) -> Mapping {
    ordered(name, Key::Keyboard(scancode), category, 0)
}

const fn mouse(name: &'static str, button: i32, category: Category) -> Mapping {
    ordered(name, Key::Mouse(button), category, 0)
}

const fn ordered(name: &'static str, default: Key, category: Category, order: i32) -> Mapping {
    Mapping {
        name,
        default,
        category,
        order,
    }
}

use Category::*;

/// `Options.keyMappings`, in its order, which `options.txt` saves them in.
pub const MAPPINGS: [Mapping; 62] = [
    mouse("key.attack", 1, Gameplay),
    mouse("key.use", 3, Gameplay),
    key("key.forward", 26, Movement),
    key("key.left", 4, Movement),
    key("key.back", 22, Movement),
    key("key.right", 7, Movement),
    key("key.jump", 44, Movement),
    key("key.sneak", 225, Movement),
    key("key.sprint", 224, Movement),
    key("key.drop", 20, Inventory),
    key("key.inventory", 8, Inventory),
    key("key.chat", 23, Multiplayer),
    key("key.playerlist", 43, Multiplayer),
    mouse("key.pickItem", 2, Gameplay),
    key("key.command", 56, Multiplayer),
    key("key.friends", 18, Multiplayer),
    key("key.socialInteractions", 19, Multiplayer),
    key("key.toggleGui", 58, Misc),
    key("key.toggleSpectatorShaderEffects", 61, Misc),
    key("key.screenshot", 59, Misc),
    key("key.togglePerspective", 62, Misc),
    key("key.smoothCamera", 0, Misc),
    key("key.fullscreen", 68, Misc),
    key("key.spectatorOutlines", 0, Spectator),
    mouse("key.spectatorHotbar", 2, Spectator),
    key("key.swapOffhand", 9, Inventory),
    key("key.saveToolbarActivator", 6, Creative),
    key("key.loadToolbarActivator", 27, Creative),
    key("key.advancements", 15, Misc),
    key("key.quickActions", 10, Misc),
    ordered("key.debug.overlay", Key::Keyboard(60), Debug, -2),
    ordered("key.debug.modifier", Key::Keyboard(60), Debug, -1),
    key("key.hotbar.1", 30, Inventory),
    key("key.hotbar.2", 31, Inventory),
    key("key.hotbar.3", 32, Inventory),
    key("key.hotbar.4", 33, Inventory),
    key("key.hotbar.5", 34, Inventory),
    key("key.hotbar.6", 35, Inventory),
    key("key.hotbar.7", 36, Inventory),
    key("key.hotbar.8", 37, Inventory),
    key("key.hotbar.9", 38, Inventory),
    key("key.debug.reloadChunk", 4, Debug),
    key("key.debug.showHitboxes", 5, Debug),
    key("key.debug.clearChat", 7, Debug),
    key("key.debug.crash", 6, Debug),
    key("key.debug.showChunkBorders", 10, Debug),
    key("key.debug.showAdvancedTooltips", 11, Debug),
    key("key.debug.copyRecreateCommand", 12, Debug),
    key("key.debug.spectate", 17, Debug),
    key("key.debug.switchGameMode", 61, Debug),
    key("key.debug.debugOptions", 63, Debug),
    key("key.debug.focusPause", 19, Debug),
    key("key.debug.dumpDynamicTextures", 22, Debug),
    key("key.debug.reloadResourcePacks", 23, Debug),
    key("key.debug.profiling", 15, Debug),
    key("key.debug.copyLocation", 6, Debug),
    key("key.debug.dumpVersion", 25, Debug),
    ordered("key.debug.profilingChart", Key::Keyboard(30), Debug, 1),
    ordered("key.debug.fpsCharts", Key::Keyboard(31), Debug, 2),
    ordered("key.debug.networkCharts", Key::Keyboard(32), Debug, 3),
    ordered("key.debug.lightmapTexture", Key::Keyboard(33), Debug, 4),
    key("key.debug.improvedTransparency", 27, Debug),
];

/// The mapping called `name`'s place in `MAPPINGS`.
pub fn index(name: &str) -> usize {
    MAPPINGS
        .iter()
        .position(|m| m.name == name)
        .unwrap_or_else(|| panic!("no key mapping {name}"))
}

/// The default bindings, in `MAPPINGS`' order.
pub fn defaults() -> Vec<Key> {
    MAPPINGS.iter().map(|m| m.default).collect()
}

/// `KeyMapping.compareTo`: the mappings by category, then order, then translated name.
pub fn sorted(lang: &HashMap<String, String>) -> Vec<usize> {
    let name = |i: usize| lang.get(MAPPINGS[i].name).map_or(MAPPINGS[i].name, |s| s);
    let mut order: Vec<usize> = (0..MAPPINGS.len()).collect();
    order.sort_by(|&a, &b| {
        let (ma, mb) = (&MAPPINGS[a], &MAPPINGS[b]);
        (ma.category, ma.order)
            .cmp(&(mb.category, mb.order))
            .then_with(|| name(a).cmp(name(b)))
    });
    order
}

/// The categories that have mappings, each listed once over its keys.
pub fn category_count() -> usize {
    let mut categories: Vec<Category> = MAPPINGS.iter().map(|m| m.category).collect();
    categories.sort();
    categories.dedup();
    categories.len()
}

/// `KeyBindsList.KeyEntry.refreshEntry`: whether the mapping `i`, bound in `keys`, shares
/// its key with another where either isn't its default.
pub fn clashes(keys: &[Key], i: usize) -> bool {
    keys[i] != UNBOUND
        && (0..keys.len()).any(|j| {
            j != i
                && keys[j] == keys[i]
                && (keys[j] != MAPPINGS[j].default || keys[i] != MAPPINGS[i].default)
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_save_and_load_by_vanillas_names() {
        assert_eq!(Key::Keyboard(26).name(), "key.keyboard.w");
        assert_eq!(Key::Keyboard(225).name(), "key.keyboard.left.shift");
        assert_eq!(Key::Mouse(3).name(), "key.mouse.right");
        assert_eq!(Key::Mouse(4).name(), "key.mouse.4");
        assert_eq!(UNBOUND.name(), "key.keyboard.unknown");
        for k in [Key::Keyboard(82), Key::Mouse(2), Key::Mouse(7), UNBOUND] {
            assert_eq!(Key::parse(&k.name()), Some(k));
        }
        assert_eq!(Key::parse("key.keyboard.keypad.0"), Some(Key::Keyboard(98)));
    }

    #[test]
    fn keys_show_one_character_or_their_translation() {
        let lang: HashMap<String, String> = [
            ("key.keyboard.space", "Space"),
            ("key.keyboard.unknown", "Not Bound"),
            ("key.mouse.left", "Left Button"),
            ("key.mouse", "Button %1$s"),
        ]
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .into();
        assert_eq!(Key::Keyboard(26).display(&lang), "W");
        assert_eq!(Key::Keyboard(30).display(&lang), "1");
        assert_eq!(Key::Keyboard(56).display(&lang), "/");
        assert_eq!(Key::Keyboard(44).display(&lang), "Space");
        assert_eq!(UNBOUND.display(&lang), "Not Bound");
        assert_eq!(Key::Mouse(1).display(&lang), "Left Button");
        assert_eq!(Key::Mouse(5).display(&lang), "Button 5");
    }

    /// Movement comes first, Debug last; Debug's ordered keys keep their order ahead of
    /// the alphabet, and the rest of a category goes by name.
    #[test]
    fn mappings_sort_like_key_mapping_compare_to() {
        let lang: HashMap<String, String> = MAPPINGS
            .iter()
            .map(|m| (m.name.to_owned(), m.name.to_owned()))
            .collect();
        let sorted: Vec<&str> = sorted(&lang).iter().map(|&i| MAPPINGS[i].name).collect();
        assert_eq!(&sorted[..3], ["key.back", "key.forward", "key.jump"]);
        let debug = sorted
            .iter()
            .position(|n| *n == "key.debug.overlay")
            .unwrap();
        assert_eq!(sorted[debug + 1], "key.debug.modifier");
        assert_eq!(sorted[debug + 2], "key.debug.clearChat");
        assert_eq!(sorted.last(), Some(&"key.debug.lightmapTexture"));
        assert_eq!(category_count(), 8);
    }

    /// Two keys on one key clash once either has been changed, not while both are their
    /// defaults, and an unbound key clashes with nothing.
    #[test]
    fn clashes_need_a_changed_key() {
        let mut keys = defaults();
        let (overlay, modifier) = (index("key.debug.overlay"), index("key.debug.modifier"));
        assert!(!clashes(&keys, overlay) && !clashes(&keys, modifier));
        let (forward, jump) = (index("key.forward"), index("key.jump"));
        keys[forward] = keys[jump];
        assert!(clashes(&keys, forward) && clashes(&keys, jump));
        keys[forward] = UNBOUND;
        keys[index("key.back")] = UNBOUND;
        assert!(!clashes(&keys, forward));
    }
}
