//! The player's own hands as they see them: the swing (`LivingEntity.SwingState`), the
//! lag behind the view (`LocalPlayer`'s `xBob` and `yBob`), and the dip that swaps the
//! items shown (`FirstPersonHandsAndItems`).

use crate::entities::SwingState;
use glam::{Mat4, Vec3};
use std::f32::consts::PI;

/// `SwingAnimation.DEFAULT`: a whack (`SwingAnimationType` 1) of 6 ticks, how every item
/// here swings. Spears' stab isn't drawn yet.
pub const WHACK: (u8, i32) = (1, 6);

#[derive(Default)]
pub struct Hands {
    pub swing: SwingState,
    /// `xBob` and `yBob`, each last tick and now.
    bob: [[f32; 2]; 2],
    /// `mainHandHeight` and `offHandHeight`, each last tick and now.
    height: [[f32; 2]; 2],
    /// `mainHandItem` and `offHandItem`: the items shown, which lag the held ones.
    shown: [Option<String>; 2],
    /// `Player.lastItemInMainHand` and `itemSwapTicker`.
    last_main: Option<String>,
    swap_ticker: u32,
}

impl Hands {
    /// `LivingEntity.swing`: the arm of `hand` (0 main, 1 off) swings.
    pub fn swing(&mut self, hand: u8) {
        let (kind, duration) = WHACK;
        self.swing.start_if_able((hand, kind, duration));
    }

    /// `Player.resetAttackStrengthTicker`: an attack starts the item swap's rise again.
    pub fn reset_swap(&mut self) {
        self.swap_ticker = 0;
    }

    /// `Player.getCurrentItemAttackStrengthDelay`: ticks for the held item's attack to
    /// recharge.
    pub fn attack_delay(&self) -> f32 {
        20.0 / attack_speed(self.last_main.as_deref())
    }

    /// `Player.getAttackStrengthScale(0)`: how far the attack has recharged, 0 to 1. The
    /// item swap's ticker is the attack's too: both reset together.
    pub fn attack_strength(&self) -> f32 {
        (self.swap_ticker as f32 / self.attack_delay()).clamp(0.0, 1.0)
    }

    /// `FirstPersonHandsAndItems.itemUsed`: placing or using drops the hand at once, to
    /// rise again.
    pub fn item_used(&mut self, hand: usize) {
        self.height[hand][1] = 0.0;
    }

    /// One tick, holding `held` in the main and off hand, looking at `yaw` and `pitch`:
    /// `Player.tick`'s swap ticker, `LocalPlayer.aiStep`'s bob, the swing and
    /// `FirstPersonHandsAndItems.tick`.
    pub fn tick(&mut self, held: [Option<&str>; 2], yaw: f32, pitch: f32) {
        self.swap_ticker += 1;
        if self.last_main.as_deref() != held[0] {
            // Another item resets the ticker (`ItemStack.isSameItem`).
            self.reset_swap();
            self.last_main = held[0].map(str::to_owned);
        }
        for (bob, rot) in self.bob.iter_mut().zip([pitch, yaw]) {
            bob[0] = bob[1];
            bob[1] += (rot - bob[1]) * 0.5;
        }
        self.swing.tick();
        // `getItemSwapScale(1)`: how far into the attack delay (`getCurrentItemAttackStrengthDelay`)
        // the ticker is.
        let delay = 20.0 / attack_speed(held[0]);
        let swap = ((self.swap_ticker as f32 + 1.0) / delay).clamp(0.0, 1.0);
        for (hand, next) in held.into_iter().enumerate() {
            let height = &mut self.height[hand];
            height[0] = height[1];
            // The same item (whatever its count or components) is shown at once
            // (`shouldInstantlyReplaceVisibleItem`); another lowers the hand to swap.
            let target = if self.shown[hand].as_deref() != next {
                0.0
            } else if hand == 0 {
                swap * swap * swap
            } else {
                1.0
            };
            height[1] += (target - height[1]).clamp(-0.4, 0.4);
            if height[1] < 0.1 {
                self.shown[hand] = next.map(str::to_owned);
            }
        }
    }

