#!/usr/bin/env python3
"""Turns vanilla's decompiled animation definitions (net/minecraft/client/animation/definitions/*.java)
into Rust data for crates/mc-render/src/animations.rs. Usage: gen_animations.py SRC_DIR OUT.rs CLASS... [!CONSTANT...],
where each !CONSTANT names a generated constant to leave out, such as one vanilla's renderers never use."""
import re, sys
src, out = sys.argv[1], sys.argv[2]
classes = [a for a in sys.argv[3:] if not a.startswith('!')]
skip = {a[1:] for a in sys.argv[3:] if a.startswith('!')}
num = r'(-?[0-9.]+(?:E-?[0-9]+)?)F?'
vec = rf'KeyframeAnimations\.(degreeVec|posVec|scaleVec)\({num}, {num}, {num}\)'
key = re.compile(rf'new Keyframe\({num}, {vec}, Interpolations\.(LINEAR|CATMULLROM)\)')
def lit(s):
    s = s.rstrip('F')
    # Rust wants a digit after the point: `4.4E-4` stays, `5E-4` becomes `5.0E-4`.
    mant, _, exp = s.partition('E')
    mant = mant if '.' in mant else mant + '.0'
    return mant + ('E' + exp if exp else '')
lines = ['//! Vanilla 26.3\'s keyframe animations (`net.minecraft.client.animation.definitions`), as',
         '//! data for [`crate::animation`]. Generated from the decompiled definitions by',
         '//! `xtask/gen_animations.py`; edit that, not this.', '',
         '// Vanilla\'s rounded values, such as 0.7071, aren\'t the constants they look like.',
         '#![allow(clippy::approx_constant)]', '',
         'use crate::animation::{Animation, Channel, Frame, Target, deg, pos, scale};', '']
for cls in classes:
    text = open(f'{src}/{cls}.java').read()
    prefix = re.sub(r'(?<!^)(?=[A-Z])', '_', cls.removesuffix('Animation')).upper()
    total = len(re.findall(r'new Keyframe\(', text)); seen = 0
    for m in re.finditer(r'public static final AnimationDefinition (\w+) = (?:AnimationDefinition\.)?Builder\.withLength\(' + num + r'\)(.*?)\.build\(\);', text, re.S):
        name, length, body = m.group(1), m.group(2), m.group(3)
        # The class's name, then the definition's without the words that repeat it:
        # `ARMADILLO_BABY_PEEK` in `BabyArmadilloAnimation` is `BABY_ARMADILLO_PEEK`.
        words = name.upper().split('_')
        while len(words) > 1 and words[0] in prefix.split('_'):
            words.pop(0)
        rust = prefix + '_' + '_'.join(words)
        looping = '.looping()' in body.split('.addAnimation', 1)[0]
        start = len(lines)
        lines.append(f'/// `{cls}.{name}`.')
        lines.append(f'pub(crate) const {rust}: Animation = Animation {{')
        lines.append(f'    length: {lit(length)},')
        lines.append(f'    looping: {"true" if looping else "false"},')
        lines.append('    channels: &[')
        for a in re.finditer(r'\.addAnimation\(\s*"(\w+)",\s*new AnimationChannel\(\s*Targets\.(\w+),\s*new Keyframe\[\]\{(.*?)\}\s*\)\s*\)', body, re.S):
            bone, target, frames = a.groups()
            ks = key.findall(frames)
            assert len(ks) == frames.count('new Keyframe('), (cls, name, bone, frames)
            seen += len(ks)
            lines.append('        Channel {')
            lines.append(f'            bone: "{bone}",')
            lines.append(f'            target: Target::{target.title()},')
            lines.append('            frames: &[')
            for t, f, x, y, z, interp in ks:
                fn = {'degreeVec': 'deg', 'posVec': 'pos', 'scaleVec': 'scale'}[f]
                lines.append(f'                Frame::new({lit(t)}, {fn}({lit(x)}, {lit(y)}, {lit(z)}), {"true" if interp == "CATMULLROM" else "false"}),')
            lines.append('            ],')
            lines.append('        },')
        lines.append('    ],')
        lines.append('};')
        lines.append('')
        if rust in skip:
            skip.remove(rust)
            del lines[start:]
    assert seen == total, (cls, seen, total)
    print(cls, total, 'keyframes')
assert not skip, ('no such constants', skip)
open(out, 'w').write('\n'.join(lines))
