#!/usr/bin/env python3
"""A player put on a boat, a minecart and a horse by `/ride` on a dedicated server, against vanilla's, seen by a
vanilla client.

A vanilla dedicated server and Sunrise's (`sunrise-client --dedicated --nogui`) each serve a copy of one flat world
vanilla generated first, with Alice an operator. LocatorBarClient (a player made of vanilla's protocol code), with
`mounts on`, records each teleport as its packet carries it (exact numbers, motion, rotation, relative flags), each
passenger list in order (Alice as `self`) and the effects of entities other than Alice.

ServerPlayer.startRiding seats Alice (positionRider), teleports her there keeping her look (Relative.ROTATION),
sends a living vehicle's effects (PlayerList.sendActiveEffects) and the vehicle's passengers, all before the
command's reply; the tracker sends the passengers again on its next look.
- The boat already carries a pig, so Alice sits first, in the front seat of a full boat (getPassengers().indexOf),
  and she faces 150 degrees from it: AbstractBoat.clampRotation leaves her at 105, which `/data get` shows.
- The minecart floats (NoGravity) at y 0.04 over a stone floor, where adding the cart's attachment and then taking
  off the player's rounds differently from taking off their difference.
- The horse (NoAI, so it keeps its facing) has an infinite Speed without particles.

Alice's teleports, passenger lists, the other entities' effects and the command replies in the mount steps and the
one after the boat's are compared exactly; the ride's reply by its key alone (the hover carries the vehicle's random
UUID). Clearing the world and the dismounts between the mounts are not compared.
"""
import sys

try:
    from . import test_locator_bar_parity as locator
except ImportError:
    import test_locator_bar_parity as locator

NAME = 'mount-packets-parity'


def build(command):
    return f'cmd Alice {command}'


def nearest(kind):
    return f'@e[type=minecraft:{kind},limit=1,sort=nearest]'


SCRIPT = [
    ('setup', [
        'chat on', 'mounts on',
        build('gamerule spawn_monsters false'), build('gamerule spawn_mobs false'),
        build('forceload add -16 -16 16 16'),
        build('tp @s 1.0 -60 0.5 150 0'),
        # A floor under the cart, so its dismount sets Alice down on it rather than in the air.
        build('fill 1 -1 -1 5 -1 2 minecraft:stone'),
    ]),
    # The flat world's own slimes and animals load with their chunks, after the setup: one that bumps the boat takes
    # its last seat. Once they are gone, the vehicles are summoned.
    ('off-world', [
        build('kill @e[type=!minecraft:player]'),
        build('summon minecraft:oak_boat 1.0 -60 0.5 {Rotation:[0f,0f]}'),
        build('summon minecraft:pig 1.0 -60 0.5 {NoAI:1b}'),
        build(f'ride {nearest("pig")} mount {nearest("oak_boat")}'),
        build('summon minecraft:minecart 3.0 0.04 0.5 {NoGravity:1b,Rotation:[0f,0f]}'),
        build('summon minecraft:horse 5.0 -60 0.5 {NoAI:1b,Rotation:[0f,0f]}'),
        build(f'effect give {nearest("horse")} minecraft:speed infinite 1 true'),
    ]),
    ('boat', [build(f'ride @s mount {nearest("oak_boat")}')]),
    ('seated', [
        build('data get entity @s Rotation'),
        build('rotate @s 0 0'),
        build('effect give @s minecraft:instant_health 1 9 true'),
    ]),
    ('off-boat', [build('ride @s dismount')]),
    ('minecart', [build(f'ride @s mount {nearest("minecart")}')]),
    ('off-minecart', [build('ride @s dismount')]),
    ('horse', [build(f'ride @s mount {nearest("horse")}')]),
]

MOUNTS = ('boat', 'minecart', 'horse')

KEPT = ('teleport', 'passengers', 'other-effect', 'chat', 'error')

# The replies that name an entity: its hover carries its random UUID, so only their key is compared.
NAMES_ENTITY = ('"commands.ride.mount.success"',)


def shown(lines):
    """Alice's teleports, passenger lists, other entities' effects and command replies from the first mount on; the
    setup and the steps between the mounts (`off-*`: clearing the world, the dismounts) are left out."""
    steps = {f'step={step}' for step, _ in SCRIPT if step != 'setup' and not step.startswith('off-')}
    kept = []
    for line in lines:
        parts = line.split(' ', 3)
        if len(parts) < 3 or parts[1] not in steps or parts[2] not in KEPT:
            continue
        key = next((key for key in NAMES_ENTITY if key in line), None)
        kept.append(' '.join(parts[:3] + [key]) if key else line)
    return kept


def check(vanilla, vanilla_all):
    """Each mount's packets reach vanilla's client in startRiding's order, before the reply: the seat teleport, the
    horse's effect, then the passengers."""
    for step in MOUNTS:
        kinds = []
        for line in vanilla:
            parts = line.split(' ')
            if parts[1] != f'step={step}':
                continue
            if parts[2] == 'chat' and NAMES_ENTITY[0] in line:
                kinds.append('reply')
                break
            if parts[2] in ('teleport', 'other-effect', 'passengers') and parts[2] not in kinds:
                kinds.append(parts[2])
        want = ['teleport'] + (['other-effect'] if step == 'horse' else []) + ['passengers', 'reply']
        if kinds != want:
            return f'vanilla\'s {step} mount sent {kinds}, not {want}'
    return None


def main():
    lines = locator.one_player(NAME, SCRIPT, shown, check, 'A player put on a boat, a minecart and a horse by /ride '
                               'on a dedicated server: the seat teleport, the mount\'s effects and the passengers '
                               'before the reply.')
    print(f'mount packets: {len(lines)} lines identical')


if __name__ == '__main__':
    try:
        main()
    except AssertionError as error:
        sys.exit(f'{NAME}: {error}')
