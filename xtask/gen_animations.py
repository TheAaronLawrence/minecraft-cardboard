#!/usr/bin/env python3
"""Turns vanilla's decompiled animation definitions (net/minecraft/client/animation/definitions/*.java)
into Rust data for crates/mc-render/src/animations.rs. Usage: gen_animations.py SRC_DIR OUT.rs CLASS..."""
import re, sys
src, out, classes = sys.argv[1], sys.argv[2], sys.argv[3:]
num = r'(-?[0-9.]+)F?'
vec = rf'KeyframeAnimations\.(degreeVec|posVec|scaleVec)\({num}, {num}, {num}\)'
key = re.compile(rf'new Keyframe\({num}, {vec}, Interpolations\.(LINEAR|CATMULLROM)\)')
def lit(s):
    s = s.rstrip('F')
    return s if '.' in s else s + '.0'
lines = ['//! Vanilla 26.3\'s keyframe animations (`net.minecraft.client.animation.definitions`), as',
         '//! data for [`crate::animation`]. Generated from the decompiled definitions by',
         '//! `xtask/gen_animations.py`; edit that, not this.', '',
         'use crate::animation::{Animation, Channel, Frame, Target, deg, pos, scale};', '']
for cls in classes:
    text = open(f'{src}/{cls}.java').read()
    prefix = re.sub(r'(?<!^)(?=[A-Z])', '_', cls.removesuffix('Animation')).upper()
    total = len(re.findall(r'new Keyframe\(', text)); seen = 0
    for m in re.finditer(r'public static final AnimationDefinition (\w+) = (?:AnimationDefinition\.)?Builder\.withLength\(' + num + r'\)(.*?)\.build\(\);', text, re.S):
        name, length, body = m.group(1), m.group(2), m.group(3)
        # Names that already say whose they are (`CAMEL_BABY_WALK`, `ARMADILLO_BABY_PEEK`) stay.
        rust = name if name.startswith((prefix, prefix.split('_')[-1] + '_')) else f'{prefix}_{name}'
        looping = '.looping()' in body.split('.addAnimation', 1)[0]
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
    assert seen == total, (cls, seen, total)
    print(cls, total, 'keyframes')
open(out, 'w').write('\n'.join(lines))
