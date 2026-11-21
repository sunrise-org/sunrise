#!/usr/bin/env python3
"""Ender pearls through Nether and End portals on a dedicated server, against vanilla's, seen by a vanilla client.

A vanilla dedicated server and Sunrise's (`sunrise-client --dedicated --nogui`) each serve a copy of one flat world
vanilla generated first, with Alice an operator in survival. LocatorBarClient (a player made of vanilla's protocol
code) builds a Nether portal in the Overworld, its partner in a cleared, force-loaded room of the Nether, an End
portal wall in the Overworld and one beside the End's spawn platform, then sends ender pearls of hers through them
(summoned with a fixed motion: a throw without its random spread, so both servers fly the same pearl). Each lands
on an open floor:

- No portal: how far the pearl flies.
- Overworld to Nether: the pearl changes level (ThrowableProjectile.canUsePortal) and takes Alice into the Nether
  (ThrownEnderpearl.onHit, isAllowedToTeleportOwner across levels) with its 5 damage.
- Nether to Overworld: the same back.
- Riding a boat, Overworld to Nether: the owner is taken although a passenger (canUsePortal(true)).
- Overworld to the End: the pearl leaves the End's spawn facing west, its motion turned with it
  (Relative.ROTATE_DELTA).
- End to Overworld before Alice has seen the credits: the pearl stays in the End (ThrownEnderpearl.canTeleport).
- End to Overworld after the credits (an End portal walked through, the credits answered with PERFORM_RESPAWN).

Alice's level changes, teleports, mounts and the replies to her `data get` of dimension, position, health and
portal cooldown in each throw and the read after it are compared exactly.
"""
import sys

try:
    from . import test_locator_bar_parity as locator
except ImportError:
    import test_locator_bar_parity as locator

NAME = 'pearl-portal-parity'
# server_dir and generate_world write these server.properties.
locator.PROPERTIES = dict(locator.PROPERTIES, gamemode='survival')

NETHER = 'execute in minecraft:the_nether run'
END = 'execute in minecraft:the_end run'
# What Alice is asked after every landing: the replies are compared.
ASK = ['cmd Alice data get entity @s Dimension', 'cmd Alice data get entity @s Pos', 'cmd Alice data get entity @s Health',
       'cmd Alice data get entity @s PortalCooldown']


def build(command):
    return f'cmd Alice {command}'


# Full health before each throw: every landing is read from 20.
HEAL = 'cmd Alice effect give @s minecraft:instant_health 1 9 true'
# The slimes the flat world spawned while vanilla generated it, cleared before each throw once their chunks are in:
# they attack Alice at their own random moments.
CLEAR = 'cmd Alice execute in minecraft:overworld run kill @e[type=minecraft:slime]'
# Alice's offline UUID (`OfflinePlayer:Alice`) as the pearls' Owner.
ALICE = '[I;278005000,-707248403,-1814916367,-1817188393]'


def pearl(at, motion, level=None):
    """An ender pearl of Alice's at `at` flying with `motion`: a throw without the shot's random spread, so both
    servers fly the same pearl."""
    x, y, z = at
    mx, my, mz = motion
    command = (f'summon minecraft:ender_pearl {x} {y} {z} '
               f'{{Motion:[{mx}d,{my}d,{mz}d],Owner:{ALICE}}}')
    return build(f'execute in minecraft:{level} run {command}' if level else command)