    /// `FirstPersonHandsAndItems.extractRenderState`, `pt` of the way to the next tick,
    /// with the view at `yaw` and `pitch` and the player's skin.
    pub fn view(
        &self,
        pt: f32,
        yaw: f32,
        pitch: f32,
        (skin, slim): (String, bool),
    ) -> mc_render::entity::FirstPerson {
        let lerp = |[old, now]: [f32; 2]| old + (now - old) * pt;
        mc_render::entity::FirstPerson {
            skin,
            slim,
            left_handed: false,
            items: self.shown.clone(),
            lowered: self.height.map(|h| 1.0 - lerp(h)),
            swing: self
                .swing
                .current
                .map(|(hand, kind, _)| (hand as usize, kind, self.swing.animation(pt))),
            sway: [pitch - lerp(self.bob[0]), yaw - lerp(self.bob[1])],
        }
    }
}

/// The `ATTACK_SPEED` attribute holding `item`: 4, plus the tool's modifier from `Items`
/// (`Item.Properties.sword`, `pickaxe`, `axe`, `shovel` and `hoe`, the trident's and the
/// mace's). Spears' and enchantments' effects aren't counted.
fn attack_speed(item: Option<&str>) -> f32 {
    let name = item.map_or("", |i| i.strip_prefix("minecraft:").unwrap_or(i));
    let (material, tool) = name.rsplit_once('_').unwrap_or(("", name));
    let modifier = match (tool, material) {
        ("sword", _) => -2.4,
        ("pickaxe", _) => -2.8,
        ("shovel", _) => -3.0,
        ("axe", "wooden" | "stone" | "copper") => -3.2,
        ("axe", "iron") => -3.1,
        ("axe", _) => -3.0,
        ("hoe", "wooden" | "golden") => -3.0,
        ("hoe", "stone" | "copper") => -2.0,
        ("hoe", "iron") => -1.0,
        ("hoe", _) => 0.0,
        ("trident", _) => -2.9,
        ("mace", _) => -3.4,
        _ => 0.0,
    };
    4.0 + modifier
}

/// What shakes the view: `ClientAvatarState`'s walk distance and bob, and
/// `LivingEntity`'s `hurtTime`, `hurtDir` and `deathTime`.
#[derive(Default)]
pub struct ViewBob {
    /// `walkDist`, last tick and now.
    walk: [f32; 2],
    /// `bob`, last tick and now.
    bob: [f32; 2],
    /// Ticks left of the hurt tilt, counting down from 10 (`hurtDuration`).
    hurt_time: i32,
    /// `Player.hurtDir`: where the last hit came from, relative to the player's yaw.
    hurt_dir: f32,
    /// Ticks since the player died.
    death_time: u32,
}

impl ViewBob {
    /// `LivingEntity.handleDamageEvent` and `LocalPlayer.hurtTo`: the view tilts.
    pub fn hurt(&mut self) {
        self.hurt_time = 10;
    }

    /// `Player.animateHurt`: the view tilts away from `dir`.
    pub fn hurt_from(&mut self, dir: f32) {
        self.hurt_dir = dir;
        self.hurt();
    }

    /// The start of a tick: `ClientAvatarState.tick`, `LivingEntity.baseTick`'s hurt
    /// countdown and `tickDeath`, then `AbstractClientPlayer.updateBob`, from the
    /// player's movement last tick. The bob only follows walking on the ground, not swimming.
    pub fn tick(&mut self, velocity: glam::DVec3, on_ground: bool, dead: bool) {
        self.walk[0] = self.walk[1];
        self.hurt_time = (self.hurt_time - 1).max(0);
        self.death_time = if dead { self.death_time + 1 } else { 0 };
        let target = if on_ground && !dead {
            (velocity.x.hypot(velocity.z) as f32).min(0.1)
        } else {
            0.0
        };
        self.bob[0] = self.bob[1];
        self.bob[1] += (target - self.bob[1]) * 0.4;
    }

    /// `LocalPlayer.move`'s `addWalkedDistance`: the player moved this far across.
    pub fn walked(&mut self, horizontal: f32) {
        self.walk[1] += horizontal * 0.6;
    }

