#!/usr/bin/env python3
"""The respawn after the End credits on a dedicated server, against vanilla's, seen by a vanilla client.

A vanilla dedicated server and Sunrise's (`sunrise-client --dedicated --nogui`) each serve a copy of one flat world
vanilla generated first, with Alice an operator in survival and no respawn point of her own. LocatorBarClient (a
player made of vanilla's protocol code) moves the world spawn, builds an End portal wall beside the End's spawn
platform and steps into it: her first exit from the End shows the credits (EndPortalBlock.entityInside). She answers
them as a client does (ServerboundClientCommandPacket PERFORM_RESPAWN): vanilla's
ServerGamePacketListenerImpl.handleClientCommand runs PlayerList.respawn(keepAllPlayerData = true), which sends the
Respawn into the Overworld and then her position at the world spawn, without a portal's sound.

Alice's level changes, teleports and level events in the respawn, and the replies to her `data get` of dimension and
position after it, are compared exactly.
"""
import sys

try:
    from . import test_locator_bar_parity as locator
except ImportError:
    import test_locator_bar_parity as locator

NAME = 'credits-respawn-parity'
# server_dir and generate_world write these server.properties.
locator.PROPERTIES = dict(locator.PROPERTIES, gamemode='survival')

END = 'execute in minecraft:the_end run'


def build(command):
    return f'cmd Alice {command}'


SCRIPT = [
    ('setup', [
        'chat on',
        'events on',
        # No mob attacks Alice at its own random moments.
        build('gamerule spawn_monsters false'), build('gamerule spawn_mobs false'),
        # The world spawn, where the respawn lands, away from where Alice joined, loaded so its slimes can be
        # cleared.
        build('setworldspawn 40 -60 0'),
        build('forceload add 0 -32 80 32'),
        # An End portal wall beside the End's spawn platform, on a floor.
        build(f'{END} forceload add 80 -20 120 40'),
        build(f'{END} fill 98 48 3 103 48 34 minecraft:obsidian'),
        build(f'{END} fill 100 49 4 101 51 4 minecraft:end_portal'),
    ]),
    # The dragon the End's first load spawned would attack Alice at its own random moments.
    ('aim-end', [build(f'{END} tp @s 100.5 49 0.5 0 0'), build(f'{END} kill @e[type=minecraft:ender_dragon]')]),
    # Into the portal: the credits roll. The slimes the flat world spawned while vanilla generated it would attack
    # Alice at the world spawn at their own random moments.
    ('aim-credits', [build('execute in minecraft:overworld run kill @e[type=minecraft:slime]'),
                     build(f'{END} tp @s 100.5 50 4.5 0 0')]),
    # The credits answered.
    ('respawn', ['respawn Alice']),
    ('back', [build('data get entity @s Dimension'), build('data get entity @s Pos')]),
]

KEPT = ('teleport', 'respawn', 'event', 'chat', 'error')


def shown(lines):
    """Alice's level changes, teleports, level events and command replies in the respawn and the read after it."""
    steps = {f'step={step}' for step, _ in SCRIPT if step != 'setup' and not step.startswith('aim-')}
    return [line for line in lines if len(parts := line.split(' ', 3)) >= 3 and parts[1] in steps
            and parts[2] in KEPT]


def check(vanilla, _):
    if not any(line.startswith('Alice step=respawn respawn ') for line in vanilla):
        return 'vanilla sent Alice no respawn: the credits did not roll'
    return None


def main():
    lines = locator.one_player(NAME, SCRIPT, shown, check, 'The respawn after the End credits on a dedicated '
                               'server: the Respawn into the Overworld and the position at the world spawn after it.')
    print(f'credits respawn: {len(lines)} lines identical')


if __name__ == '__main__':
    try:
        main()
    except AssertionError as error:
        sys.exit(f'{NAME}: {error}')