SCRIPT = [
    ('setup', [
        'chat on',
        # No regeneration between the landing and the health read, no endermite (its 5% draw still happens) and
        # no natural spawns (a flat world's slimes attack Alice at their own random moments).
        build('gamerule natural_health_regeneration false'), build('gamerule spawn_monsters false'),
        build('gamerule spawn_mobs false'),
        build('give @s minecraft:ender_pearl 16'),
        # Every pearl lands on an open floor, never against a wall its owner would stand in (an owner taken to
        # a pearl's last position beside a wall suffocates). The world spawn, where a pearl out of the End
        # arrives, is moved off the portals' line.
        build('setworldspawn 40 -60 0'),
        # The Overworld's Nether portal along x at z 4: its inside x 0..1, y -60..-58.
        build('fill -1 -61 4 2 -57 4 minecraft:obsidian'),
        build('fill 0 -60 4 1 -58 4 minecraft:nether_portal[axis=x]'),
        # A long Nether room on an obsidian floor around its partner at z 0 (inside x 0..1, y 65..67).
        build(f'{NETHER} forceload add -32 -48 32 48'),
        build(f'{NETHER} fill -6 64 -24 6 74 30 minecraft:air'),
        build(f'{NETHER} fill -6 63 -24 6 63 30 minecraft:obsidian'),
        build(f'{NETHER} fill -1 64 0 2 68 0 minecraft:obsidian'),
        build(f'{NETHER} fill 0 65 0 1 67 0 minecraft:nether_portal[axis=x]'),
        # An End portal wall in the Overworld at z 4, at eye height.
        build('fill 10 -60 4 11 -58 4 minecraft:end_portal'),
        build(f'{END} forceload add 80 -20 120 40'),
        # A floor west of the End's spawn platform, where a pearl through the Overworld's End portal lands (an
        # owner left over the void is kicked for floating).
        build(f'{END} fill 84 48 -6 97 48 6 minecraft:obsidian'),
        build('forceload add -32 -32 64 48'),
    ]),
    # A pearl that meets no portal: how far a throw flies, for the landings below.
    ('aim-plain', [build('tp @s 20.5 -60 0.5 0 0'), HEAL, CLEAR]),
    ('plain', [pearl((20.5, -58.48, 0.5), (0.0, 0.0, 1.5))]),
    ('landed-plain', ASK),
    ('aim-overworld-to-nether', [build('tp @s 1.0 -60 0.5 0 0'), HEAL, CLEAR]),
    ('overworld-to-nether', [pearl((1.0, -58.48, 0.5), (0.0, 0.0, 1.5))]),
    ('in-nether', ASK),
    ('aim-nether-to-overworld', [build(f'{NETHER} tp @s 1.0 64 3.5 180 0'), HEAL, CLEAR]),
    ('nether-to-overworld', [pearl((1.0, 65.52, 3.5), (0.0, 0.0, -1.5), 'the_nether')]),
    ('in-overworld', ASK),
    ('aim-boat-to-nether', [
        build('execute in minecraft:overworld run tp @s 1.0 -60 0.5 0 0'),
        build('summon minecraft:oak_boat 1.0 -60 0.5'),
        build('ride @s mount @e[type=minecraft:oak_boat,limit=1,sort=nearest]'),
        build('rotate @s 0 0'),
        HEAL,
        CLEAR,
    ]),
    ('boat-to-nether', [pearl((1.0, -58.9, 0.5), (0.0, 0.0, 1.5))]),
    ('boat-in-nether', ASK),
    ('aim-overworld-to-end', [build('execute in minecraft:overworld run tp @s 10.5 -60 0.5 0 0'), HEAL, CLEAR]),
    ('overworld-to-end', [pearl((10.5, -58.48, 0.5), (0.0, 0.0, 1.5))]),
    ('in-end', ASK),
    ('aim-end-without-credits', [
        # The dragon the first visit spawned would attack Alice at its own random moments.
        build(f'{END} kill @e[type=minecraft:ender_dragon]'),
        # An End portal wall beside the spawn platform and a long floor past it.
        build(f'{END} fill 98 48 3 103 48 34 minecraft:obsidian'),
        build(f'{END} fill 100 49 4 101 51 4 minecraft:end_portal'),
        build(f'{END} tp @s 100.5 49 0.5 0 0'),
        HEAL,
        CLEAR,
    ]),
    ('end-without-credits', [pearl((100.5, 50.52, 0.5), (0.0, 0.0, 1.5), 'the_end')]),
    ('stayed-in-end', ASK),
    # Into the End portal: the credits, answered as a client does.
    ('aim-credits', [build(f'{END} tp @s 100.5 50 4.5 0 0')]),
    ('aim-credits-seen', ['respawn Alice']),
    ('aim-end-with-credits', [build(f'{END} tp @s 100.5 49 0.5 0 0'), HEAL, CLEAR]),
    ('end-with-credits', [pearl((100.5, 50.52, 0.5), (0.0, 0.0, 1.5), 'the_end')]),
    ('back-in-overworld', ASK),
]

KEPT = ('teleport', 'respawn', 'passenger', 'chat', 'killed', 'gamemode', 'error')


def shown(lines):
    """Alice's level changes, teleports, mounts and command replies in the throws and the reads after them; the
    setup and aiming steps, which move her by command, are left out."""
    steps = {f'step={step}' for step, _ in SCRIPT if step != 'setup' and not step.startswith('aim-')}
    # A summoned pearl's reply names its random UUID.
    return [line for line in lines if len(parts := line.split(' ', 3)) >= 3 and parts[1] in steps
            and parts[2] in KEPT and '"commands.summon.success"' not in line]


def check(vanilla, vanilla_all):
    respawns = [line.split(' ')[1] for line in vanilla if line.split(' ')[2] == 'respawn']
    for step in ('step=overworld-to-nether', 'step=nether-to-overworld', 'step=boat-to-nether',
                 'step=overworld-to-end', 'step=end-with-credits'):
        if step not in respawns:
            return f'vanilla sent Alice no level change in {step}'
    return None


def main():
    lines = locator.one_player(NAME, SCRIPT, shown, check, 'Ender pearls thrown into Nether and End portals on a '
                               'dedicated server: the pearl changes level and takes its owner after it.')
    print(f'pearl portal: {len(lines)} lines identical')


if __name__ == '__main__':
    try:
        main()
    except AssertionError as error:
        sys.exit(f'{NAME}: {error}')