    /// A respawned player is a new one, still.
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// `GameRenderer.bobHurt` then `bobView`, `pt` of the way to the next tick: a
    /// transform in view space, applied between the view and the projection to the world
    /// and the hands alike.
    pub fn matrix(&self, pt: f32, dead: bool) -> Mat4 {
        let mut m = Mat4::IDENTITY;
        let deg = f32::to_radians;
        // The view rolls over as the player dies.
        if dead {
            let duration = (self.death_time as f32 + pt).min(20.0);
            m *= Mat4::from_rotation_z(deg(40.0 - 8000.0 / (duration + 200.0)));
        }
        let hurt = self.hurt_time as f32 - pt;
        if hurt >= 0.0 {
            let hurt = (hurt / 10.0).powi(4) * PI;
            m *= Mat4::from_rotation_y(deg(-self.hurt_dir))
                * Mat4::from_rotation_z(deg(-hurt.sin() * 14.0))
                * Mat4::from_rotation_y(deg(self.hurt_dir));
        }
        // `getBackwardsInterpolatedWalkDistance` and `getInterpolatedBob`.
        let [walk_old, walk] = self.walk;
        let distance = -(walk + (walk - walk_old) * pt) * PI;
        let bob = self.bob[0] + (self.bob[1] - self.bob[0]) * pt;
        m * Mat4::from_translation(Vec3::new(
            distance.sin() * bob * 0.5,
            -(distance.cos() * bob).abs(),
            0.0,
        )) * Mat4::from_rotation_z(deg(distance.sin() * bob * 3.0))
            * Mat4::from_rotation_x(deg(((distance - 0.2).cos() * bob).abs() * 5.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Switching to a sword: the empty hand drops 0.4 a tick until it's under 0.1, shows
    /// the sword, then rises as the swap ticker fills the sword's 12.5-tick attack delay,
    /// cubed, at most 0.4 a tick.
    #[test]
    fn swapping_items_dips_the_hand() {
        let mut h = Hands::default();
        for _ in 0..20 {
            h.tick([None, None], 0.0, 0.0);
        }
        assert_eq!(h.height[0][1], 1.0);
        let sword = Some("minecraft:iron_sword");
        let mut heights = Vec::new();
        for _ in 0..6 {
            h.tick([sword, None], 0.0, 0.0);
            heights.push(h.height[0][1]);
        }
        // Down 0.4, 0.4, then to 0 (under 0.1, so the sword shows); then the ticker,
        // reset on the switch, is at 3, 4, 5 of 12.5 by the next ticks.
        let rise = |t: f32| ((t + 1.0) / 12.5_f32).powi(3);
        let expected = [0.6, 0.2, 0.0, rise(3.0), rise(4.0), rise(5.0)];
        for (got, want) in heights.iter().zip(expected) {
            assert!((got - want).abs() < 1e-5, "{heights:?} vs {expected:?}");
        }
        assert_eq!(h.shown[0].as_deref(), sword);
    }

    /// A hit rolls the view 14° at the hurt curve's peak, about the axis the player
    /// looks along when it comes from straight ahead or behind (`hurtDir` 0); from the
    /// side (90) the same turn pitches the view instead. Walking lowers the view.
    #[test]
    fn hurt_tilts_the_view_away_from_the_hit() {
        let mut v = ViewBob::default();
        v.hurt_from(0.0);
        // Peak: (hurt / 10)^4 = 1/2, at 8.41 ticks left.
        let pt = 10.0 - 0.5_f32.powf(0.25) * 10.0;
        let up = |m: Mat4| m.transform_vector3(Vec3::Y);
        let tilt = |m: Mat4| up(m).angle_between(Vec3::Y).to_degrees();
        let rolled = v.matrix(pt, false);
        assert!((tilt(rolled) - 14.0).abs() < 1e-3, "{}", tilt(rolled));
        assert!(
            up(rolled).z.abs() < 1e-5 && up(rolled).x > 0.0,
            "{:?}",
            up(rolled)
        );
        v.hurt_from(90.0);
        let pitched = v.matrix(pt, false);
        assert!((tilt(pitched) - 14.0).abs() < 1e-3);
        assert!(up(pitched).x.abs() < 1e-5, "{:?}", up(pitched));

        let mut v = ViewBob::default();
        for _ in 0..20 {
            v.tick(glam::DVec3::new(0.0, 0.0, 0.2), true, false);
            v.walked(0.2);
        }
        // The bob settles at 0.1; the view drops by |cos(walk·π)| of it.
        let drop = v.matrix(0.0, false).transform_point3(Vec3::ZERO).y;
        let walk = 20.0 * 0.2 * 0.6 * PI;
        assert!((drop + (walk.cos() * 0.1).abs()).abs() < 1e-3, "{drop}");
    }
}
