#!/usr/bin/env python3
"""The packets a Sunrise client sends, tick by tick and field by field, against a real vanilla 26.3 client's
for the same scripted play: an anticheat (Grim, Vulcan, Polar, Watchdog, Intave...) judges the serverbound
stream, also after ViaVersion translated it for an older server, so Sunrise's must be vanilla's.

A vanilla 26.3 dedicated server (flat world, offline) runs behind tools/net_capture.py. Each client joins it in
turn as Steve and plays every scenario below: the harness resets the place and the player from the server's
console, then hands the client the scenario, which it plays tick by tick from its own tick loop (Sunrise:
SUNRISE_TICK_SCRIPT, game/world/tick_script.rs; vanilla: VanillaTickScript.java, the unchanged client with its
keys pressed through KeyMapping). Both send /wire-mark on the scenario's first tick. tools/wire_stream.py
cuts each scenario's stream into client ticks (ClientTickEnd ends one) and decodes the packets; every tick
must hold the same packets, in the same order, with the same bytes (doubles and floats by their bits). In the
replay each tick is one ordered stream: the answers to the server (pong, teleport acceptance, chat acknowledgement)
and the network thread's (keep-alive, chunk batch) keep their place among the tick's packets. Where the network
thread's answers sit, and a chunk batch's measured rate, are not verified when that is all that differs; a scenario
whose delivery ticks the replay can't vouch for is inconclusive, and one a client didn't finish fails. Live, the
answers compare by name and order, and must never fall between two packets of one tick. Neither is an exact pass:
no client reports the tick it handled the server's packets on, so a run without differences is a `match`, a
diagnostic (`verdict`).

Vanilla plays the live server; Sunrise then gets vanilla's server stream replayed on the same scenario ticks
(tools/wire_replay.py), so both clients answered the same server and a difference is the client's own. --live plays
Sunrise against the live server too (two independent 50 ms clocks: the tick an answer lands on can differ from run
to run).

  python3 tools/test_wire_stream_parity.py                      # the DEFAULT set
  python3 tools/test_wire_stream_parity.py --scenarios all      # every scenario
  python3 tools/test_wire_stream_parity.py --scenarios walk,bow --keep
  python3 tools/test_wire_stream_parity.py --clients sunrise,sunrise-legacy   # Legacy (1.8.9) combat feel

The second form plays the same scenarios with two Sunrise clients, the second set to the Legacy combat feel with
every combat extra on (sunrise-extras.txt: `LEGACY_EXTRAS`), and compares their streams the same way: the feel
changes what the player sees and hears, never what the client sends.

Switches, in the environment: WIRE_STREAM_GRIM=DIR (Paper with Grim; WIRE_STREAM_VIA=VERSION an older Paper with
ViaVersion), WIRE_STREAM_LATENCY=MS[~J] (a slow or changing connection), WIRE_STREAM_SERVER_TPS=N,
WIRE_STREAM_PHASE=SECONDS, WIRE_STREAM_OPTIONS=key:value,... (options both clients start with),
WIRE_STREAM_EXTRAS=key:value,... (Sunrise's extras), WIRE_STREAM_RANDOM=SEED,COUNT (generated scenarios),
WIRE_STREAM_RESOURCE_PACK=1, WIRE_STREAM_TRANSFER=1, WIRE_STREAM_WINDOWED=1 (Sunrise in a window). Each is
described where it is read.

Sunrise runs `--headless` (no window or GPU: the ticks are what it sends from); the vanilla client runs on the
virtual screen (`vscreen`) on Linux, from the logged-in session on macOS. Every game and server here runs in fresh temporary folders (SUNRISE_HOME,
SUNRISE_LAUNCHER_HOME, XDG_*), with no audio and no self-update. Needs .local (the vanilla jars and prepared
resources) and a built sunrise-client and sunrise-ctl (--bin DIR, default the parity target's fast build).
"""
import argparse
import itertools
import json
import os
import re
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import threading
import time

sys.path.insert(0, str(Path(__file__).resolve().parent))
import lab  # noqa: E402
import net_capture  # noqa: E402
import test_rust_parity as base  # noqa: E402
import wire_stream  # noqa: E402
from server_pack_e2e import Game  # noqa: E402
from test_dedicated_server_parity import Server, free_port  # noqa: E402

PLAYER = 'Steve'
PROPERTIES = {
    'online-mode': 'false', 'white-list': 'false', 'level-seed': '12345', 'level-type': 'minecraft\\:flat',
    'generate-structures': 'false', 'view-distance': '4', 'simulation-distance': '4', 'spawn-protection': '0',
    'pause-when-empty-seconds': '-1', 'gamemode': 'survival', 'difficulty': 'easy', 'spawn-monsters': 'false',
    'spawn-animals': 'false', 'enable-command-block': 'false',
}
# The flat world's grass is at y -61: the player stands at -60.
FLOOR = -61
# The attributes a scenario sets, reset before every scenario.
ATTRIBUTES = ('movement_speed', 'jump_strength', 'step_height', 'gravity', 'scale', 'safe_fall_distance',
              'block_interaction_range', 'entity_interaction_range')
RESET = [
    'gamerule spawn_mobs false', 'gamerule advance_time false', 'gamerule advance_weather false', 'weather clear',
    f'fill -12 {FLOOR + 1} -12 12 {FLOOR + 12} 28 minecraft:air',
    # The flat world's layers under the grass too: water_swim's pool replaces the bedrock, and the pits of
    # jump_bridge and the clutches take the dirt. The clients play one after the other on one world, so without
    # these Sunrise met what vanilla's run had dug (it fell through water_swim's bedrock in jump_bridge).
    f'fill -12 {FLOOR - 3} -12 12 {FLOOR - 3} 28 minecraft:bedrock',
    f'fill -12 {FLOOR - 2} -12 12 {FLOOR - 1} 28 minecraft:dirt',
    f'fill -12 {FLOOR} -12 12 {FLOOR} 28 minecraft:grass_block',
    # Twice: a killed mob drops its loot and equipment (a saddled pig its saddle) during the first kill, after
    # its selector has run, and the drops would land in the next scenario's hands after their pickup delay.
    'kill @e[type=!minecraft:player]', 'kill @e[type=!minecraft:player]', f'clear {PLAYER}', f'effect clear {PLAYER}', f'gamemode survival {PLAYER}',
    f'effect give {PLAYER} minecraft:saturation 1 20 true', f'effect give {PLAYER} minecraft:instant_health 1 5 true',
    # What a scenario may have changed that the commands above don't undo: server-set attributes, the border.
    *[f'attribute {PLAYER} minecraft:{a} base reset' for a in ATTRIBUTES], 'worldborder set 59999968',
    'tick unfreeze', 'tick rate 20',
    f'tp {PLAYER} 0.5 {FLOOR + 1} 0.5 0 0',
]
# A target that takes hits without moving: no AI, no knockback.
TARGET = ('summon minecraft:husk {x} {y} {z} {{NoAI:1b,Silent:1b,PersistenceRequired:1b,Health:1000f,'
          'attributes:[{{id:"minecraft:max_health",base:1000.0}},{{id:"minecraft:knockback_resistance",base:1.0}}]}}')
# Every scenario starts with a short step forward: it moves the player, which starts LocalPlayer.sendPosition's
# 20-tick reminder over on both clients alike, and selects the first slot.
PRELUDE = 10
# Client ticks between a reset's last teleport and the scenario: the player lands and stands still.
SETTLE = 40
# The Legacy client's sunrise-extras.txt (crates/sunrise-client-render/src/pvp.rs): the 1.8.9 feel, its
# indicator and sounds.
LEGACY_EXTRAS = 'combatFeel:legacy\ncombatIndicator:true\ncombatSounds:true\n'
# The clients --clients may name: the first is the reference, the second must send what it sends.
CLIENTS = {'vanilla': 'vanilla', 'sunrise': 'Sunrise', 'sunrise-legacy': 'Sunrise Legacy'}


def taps(key, ticks):
    return {t: [f'key {key} tap'] for t in ticks}


def merge(*parts):
    out = {}
    for part in parts:
        for tick, lines in part.items():
            out.setdefault(tick, []).extend(lines)
    return out


def every(start, stop, step, *lines):
    return {t: list(lines) for t in range(start, stop, step)}


# A mob's hit on the player, from the player's own command: knockback away from the far husk (LivingEntity.knockback).
KNOCKBACK = 'command damage @s 1 minecraft:mob_attack by @e[tag=far,limit=1]'
# The attacker for KNOCKBACK, far ahead (+z): its hits push the player back (-z).
FAR_HUSK = ('summon minecraft:husk 0.5 {y} 9.0 {{NoAI:1b,Silent:1b,PersistenceRequired:1b,Tags:["far"]}}'
            .format(y=FLOOR + 1))
SWORD = f'item replace entity {PLAYER} weapon.mainhand with minecraft:diamond_sword'
# A husk with its AI, 6 blocks ahead: it walks to the player and attacks (a thousand health, takes knockback).
CHASER = ('summon minecraft:husk 0.5 {y} 6.5 {{PersistenceRequired:1b,Health:1000f,'
          'attributes:[{{id:"minecraft:max_health",base:1000.0}}]}}'.format(y=FLOOR + 1))
# Puts the player 0.3 into a husk at z 2.2, from behind.
OVERLAP = 'command tp @s 0.5 ~ 1.9'
# A smash's fall damage, and a 20-block fall's, must not kill the player mid-scenario (a death screen ends it).
MACE_RESISTANCE = f'effect give {PLAYER} minecraft:resistance 60 4 true'


def mace_fall(height, enchantment=None, effects=(), target_z=5.2):
    """The player stands on a 3x3 stone platform `height` blocks up, walks off its south edge looking down and
    ahead, and swings a mace every third tick on the way down at a target standing below: MaceItem's smash needs
    a fall of more than 1.5 blocks (heavy past 5), and the server answers a hit with the attacker's new motion
    (ClientboundSetEntityMotionPacket), which the client's next moves follow."""
    top = FLOOR + height
    mace = 'minecraft:mace' + (f'[minecraft:enchantments={{{enchantment}}}]' if enchantment else '')
    fall_ticks = 20 + 2 * height
    return dict(
        setup=[f'fill -1 {top} -1 1 {top} 1 minecraft:stone', TARGET.format(x=0.5, y=FLOOR + 1, z=target_z),
               MACE_RESISTANCE, *[f'effect give {PLAYER} {effect} 60 0 true' for effect in effects],
               f'item replace entity {PLAYER} weapon.mainhand with {mace}', f'tp {PLAYER} 0.5 {top + 1} 0.5 0 0'],
        ticks=20 + fall_ticks + 20,
        steps=merge({0: ['look 0 55', 'key w down'], 14: ['key w up']},
                    every(12, 12 + fall_ticks, 3, 'mouse left click')))


# A scenario: `ticks`, `steps` (tick: lines), `setup` (console commands before the settle's teleport), `mount`
# (console commands after it, which must leave the player riding: play_all's mount), `enters` (a dimension the
# scenario must take the player into: a Respawn into it, or the run fails), `anchor` (live comparison only:
# anchored_diffs), `sunrise_steps`.
SCENARIOS = {
    'walk': dict(ticks=70, steps=merge(
        {0: ['key w down'], 20: ['look 30 0'], 30: ['look 30 15'], 40: ['key w up'], 50: ['look 0 0']})),
    'idle': dict(ticks=60, steps={}),
    'sprint_jump': dict(ticks=70, steps={
        0: ['key ctrl down', 'key w down'], 5: ['key space down'], 45: ['key space up'],
        50: ['key w up', 'key ctrl up']}),
    'strafe': dict(ticks=70, steps={
        0: ['key w down', 'key a down'], 12: ['key a up', 'key d down'], 24: ['key d up', 'key a down'],
        36: ['key a up', 'key ctrl down'], 48: ['key w up', 'key ctrl up', 'key s down'], 56: ['key s up']}),
    'w_tap': dict(ticks=70, steps=merge(
        {0: ['key ctrl down', 'key w down']},
        every(10, 50, 8, 'key w up'), every(12, 52, 8, 'key w down'),
        {55: ['key w up', 'key ctrl up']})),
    'sneak': dict(ticks=60, steps={
        0: ['key shift down'], 5: ['key w down'], 25: ['key shift up'], 30: ['key shift down'],
        40: ['key w up'], 45: ['key shift up']}),
    'hit_target': dict(
        setup=[TARGET.format(x=0.5, y=FLOOR + 1, z=3.0), f'item replace entity {PLAYER} weapon.mainhand with '
               'minecraft:diamond_sword'],
        ticks=70, steps=merge({0: ['look 0 20']}, every(5, 65, 12, 'mouse left click'))),
    'sprint_combo': dict(
        setup=[TARGET.format(x=0.5, y=FLOOR + 1, z=7.0), f'item replace entity {PLAYER} weapon.mainhand with '
               'minecraft:diamond_sword'],
        ticks=60, steps=merge(
            {0: ['look 0 10', 'key ctrl down', 'key w down']}, every(12, 52, 10, 'mouse left click'),
            every(14, 54, 10, 'key w up'), every(16, 56, 10, 'key w down'), {56: ['key w up', 'key ctrl up']})),
    'block_hit': dict(
        setup=[TARGET.format(x=0.5, y=FLOOR + 1, z=3.0),
               f'item replace entity {PLAYER} weapon.mainhand with minecraft:diamond_sword',
               f'item replace entity {PLAYER} weapon.offhand with minecraft:shield'],
        ticks=70, steps={0: ['look 0 20'], 5: ['mouse left click'], 8: ['mouse right down'], 20: ['mouse right up'],
                         22: ['mouse left click'], 25: ['mouse right down'], 30: ['mouse left click'],
                         40: ['mouse right up'], 50: ['mouse left click']}),
    'rod': dict(
        setup=[TARGET.format(x=0.5, y=FLOOR + 1, z=6.0),
               f'item replace entity {PLAYER} weapon.mainhand with minecraft:fishing_rod'],
        ticks=80, steps={0: ['look 0 -5'], 5: ['mouse right click'], 35: ['mouse right click'],
                         45: ['mouse right click'], 70: ['mouse right click']}),
    'bow': dict(
        setup=[f'item replace entity {PLAYER} weapon.mainhand with minecraft:bow',
               f'give {PLAYER} minecraft:arrow 16'],
        ticks=80, steps={0: ['look 0 -10'], 5: ['mouse right down'], 30: ['mouse right up'],
                         40: ['mouse right down'], 45: ['mouse right up'], 55: ['mouse right down'],
                         75: ['mouse right up']}),
    'eat': dict(
        setup=[f'item replace entity {PLAYER} weapon.mainhand with minecraft:golden_apple 4'],
        # Two bites of 10 ticks, a third of the 32 a golden apple takes: when a full one ends is the server's to
        # say (its entity event), and a loaded server that catches up runs its ticks in a burst.
        ticks=50, steps={5: ['mouse right down'], 15: ['mouse right up'], 25: ['mouse right down'],
                         35: ['mouse right up']}),
    'bridge': dict(
        setup=[f'fill -2 {FLOOR + 1} -2 2 {FLOOR + 2} 2 minecraft:stone', f'tp {PLAYER} 0.5 {FLOOR + 3} 2.2 180 78',
               f'item replace entity {PLAYER} weapon.mainhand with minecraft:stone 64'],
        ticks=90, steps=merge({0: ['look 0 78', 'key shift down', 'key s down']}, every(4, 80, 4, 'mouse right click'),
                              {82: ['key s up', 'key shift up']})),
    'inventory': dict(
        setup=[f'item replace entity {PLAYER} hotbar.0 with minecraft:diamond_sword',
               f'item replace entity {PLAYER} hotbar.1 with minecraft:cooked_beef 16',
               f'item replace entity {PLAYER} inventory.0 with minecraft:golden_apple 8'],
        ticks=60, steps={5: ['key e tap'], 10: ['click-slot 36'], 14: ['click-slot 9'], 18: ['click-slot 36'],
                         22: ['click-slot 37 shift'], 26: ['click-slot 10 right'], 30: ['key e tap'],
                         40: ['key e tap'], 45: ['key esc tap']}),
    'hotbar_swap': dict(
        setup=[TARGET.format(x=0.5, y=FLOOR + 1, z=3.0)] +
              [f'item replace entity {PLAYER} hotbar.{i} with minecraft:{item}' for i, item in
               enumerate(['diamond_sword', 'stone_axe', 'golden_apple', 'stone', 'bow'])],
        # 52-54: a swing at the sky with a new slot sends no slot before its Punch (no ensureHasSentCarriedItem
        # on a miss); the next tick's MultiPlayerGameMode.tick does.
        ticks=60, steps=merge({0: ['look 0 20']}, {t: [f'key {t % 5 + 1} tap'] for t in range(5, 15)},
                              {16: ['key 2 tap', 'mouse left click'], 20: ['key 1 tap'], 21: ['key 3 tap'],
                               30: ['key 5 tap', 'mouse left click'], 35: ['key q tap'], 40: ['key f tap'],
                               52: ['look 0 -80'], 54: ['key 3 tap', 'mouse left click']})),
    'dig_teleport': dict(
        setup=[f'setblock 0 {FLOOR + 1} 2 minecraft:stone'],
        ticks=60, anchor='accept_teleportation',
        steps={0: ['look 0 45'], 3: ['mouse left down'], 15: ['command tp @s ~ ~ ~'], 40: ['mouse left up']}),
    # A mace on the ground: no fall, so no smash; one hit at full charge and one early.
    'mace_ground': dict(
        setup=[TARGET.format(x=0.5, y=FLOOR + 1, z=3.0), f'item replace entity {PLAYER} weapon.mainhand with '
               'minecraft:mace'],
        ticks=80, steps={0: ['look 0 20'], 5: ['mouse left click'], 20: ['mouse left click'],
                         60: ['mouse left click']}),
    **{f'mace_fall_{height}': mace_fall(height) for height in (3, 5, 10, 20)},
    # Each of the mace's own enchantments on a 5-block smash; Wind Burst launches the attacker back up.
    **{f'mace_{name}': mace_fall(5, enchantment=f'"minecraft:{name}":{level}')
       for name, level in (('density', 5), ('breach', 4), ('wind_burst', 3))},
    # A smash from 10 blocks under Slow Falling, whose fall the server still counts.
    'mace_slow_falling': mace_fall(10, effects=['minecraft:slow_falling']),
    # The first demo's geometry: the player lands past the target, still sliding, with no key held.
    'mace_fall_5_overshoot': mace_fall(5, target_z=3.4),
    # Falling past the target and swinging at the air: the misses' 10-tick delay, no hit.
    'mace_fall_miss': mace_fall(10, target_z=8.0),
    # Knockback while overlapping a mob. The player is teleported 0.3 into a husk in front of it (z 1.9 against
    # 2.2), whose pushEntities then pushes it back each tick, after the player's own tick; two ticks later a second
    # husk far ahead hits it, so the knockback and the push both point back and nothing blocks them. The server's
    # motion replaces the player's velocity, push included (ClientPacketListener.handleSetEntityMotion).
    'push_knockback': dict(
        setup=[TARGET.format(x=0.5, y=FLOOR + 1, z=2.2), FAR_HUSK],
        ticks=60, steps=merge({t: [OVERLAP] for t in (10, 25, 40)}, {t: [KNOCKBACK] for t in (12, 27, 42)})),
    # The same teleports and hits with no husk to overlap: the control for push_knockback.
    'push_knockback_control': dict(
        setup=[FAR_HUSK],
        ticks=60, steps=merge({t: [OVERLAP] for t in (10, 25, 40)}, {t: [KNOCKBACK] for t in (12, 27, 42)})),
    # A teleport onto itself two ticks after overlapping the husk: the teleport's velocity reset and the push.
    'push_teleport': dict(
        setup=[TARGET.format(x=0.5, y=FLOOR + 1, z=2.2)],
        ticks=60, steps={10: [OVERLAP], 12: ['command tp @s ~ ~ ~'], 30: [OVERLAP], 32: ['command tp @s ~ ~ ~']}),
    # Knockback from the far husk in different states, no mob in reach: what the server's motion meets.
    'knockback_sprint': dict(
        setup=[FAR_HUSK],
        ticks=60, steps=merge({0: ['key ctrl down', 'key w down'], 40: ['key w up', 'key ctrl up']},
                              {t: [KNOCKBACK] for t in (10, 25)})),
    'knockback_air': dict(
        setup=[FAR_HUSK],
        ticks=60, steps={5: ['key space down'], 7: ['key space up', KNOCKBACK], 30: ['key space down'],
                         33: ['key space up', KNOCKBACK]}),
    'knockback_sneak': dict(
        setup=[FAR_HUSK],
        ticks=60, steps={0: ['key shift down', 'key s down'], 15: [KNOCKBACK], 30: [KNOCKBACK],
                         45: ['key s up', 'key shift up']}),
    # No sprint at 6 food or less (LocalPlayer.hasEnoughFoodToSprint): Hunger 255 empties the bar at once.
    'hunger_sprint': dict(
        setup=[f'effect give {PLAYER} minecraft:hunger 3 255 true'],
        ticks=60, steps={5: ['key ctrl down', 'key w down'], 40: ['key w up', 'key ctrl up']}),
    # Opening the inventory while walking: Minecraft.setScreen releases every key (KeyMapping.releaseAll), so the
    # player stops though W was never let go; it walks again only after W is pressed anew.
    'inventory_walk': dict(
        ticks=60, steps={0: ['key w down'], 10: ['key e tap'], 30: ['key e tap'], 40: ['key w down'],
                         50: ['key w up']}),
    # Vehicles: the rider's client moves the vehicle itself (ServerboundMoveVehiclePacket, the
    # boat's paddles, PlayerInput), so a vehicle-predicting anticheat checks these as closely as walking. Each
    # mounts by command on tick 0, after the prelude, except boat_mount_click.
    'boat_paddle': dict(
        setup=[f'fill -5 {FLOOR} 1 5 {FLOOR} 26 minecraft:water', f'summon minecraft:oak_boat 0.5 {FLOOR + 1} 3.5'],
        ticks=110, steps=merge({0: ['command ride @s mount @e[type=minecraft:oak_boat,limit=1]'],
                                8: ['key w down'], 30: ['key a down'], 40: ['key a up'], 50: ['key d down'],
                                58: ['key d up'], 62: ['look 120 10'], 70: ['look 200 10'], 76: ['look -150 10'],
                                80: ['key w up', 'key s down'], 92: ['key s up'], 100: ['key shift down'], 104: ['key shift up']})),
    'boat_land': dict(
        setup=[f'summon minecraft:oak_boat 0.5 {FLOOR + 1} 3.5'],
        ticks=70, steps={0: ['command ride @s mount @e[type=minecraft:oak_boat,limit=1]'], 6: ['key w down'],
                         20: ['key a down'], 30: ['key a up'], 40: ['key w up'], 55: ['key shift down'], 59: ['key shift up']}),
    'boat_mount_click': dict(
        setup=[f'summon minecraft:oak_boat 0.5 {FLOOR + 1} 2.5'],
        ticks=60, steps={0: ['look 0 40'], 5: ['mouse right click'], 12: ['key w down'], 25: ['key w up'],
                         35: ['key shift down'], 39: ['key shift up'], 45: ['key w down'], 50: ['key w up']}),
    'minecart_rails': dict(
        setup=[f'fill 0 {FLOOR} 4 0 {FLOOR} 7 minecraft:redstone_block',
               f'fill 0 {FLOOR + 1} 1 0 {FLOOR + 1} 24 minecraft:rail[shape=north_south]',
               f'fill 0 {FLOOR + 1} 4 0 {FLOOR + 1} 7 minecraft:powered_rail[shape=north_south,powered=true]',
               f'summon minecraft:minecart 0.5 {FLOOR + 1} 2.5'],
        ticks=90, steps={0: ['command ride @s mount @e[type=minecraft:minecart,limit=1]'], 5: ['key w down'],
                         25: ['key w up'], 40: ['look 90 0'], 60: ['key s down'], 70: ['key s up'],
                         80: ['key shift down'], 84: ['key shift up']}),
    'horse_ride': dict(
        setup=[f'summon minecraft:horse 0.5 {FLOOR + 1} 2.5 {{Tame:1b,PersistenceRequired:1b,Rotation:[0f,0f],Variant:0,'
               'equipment:{saddle:{id:"minecraft:saddle",count:1}},attributes:[{id:"minecraft:movement_speed",'
               'base:0.3},{id:"minecraft:jump_strength",base:0.8},{id:"minecraft:max_health",base:30.0}]}'],
        ticks=110, steps={0: ['command ride @s mount @e[type=minecraft:horse,limit=1]'], 6: ['key w down'],
                          20: ['key space down'], 30: ['key space up'], 45: ['key a down'], 55: ['key a up'],
                          60: ['key ctrl down'], 62: ['key space down'], 64: ['key space up'], 80: ['key ctrl up'],
                          85: ['key w up'], 100: ['key shift down'], 104: ['key shift up']}),
    # Turned to 0 and mounted after the settle: a camel left alone turns its head at random before the player
    # mounts, and a rider steering it takes its head's yaw, so the two clients dashed different ways.
    'camel_dash': dict(
        setup=[f'summon minecraft:camel 0.5 {FLOOR + 1} 2.5 {{PersistenceRequired:1b,Tame:1b,Rotation:[0f,0f],'
               'equipment:{saddle:{id:"minecraft:saddle",count:1}}}'],
        mount=['rotate @e[type=minecraft:camel,limit=1] 0 0', f'ride {PLAYER} mount @e[type=minecraft:camel,limit=1]'],
        ticks=120, steps={10: ['key w down'],
                          30: ['key space down'], 34: ['key space up'], 60: ['key ctrl down'], 70: ['key ctrl up'], 85: ['key w up'],
                          110: ['key shift down'], 114: ['key shift up']}),
    # Turned to 0 and mounted after the settle: left alone, the mob turned at random before the mount, so the two
    # clients' rides differed.
    'pig_carrot': dict(
        setup=[f'summon minecraft:pig 0.5 {FLOOR + 1} 2.5 {{PersistenceRequired:1b,Rotation:[0f,0f],'
               'equipment:{saddle:{id:"minecraft:saddle",count:1}}}',
               f'item replace entity {PLAYER} weapon.mainhand with minecraft:carrot_on_a_stick'],
        mount=['rotate @e[type=minecraft:pig,limit=1] 0 0', f'ride {PLAYER} mount @e[type=minecraft:pig,limit=1]'],
        ticks=90, steps={10: ['look 30 10'],
                         30: ['mouse right click'], 50: ['look -20 10'], 75: ['key shift down'], 79: ['key shift up']}),
    # Mounting turns the rider (AbstractHorse.addPassenger's absSnapRotationTo; a boat's yaw when boarding): the
    # boat and the horse face 123 degrees, the player 0, so the rider's first look and the vehicle's moves show it.
    'boat_turned': dict(
        setup=[f'fill -5 {FLOOR} 1 5 {FLOOR} 26 minecraft:water',
               f'summon minecraft:oak_boat 0.5 {FLOOR + 1} 3.5 {{Rotation:[123.0f,0.0f]}}'],
        ticks=50, steps={0: ['command ride @s mount @e[type=minecraft:oak_boat,limit=1]'], 8: ['key w down'],
                         30: ['key w up'], 40: ['key shift down'], 44: ['key shift up']}),
    'horse_turned': dict(
        setup=[f'summon minecraft:horse 0.5 {FLOOR + 1} 2.5 {{Tame:1b,PersistenceRequired:1b,Variant:0,'
               'Rotation:[123.0f,10.0f],equipment:{saddle:{id:"minecraft:saddle",count:1}},'
               'attributes:[{id:"minecraft:movement_speed",base:0.3}]}'],
        ticks=50, steps={0: ['command ride @s mount @e[type=minecraft:horse,limit=1]'], 8: ['key w down'],
                         30: ['key w up'], 40: ['key shift down'], 44: ['key shift up']}),
    # Server-set attributes: the client's prediction must use the server's values.
    'attr_speed': dict(
        setup=[f'attribute {PLAYER} minecraft:movement_speed base set 0.17'],
        ticks=60, steps={0: ['key w down'], 15: ['key ctrl down'], 20: ['key space down'], 30: ['key space up'],
                         45: ['key w up', 'key ctrl up']}),
    'attr_jump': dict(
        setup=[f'attribute {PLAYER} minecraft:jump_strength base set 0.75'],
        ticks=60, steps={5: ['key space down'], 8: ['key space up'], 30: ['key w down', 'key space down'],
                         34: ['key space up'], 45: ['key w up']}),
    'attr_step': dict(
        setup=[f'attribute {PLAYER} minecraft:step_height base set 1.25',
               f'fill -2 {FLOOR + 1} 3 2 {FLOOR + 1} 4 minecraft:stone', f'fill -2 {FLOOR + 2} 5 2 {FLOOR + 2} 6 minecraft:stone'],
        ticks=50, steps={0: ['key w down'], 40: ['key w up']}),
    'attr_gravity': dict(
        setup=[f'attribute {PLAYER} minecraft:gravity base set 0.03'],
        ticks=70, steps={5: ['key space down'], 8: ['key space up'], 10: ['key w down'], 50: ['key w up']}),
    'attr_scale': dict(
        setup=[f'attribute {PLAYER} minecraft:scale base set 1.6'],
        ticks=60, steps={0: ['key w down'], 20: ['key space down'], 24: ['key space up'], 30: ['key shift down'],
                         45: ['key shift up', 'key w up']}),
    'attr_safe_fall': dict(
        setup=[f'attribute {PLAYER} minecraft:safe_fall_distance base set 6'],
        ticks=50, anchor='accept_teleportation', steps={0: ['command tp @s 0.5 ~8 0.5']}),
    'attr_reach': dict(
        setup=[f'attribute {PLAYER} minecraft:block_interaction_range base set 2.0',
               f'attribute {PLAYER} minecraft:entity_interaction_range base set 2.0',
               TARGET.format(x=0.5, y=FLOOR + 1, z=3.2), f'setblock 0 {FLOOR + 1} 2 minecraft:stone', SWORD],
        ticks=50, steps={0: ['look 0 40'], 5: ['mouse left click'], 15: ['look 0 15'], 20: ['mouse left click'],
                         30: ['mouse right click']}),
    # Walking into a world border five blocks wide, around the start.
    'world_border': dict(
        setup=['worldborder center 0.5 0.5', 'worldborder set 5'],
        ticks=60, steps={0: ['key w down', 'key ctrl down'], 25: ['key d down'], 40: ['key w up', 'key ctrl up',
                         'key d up']}),
    # Enchantments that change movement.
    'depth_strider': dict(
        setup=[f'fill -3 {FLOOR} 1 3 {FLOOR + 1} 14 minecraft:water',
               f'item replace entity {PLAYER} armor.feet with minecraft:diamond_boots[minecraft:enchantments={{"minecraft:depth_strider":3}}]'],
        ticks=60, steps={0: ['key w down'], 50: ['key w up']}),
    'soul_speed': dict(
        setup=[f'fill -1 {FLOOR} 1 1 {FLOOR} 14 minecraft:soul_sand',
               f'item replace entity {PLAYER} armor.feet with minecraft:diamond_boots[minecraft:enchantments={{"minecraft:soul_speed":3}}]'],
        ticks=60, steps={0: ['key w down'], 20: ['key ctrl down'], 50: ['key w up', 'key ctrl up']}),
    'swift_sneak': dict(
        setup=[f'item replace entity {PLAYER} armor.legs with minecraft:diamond_leggings[minecraft:enchantments={{"minecraft:swift_sneak":3}}]'],
        ticks=50, steps={0: ['key shift down', 'key w down'], 40: ['key w up', 'key shift up']}),
    'frost_walker': dict(
        setup=[f'fill -3 {FLOOR} 2 3 {FLOOR} 14 minecraft:water',
               f'item replace entity {PLAYER} armor.feet with minecraft:diamond_boots[minecraft:enchantments={{"minecraft:frost_walker":2}}]'],
        ticks=60, steps={0: ['key w down'], 50: ['key w up']}),
    # Game modes: adventure can neither break nor place here; creative double-taps jump to
    # fly (ServerboundPlayerAbilities), sprint-flies, and lands; spectator flies through blocks.
    'adventure_mode': dict(
        setup=[f'gamemode adventure {PLAYER}', f'setblock 0 {FLOOR + 1} 2 minecraft:stone',
               f'item replace entity {PLAYER} weapon.mainhand with minecraft:stone 8'],
        ticks=50, steps={0: ['look 0 45'], 5: ['mouse left down'], 20: ['mouse left up'], 25: ['mouse right click'],
                         30: ['look 0 70'], 35: ['mouse right click']}),
    'creative_flight': dict(
        setup=[f'gamemode creative {PLAYER}'],
        ticks=90, steps=merge({5: ['key space down'], 6: ['key space up'], 8: ['key space down'], 9: ['key space up']}, {12: ['key space down'], 20: ['key space up'],
                               22: ['key w down'], 30: ['key ctrl down'], 50: ['key ctrl up', 'key shift down'],
                               58: ['key shift up'], 60: ['key w up']}, {70: ['key space down'], 71: ['key space up'], 73: ['key space down'], 74: ['key space up']})),
    'spectator_flight': dict(
        setup=[f'gamemode spectator {PLAYER}', f'fill -2 {FLOOR + 1} 3 2 {FLOOR + 3} 5 minecraft:stone'],
        ticks=60, steps={0: ['key w down'], 20: ['key space down'], 28: ['key space up'], 35: ['key ctrl down'],
                         50: ['key w up', 'key ctrl up']}),
    # Chat on this offline-mode server: unsigned chat, and commands typed into chat, among them
    # ones with a message argument (signed on an online server) and one that fails to parse.
    'chat_plain': dict(
        ticks=60, steps={5: ['chat hello world'], 25: ['chat a second message'], 45: ['chat   spaced   out   ']}),
    'chat_commands': dict(
        ticks=70, steps={5: ['chat /me waves'], 20: ['chat /msg Steve hi there'], 35: ['chat /say hello'],
                         50: ['chat /notacommand at all']}),
    # Client detection: an item named by translatable components goes into an anvil. AnvilScreen's
    # slotChanged sets the name box to the name as this client resolves it (Component.getString), and the box's
    # responder sends it (ServerboundRenameItemPacket). A key only Sunrise knows must come back as the key, as
    # vanilla's does; a sign can't be the probe here (SignBlock won't open an editor on non-plain lines).
    'anvil_probe': dict(
        setup=[f'setblock 0 {FLOOR + 1} 2 minecraft:anvil',
               f'item replace entity {PLAYER} hotbar.0 with minecraft:paper[minecraft:custom_name='
               '{translate:"sunrise.agent.off"}]',
               f'item replace entity {PLAYER} hotbar.1 with minecraft:paper[minecraft:custom_name='
               '{translate:"block.minecraft.stone"}]',
               f'item replace entity {PLAYER} hotbar.2 with minecraft:paper[minecraft:custom_name='
               '{translate:"no.such.key",fallback:"fallback text"}]',
               f'experience add {PLAYER} 10 levels'],
        ticks=50, steps={0: ['look 0 45'], 5: ['mouse right click'], 12: ['click-slot 30 shift'],
                         16: ['click-slot 0 shift'], 20: ['click-slot 31 shift'], 24: ['click-slot 0 shift'],
                         28: ['click-slot 32 shift'], 32: ['click-slot 0 shift'], 40: ['key e tap']}),
    # Two minutes of standing and walking: the packets of each of its ticks. The harness doesn't measure the
    # ticks against the wall clock.
    'timing_long': dict(
        ticks=2400, steps=merge(every(100, 2300, 200, 'key w down'), every(140, 2340, 200, 'key w up'),
                                every(160, 2360, 200, 'key s down'), every(190, 2390, 200, 'key s up'))),
    # Using blocks and tools on blocks: each right click's use_item_on and the client's
    # prediction of the result (an opened door, a stripped log, farmland, a path, water placed and taken back).
    'door_lever_button': dict(
        setup=[f'setblock 0 {FLOOR + 1} 2 minecraft:oak_door[half=lower,facing=north]',
               f'setblock 0 {FLOOR + 2} 2 minecraft:oak_door[half=upper,facing=north]',
               f'setblock 1 {FLOOR + 1} 2 minecraft:lever[face=floor,facing=north]',
               f'setblock -1 {FLOOR + 1} 2 minecraft:stone_button[face=floor,facing=north]'],
        ticks=60, steps={0: ['look 0 20'], 5: ['mouse right click'], 15: ['mouse right click'],
                         20: ['look -30 40'], 25: ['mouse right click'], 35: ['look 30 40'],
                         40: ['mouse right click']}),
    'axe_strip': dict(
        setup=[f'setblock 0 {FLOOR + 1} 2 minecraft:oak_log',
               f'item replace entity {PLAYER} weapon.mainhand with minecraft:iron_axe'],
        ticks=40, steps={0: ['look 0 30'], 5: ['mouse right click'], 15: ['mouse right click']}),
    'hoe_shovel': dict(
        setup=[f'item replace entity {PLAYER} hotbar.0 with minecraft:iron_hoe',
               f'item replace entity {PLAYER} hotbar.1 with minecraft:iron_shovel'],
        ticks=50, steps={0: ['look 0 60'], 5: ['mouse right click'], 15: ['look 20 60', 'key 2 tap'],
                         20: ['mouse right click']}),
    'bone_meal_bucket': dict(
        setup=[f'item replace entity {PLAYER} hotbar.0 with minecraft:bone_meal 4',
               f'item replace entity {PLAYER} hotbar.1 with minecraft:water_bucket'],
        ticks=60, steps={0: ['look 0 60'], 5: ['mouse right click'], 15: ['look 40 60', 'key 2 tap'],
                         20: ['mouse right click'], 35: ['mouse right click']}),
    'flint_and_steel': dict(
        setup=[f'setblock 0 {FLOOR + 1} 2 minecraft:netherrack',
               f'item replace entity {PLAYER} weapon.mainhand with minecraft:flint_and_steel'],
        ticks=40, steps={0: ['look 0 25'], 5: ['mouse right click'], 20: ['mouse left click']}),
    # Pick block: a middle click asks the server for the block's item (ServerboundPickItemFromBlock),
    # in survival with it in the inventory and in creative.
    'pick_block': dict(
        setup=[f'setblock 0 {FLOOR + 1} 2 minecraft:oak_log',
               f'item replace entity {PLAYER} inventory.3 with minecraft:oak_log 5'],
        ticks=50, steps={0: ['look 0 30'], 5: ['mouse middle click'], 15: ['command gamemode creative'],
                         25: ['look 0 70'], 30: ['mouse middle click']}),
    # Entities used with items: an item frame filled, turned and emptied; a spawn egg; a name
    # tag and a lead on a cow.
    'item_frame': dict(
        setup=[f'setblock 0 {FLOOR + 1} 3 minecraft:stone',
               f'summon minecraft:item_frame 0 {FLOOR + 1} 2 {{Facing:2b}}',
               f'item replace entity {PLAYER} weapon.mainhand with minecraft:apple 3'],
        ticks=50, steps={0: ['look 0 25'], 5: ['mouse right click'], 15: ['mouse right click'],
                         25: ['mouse right click'], 35: ['mouse left click']}),
    'spawn_egg_name_lead': dict(
        setup=[f'summon minecraft:cow 0.5 {FLOOR + 1} 2.5 {{NoAI:1b,PersistenceRequired:1b,Rotation:[180f,0f]}}',
               f'item replace entity {PLAYER} hotbar.0 with minecraft:name_tag[minecraft:custom_name="Bob"]',
               f'item replace entity {PLAYER} hotbar.1 with minecraft:lead',
               f'item replace entity {PLAYER} hotbar.2 with minecraft:pig_spawn_egg'],
        ticks=60, steps={0: ['look 0 25'], 5: ['mouse right click'], 15: ['key 2 tap'], 20: ['mouse right click'],
                         30: ['key 3 tap', 'look 60 60'], 35: ['mouse right click']}),
    # Bundles: items clicked into a bundle in the inventory, and one taken out.
    'bundle_clicks': dict(
        setup=[f'item replace entity {PLAYER} hotbar.0 with minecraft:bundle',
               f'item replace entity {PLAYER} inventory.0 with minecraft:stone 10',
               f'item replace entity {PLAYER} inventory.1 with minecraft:apple 5'],
        ticks=50, steps={5: ['key e tap'], 10: ['click-slot 9'], 13: ['click-slot 36 right'],
                         16: ['click-slot 10'], 19: ['click-slot 36 right'], 22: ['click-slot 36 right'],
                         25: ['click-slot 11'], 30: ['key e tap']}),
    # The server changes things mid-action: effects and an attribute arrive while sprinting and in the
    # middle of a jump.
    'mid_action_changes': dict(
        ticks=70, steps={0: ['key w down', 'key ctrl down'], 10: ['command effect give @s minecraft:speed 30 2'],
                         20: ['key space down'], 22: ['command effect give @s minecraft:jump_boost 30 2'],
                         24: ['key space up'], 35: ['command attribute @s minecraft:movement_speed base set 0.14'],
                         45: ['command effect clear @s'], 55: ['key w up', 'key ctrl up']}),
    # The server's tick rate: a server's client follows a slower rate (its tick target only ever grows,
    # ClientboundTickingStatePacket), and keeps moving while the server is frozen.
    'tick_rate_slow': dict(
        ticks=90, steps={5: ['command tick rate 10'], 15: ['key w down'], 40: ['key w up'], 45: ['key space down'],
                         48: ['key space up'], 60: ['command tick rate 20'], 70: ['key w down'], 80: ['key w up']}),
    'tick_freeze': dict(
        ticks=70, steps={5: ['command tick freeze'], 10: ['key w down'], 25: ['key w up'], 30: ['key space down'],
                         33: ['key space up'], 45: ['command tick unfreeze'], 50: ['key w down'], 60: ['key w up']}),
    # Blocks that hurt and slow: walking into a cactus, over magma, into a campfire.
    'damaging_blocks': dict(
        setup=[f'setblock 0 {FLOOR + 1} 2 minecraft:cactus', f'fill -2 {FLOOR} 3 2 {FLOOR} 5 minecraft:magma_block',
               f'setblock 1 {FLOOR + 1} 6 minecraft:campfire'],
        ticks=80, steps={0: ['key w down'], 20: ['key w up', 'key d down'], 26: ['key d up', 'key w down'],
                         50: ['look 30 0'], 70: ['key w up']}),
    # More movement niches.
    'lava_fire_resistance': dict(
        setup=[f'effect give {PLAYER} minecraft:fire_resistance 60 0 true',
               f'fill -2 {FLOOR} 2 2 {FLOOR} 8 minecraft:lava'],
        ticks=80, steps={0: ['key w down'], 30: ['key space down'], 45: ['key space up'], 60: ['key w up']}),
    'riptide': dict(
        setup=[f'fill -3 {FLOOR - 2} 1 3 {FLOOR} 10 minecraft:water',
               f'item replace entity {PLAYER} weapon.mainhand with minecraft:trident[minecraft:enchantments={{"minecraft:riptide":3}}]',
               f'tp {PLAYER} 0.5 {FLOOR - 1} 2.5 0 0'],
        ticks=70, steps={0: ['look 0 -20'], 5: ['mouse right down'], 20: ['mouse right up'], 50: ['key w down'],
                         60: ['key w up']}),
    'wind_charge_jump': dict(
        setup=[f'item replace entity {PLAYER} weapon.mainhand with minecraft:wind_charge 4'],
        ticks=70, steps={0: ['look 0 90'], 5: ['key space down'], 8: ['key space up'], 9: ['mouse right click'],
                         40: ['key w down'], 45: ['key space down'], 47: ['key space up', 'mouse right click'],
                         60: ['key w up']}),
    'bubble_columns': dict(
        setup=[f'fill 0 {FLOOR} 2 0 {FLOOR} 2 minecraft:soul_sand', f'fill 0 {FLOOR + 1} 2 0 {FLOOR + 5} 2 minecraft:water',
               f'fill 0 {FLOOR} 4 0 {FLOOR} 4 minecraft:magma_block', f'fill 0 {FLOOR + 1} 4 0 {FLOOR + 5} 4 minecraft:water',
               'tick freeze', 'tick unfreeze'],
        ticks=100, steps={0: ['key w down'], 5: ['key w up'], 40: ['key w down'], 46: ['key w up']}),
    'chorus_fruit': dict(
        setup=[f'item replace entity {PLAYER} weapon.mainhand with minecraft:chorus_fruit 2',
               f'effect give {PLAYER} minecraft:hunger 2 255 true'],
        ticks=80, anchor='accept_teleportation', steps={5: ['mouse right down'], 45: ['mouse right up']}),
    'unloaded_chunks': dict(
        ticks=120, anchor='accept_teleportation', steps={5: ['command tp @s 30000.5 -60 30000.5'], 10: ['key w down'],
                                                         60: ['key w up'], 80: ['command tp @s 0.5 -60 0.5']}),
    # Knockback in more situations: on each surface, against a wall, at an edge, with knockback
    # resistance, behind a raised shield, from an explosion and a snowball, in a boat, under levitation, and two
    # in one tick. The server's velocity reaches both clients on the same tick through the replay.
    **{f'kb_on_{name}': dict(
        setup=[FAR_HUSK, f'fill -2 {FLOOR} -2 2 {FLOOR} 6 minecraft:{block}'],
        ticks=60, steps=merge({0: ['key w down'], 30: ['key w up']}, {t: [KNOCKBACK] for t in (12, 35)}))
       for name, block in (('ice', 'packed_ice'), ('slime', 'slime_block'), ('soul_sand', 'soul_sand'))},
    'kb_in_water': dict(
        setup=[FAR_HUSK, f'fill -2 {FLOOR + 1} -2 2 {FLOOR + 1} 6 minecraft:water'],
        ticks=60, steps=merge({0: ['key w down'], 30: ['key w up']}, {t: [KNOCKBACK] for t in (12, 35)})),
    'kb_on_ladder': dict(
        setup=[FAR_HUSK, f'fill 0 {FLOOR + 1} -1 0 {FLOOR + 4} -1 minecraft:stone',
               f'fill 0 {FLOOR + 1} 0 0 {FLOOR + 4} 0 minecraft:ladder[facing=south]'],
        ticks=60, steps=merge({0: ['look 180 0', 'key w down'], 25: ['key w up', 'key shift down'],
                               45: ['key shift up']}, {t: [KNOCKBACK] for t in (15, 30)})),
    'kb_against_wall': dict(
        setup=[FAR_HUSK, f'fill -2 {FLOOR + 1} -1 2 {FLOOR + 3} -1 minecraft:stone'],
        ticks=50, steps={10: [KNOCKBACK], 25: [KNOCKBACK]}),
    'kb_at_edge': dict(
        setup=[FAR_HUSK, f'fill -2 {FLOOR + 1} -1 2 {FLOOR + 1} 1 minecraft:stone', f'tp {PLAYER} 0.5 {FLOOR + 2} 0.5 0 0'],
        ticks=50, steps={5: ['key shift down'], 10: [KNOCKBACK], 30: ['key shift up', KNOCKBACK]}),
    'kb_resistance': dict(
        setup=[FAR_HUSK, f'item replace entity {PLAYER} armor.chest with minecraft:netherite_chestplate',
               f'item replace entity {PLAYER} armor.legs with minecraft:netherite_leggings'],
        ticks=50, steps={10: [KNOCKBACK], 30: ['key w down', KNOCKBACK], 40: ['key w up']}),
    'kb_shield': dict(
        setup=[FAR_HUSK, f'item replace entity {PLAYER} weapon.offhand with minecraft:shield'],
        ticks=60, steps={5: ['mouse right down'], 15: [KNOCKBACK], 30: ['mouse right up'], 40: [KNOCKBACK]}),
    'kb_creeper': dict(
        setup=[f'effect give {PLAYER} minecraft:resistance 20 4 true'],
        ticks=60, steps={0: [f'command summon minecraft:creeper 0.5 {FLOOR + 1} 4.5 '
                             '{NoAI:1b,Fuse:15s,ignited:1b,ExplosionRadius:1b}', 'key w down'], 20: ['key w up']}),
    'kb_snowball': dict(
        setup=[f'effect give {PLAYER} minecraft:resistance 20 4 true'],
        ticks=75, steps={10: ['command summon minecraft:snowball 0.5 -59 4.0 {Motion:[0.0,0.0,-1.2]}'],
                         30: ['command summon minecraft:egg 0.8 -59 4.0 {Motion:[0.0,0.0,-1.2]}'],
                         50: ['command summon minecraft:arrow 0.5 -59 4.0 {Motion:[0.0,0.0,-1.2],damage:0.5d}']}),
    'kb_in_boat': dict(
        setup=[FAR_HUSK, f'summon minecraft:oak_boat 0.5 {FLOOR + 1} 2.5'],
        ticks=60, steps={0: ['command ride @s mount @e[type=minecraft:oak_boat,limit=1]'], 15: [KNOCKBACK],
                         30: ['key w down'], 35: [KNOCKBACK], 45: ['key w up'], 50: ['key shift down'],
                         54: ['key shift up']}),
    'kb_levitation': dict(
        setup=[FAR_HUSK],
        ticks=80, steps={0: [f'command effect give {PLAYER} minecraft:levitation 3 1 true'], 10: [KNOCKBACK],
                         30: [KNOCKBACK]}),
    'kb_twice_one_tick': dict(
        setup=[FAR_HUSK],
        ticks=50, steps={10: [KNOCKBACK, KNOCKBACK], 30: ['key w down', KNOCKBACK, KNOCKBACK], 40: ['key w up']}),
    # Block shapes and collision. A trapdoor closed at head height forces a crawl, then
    # a one-high tunnel and out again; falls from 11 blocks onto what breaks them; climbing vines; step-ups
    # walking; brushing along a row of non-full shapes; walking up snow layers 1-8.
    'crawl_trapdoor': dict(
        setup=[f'fill -1 {FLOOR + 2} 1 1 {FLOOR + 2} 6 minecraft:stone'],
        ticks=80, steps={3: [f'command setblock 0 {FLOOR + 2} 0 minecraft:oak_trapdoor[half=bottom]'],
                         10: ['key w down'], 60: ['key w up']}),
    **{f'fall_onto_{name}': dict(
        # Water in a hole in the floor, where it can't spread and push the player before the scenario starts.
        setup=[f'setblock 0 {FLOOR if block == "water" else FLOOR + 1} 3 minecraft:{block}'],
        ticks=60, anchor='accept_teleportation',
        steps=merge({0: [f'command tp @s 0.5 {FLOOR + 12} 3.5']}, extra))
       for name, block, extra in (('water', 'water', {}), ('hay', 'hay_block', {}), ('slime', 'slime_block', {}),
                                  ('slime_sneaking', 'slime_block', {0: ['key shift down'], 55: ['key shift up']}),
                                  ('powder_snow', 'powder_snow', {}), ('cobweb', 'cobweb', {}))},
    'vine_climb': dict(
        setup=[f'fill 0 {FLOOR + 1} 2 0 {FLOOR + 8} 2 minecraft:stone',
               f'fill 0 {FLOOR + 1} 1 0 {FLOOR + 8} 1 minecraft:vine[south=true]'],
        ticks=90, steps={0: ['key w down'], 30: ['key shift down'], 45: ['key shift up'], 60: ['key w up']}),
    'twisting_vines_climb': dict(
        setup=[f'fill 0 {FLOOR + 1} 0 0 {FLOOR + 6} 0 minecraft:twisting_vines'],
        ticks=80, steps={0: ['key space down'], 40: ['key space up', 'key shift down'], 55: ['key shift up']}),
    'step_ups_walk': dict(
        setup=[f'setblock 0 {FLOOR + 1} 2 minecraft:white_carpet', f'setblock 0 {FLOOR + 1} 3 minecraft:snow[layers=4]',
               f'setblock 0 {FLOOR + 1} 4 minecraft:stone_slab', f'fill 0 {FLOOR + 1} 5 0 {FLOOR + 1} 12 minecraft:stone',
               f'setblock 0 {FLOOR + 2} 6 minecraft:stone_slab', f'fill 0 {FLOOR + 2} 7 0 {FLOOR + 2} 12 minecraft:stone',
               f'fill 0 {FLOOR + 3} 9 0 {FLOOR + 3} 12 minecraft:stone'],
        ticks=70, steps={0: ['key w down'], 38: ['key space down'], 39: ['key space up'], 60: ['key w up']}),
    # One shape at a time, 1.5 blocks ahead: walked into while strafing, so the player slides along its faces
    # and off its corner.
    **{f'shape_{block}': dict(
        setup=[f'setblock 0 {FLOOR + 1} 2 minecraft:{block}'],
        ticks=40, steps={0: ['key w down', 'key a down'], 8: ['key a up'], 12: ['key a down'], 30: ['key w up', 'key a up']})
       for block in ('oak_fence', 'cobblestone_wall', 'glass_pane', 'iron_bars', 'iron_chain', 'lantern', 'end_rod',
                     'lightning_rod', 'amethyst_cluster', 'pointed_dripstone', 'cauldron', 'hopper', 'anvil', 'chest',
                     'enchanting_table', 'daylight_detector')},
    'snow_layers_walk': dict(
        setup=[f'setblock 0 {FLOOR + 1} {z} minecraft:snow[layers={z - 1}]' for z in range(2, 10)],
        ticks=60, steps={0: ['key w down'], 45: ['key w up']}),
    # Pistons moving the player: pushed sideways, then launched on a slime block.
    'piston_push': dict(
        setup=[f'setblock 0 {FLOOR + 1} -1 minecraft:piston[facing=south]'],
        ticks=50, steps={5: [f'command setblock 1 {FLOOR + 1} -1 minecraft:redstone_block'],
                         25: [f'command setblock 1 {FLOOR + 1} -1 minecraft:air']}),
    'slime_piston_launch': dict(
        setup=[f'setblock 0 {FLOOR + 1} 3 minecraft:piston[facing=up]', f'setblock 0 {FLOOR + 2} 3 minecraft:slime_block',
               f'tp {PLAYER} 0.5 {FLOOR + 3} 3.5 0 0'],
        ticks=70, steps={5: [f'command setblock 1 {FLOOR + 1} 3 minecraft:redstone_block']}),
    # Effects at 255: Speed, Jump Boost, Levitation, and Levitation with Slow Falling.
    'speed_255': dict(
        ticks=40, steps={0: [f'command effect give {PLAYER} minecraft:speed 2 254 true'], 3: ['key w down'],
                         8: ['key w up']}),
    'jump_boost_255': dict(
        ticks=60, steps={0: [f'command effect give {PLAYER} minecraft:jump_boost 3 254 true'], 3: ['key space down'],
                         4: ['key space up']}),
    'levitation_255_slow_falling': dict(
        ticks=60, steps={0: [f'command effect give {PLAYER} minecraft:levitation 1 254 true'],
                         10: [f'command effect give {PLAYER} minecraft:slow_falling 5 0 true']}),
    # Switching slots and hands while an item is in use: eating, then another slot; drawing a bow,
    # then swapping hands; a loaded crossbow switched away and back; dropping while walking.
    'switch_while_eating': dict(
        setup=[f'item replace entity {PLAYER} hotbar.0 with minecraft:bread 8',
               f'item replace entity {PLAYER} hotbar.1 with minecraft:bread 8', f'effect give {PLAYER} minecraft:hunger 2 255 true'],
        ticks=50, steps={5: ['mouse right down'], 12: ['key 2 tap'], 20: ['key 1 tap'], 30: ['mouse right up']}),
    'swap_hands_while_drawing': dict(
        setup=[f'item replace entity {PLAYER} weapon.mainhand with minecraft:bow', f'give {PLAYER} minecraft:arrow 16'],
        ticks=50, steps={0: ['look 0 -10'], 5: ['mouse right down'], 15: ['key f tap'], 25: ['key f tap'],
                         35: ['mouse right up']}),
    'crossbow_switch_away': dict(
        setup=[f'item replace entity {PLAYER} hotbar.0 with minecraft:crossbow', f'give {PLAYER} minecraft:arrow 16'],
        ticks=70, steps={0: ['look 0 -10'], 5: ['mouse right down'], 35: ['mouse right up'], 40: ['key 2 tap'],
                         45: ['key 1 tap'], 50: ['mouse right click']}),
    'drop_while_walking': dict(
        setup=[f'item replace entity {PLAYER} hotbar.0 with minecraft:stone 16'],
        ticks=40, steps={0: ['key w down'], 5: ['key q tap'], 6: ['key q tap'], 10: ['key ctrl down', 'key q down'],
                         12: ['key q up'], 25: ['key w up', 'key ctrl up']}),
    # Clutches: a fall caught by a cobweb, a ladder, slime, hay or powder snow placed below at the
    # last moment, and a boat.
    **{f'clutch_{name}': dict(
        setup=[f'item replace entity {PLAYER} weapon.mainhand with minecraft:{item}'],
        ticks=60, anchor='accept_teleportation',
        steps={0: [f'command tp @s 0.5 {FLOOR + 12} 0.5 0 90'], 1: ['look 0 90'], 24: ['mouse right click'],
               25: ['mouse right click'], 26: ['mouse right click']})
       for name, item in (('cobweb', 'cobweb'), ('slime', 'slime_block'), ('hay', 'hay_block'),
                          ('powder_snow', 'powder_snow_bucket'), ('boat', 'oak_boat'))},
    # Actions in the tick of a teleport: a click, a slot change and a jump as the teleport arrives.
    'act_on_teleport_tick': dict(
        setup=[SWORD, f'item replace entity {PLAYER} hotbar.1 with minecraft:stone 16'],
        ticks=40, anchor='accept_teleportation',
        steps={5: ['command tp @s 0.5 ~ 4.5', 'mouse left click', 'key 2 tap', 'key space down'],
               6: ['key space up', 'mouse right click'], 15: ['key w down'], 25: ['key w up']}),
    # Portals: a nether portal and an end portal walked into, then back to the start.
    'nether_portal': dict(
        setup=[f'fill -1 {FLOOR} 3 2 {FLOOR + 4} 3 minecraft:obsidian',
               f'fill 0 {FLOOR + 1} 3 1 {FLOOR + 3} 3 minecraft:nether_portal[axis=x]'],
        ticks=200, steps={0: ['key w down'], 12: ['key w up'], 140: ['key w down'], 150: ['key w up'],
                          170: ['command execute in minecraft:overworld run tp @s 0.5 -60 0.5']}),
    'end_portal': dict(
        setup=[f'setblock 0 {FLOOR} 3 minecraft:end_portal'],
        ticks=140, steps={0: ['key w down'], 12: ['key w up'],
                          110: ['command execute in minecraft:overworld run tp @s 0.5 -60 0.5']}),
    # The End portal entered in one state by both clients (in end_portal the server teleports them at different
    # points of the fall into it): falling fast from 5 blocks up, and standing still with the portal placed where the
    # player stands. The portal goes once the player is through: left at the spawn point, it sends the next client
    # to join (after the last scenario, before any reset) straight into the End.
    'end_portal_drop': dict(
        setup=[f'setblock 0 {FLOOR} 3 minecraft:end_portal'], enters='minecraft:the_end',
        ticks=120, steps={0: [f'command tp @s 0.5 {FLOOR + 6} 3.5'],
                          100: ['command execute in minecraft:overworld run tp @s 0.5 -60 0.5']}),
    'end_portal_inside': dict(
        enters='minecraft:the_end', ticks=110, steps={0: [f'command setblock 0 {FLOOR + 1} 0 minecraft:end_portal'],
                          5: [f'command execute in minecraft:overworld run setblock 0 {FLOOR + 1} 0 minecraft:air'],
                          90: ['command execute in minecraft:overworld run tp @s 0.5 -60 0.5']}),
    # Mobs' and projectiles' pushes: a shulker's bullets (levitation), a ghast's fireball, a breeze's
    # wind charge.
    'shulker_bullets': dict(
        setup=[MACE_RESISTANCE, f'summon minecraft:shulker 0.5 {FLOOR + 1} 7.5 {{PersistenceRequired:1b}}'],
        ticks=200, steps={}),
    'ghast_fireball': dict(
        setup=[MACE_RESISTANCE],
        ticks=60, steps={5: [f'command summon minecraft:fireball 0.5 {FLOOR + 2} 6.0 '
                             '{Motion:[0.0,0.0,-0.8],acceleration_power:0.0d,ExplosionPower:1b}']}),
    'breeze_wind_charge': dict(
        setup=[MACE_RESISTANCE],
        ticks=50, steps={5: [f'command summon minecraft:breeze_wind_charge 0.5 {FLOOR + 1.5} 5.0 '
                             '{Motion:[0.0,0.0,-0.8],acceleration_power:0.0d}']}),
    # Entities pushing: six pigs crammed on the player; a shulker box's lid opening under it; a boat
    # paddled on packed ice.
    'mob_cramming': dict(
        setup=[f'summon minecraft:pig {x} {FLOOR + 1} {z} {{NoAI:1b,PersistenceRequired:1b}}'
               for x, z in ((0.3, 0.7), (0.7, 0.4), (0.5, 0.2), (0.2, 0.3), (0.8, 0.8), (0.5, 0.9))],
        ticks=60, steps={20: ['key w down'], 35: ['key w up']}),
    'shulker_box_lid': dict(
        setup=[f'setblock 0 {FLOOR + 1} 0 minecraft:shulker_box', f'tp {PLAYER} 0.5 {FLOOR + 2} 0.5 0 90'],
        ticks=60, steps={0: ['look 0 90'], 5: ['mouse right click'], 30: ['key esc tap']}),
    'boat_on_ice': dict(
        setup=[f'fill -5 {FLOOR} 1 5 {FLOOR} 26 minecraft:packed_ice', f'summon minecraft:oak_boat 0.5 {FLOOR + 1} 3.5'],
        ticks=90, steps={0: ['command ride @s mount @e[type=minecraft:oak_boat,limit=1]'], 8: ['key w down'],
                         40: ['key a down'], 50: ['key a up'], 60: ['key w up'], 80: ['key shift down'],
                         84: ['key shift up']}),
    # A skeleton horse ridden and dismounted.
    'skeleton_horse_ride': dict(
        setup=[f'summon minecraft:skeleton_horse 0.5 {FLOOR + 1} 2.5 {{Tame:1b,PersistenceRequired:1b,Rotation:[0f,0f],'
               'equipment:{saddle:{id:"minecraft:saddle",count:1}}}'],
        ticks=80, steps={0: ['command ride @s mount @e[type=minecraft:skeleton_horse,limit=1]'], 6: ['key w down'],
                         20: ['key space down'], 30: ['key space up'], 50: ['key w up'], 60: ['key shift down'],
                         64: ['key shift up']}),
    # Respawn anchor: charged with glowstone, then used in the Overworld, where it explodes; a TNT
    # minecart's explosion.
    'respawn_anchor': dict(
        setup=[MACE_RESISTANCE, f'setblock 0 {FLOOR + 1} 2 minecraft:respawn_anchor',
               f'item replace entity {PLAYER} hotbar.0 with minecraft:glowstone 4'],
        ticks=60, steps={0: ['look 0 30'], 5: ['mouse right click'], 15: ['key 2 tap'], 20: ['mouse right click']}),
    'tnt_minecart': dict(
        setup=[MACE_RESISTANCE, f'summon minecraft:tnt_minecart 0.5 {FLOOR + 1} 3.5 {{fuse:20}}'],
        ticks=60, steps={}),
    # Block uses: a cauldron filled from a bucket, a composter fed, a note block tuned and played, a
    # jukebox given a disc and emptied, a bed in daytime.
    'use_cauldron': dict(
        setup=[f'setblock 0 {FLOOR + 1} 2 minecraft:cauldron',
               f'item replace entity {PLAYER} hotbar.0 with minecraft:water_bucket'],
        ticks=40, steps={0: ['look 0 30'], 5: ['mouse right click'], 15: ['mouse right click']}),
    'use_composter': dict(
        setup=[f'setblock 0 {FLOOR + 1} 2 minecraft:composter',
               f'item replace entity {PLAYER} hotbar.0 with minecraft:wheat_seeds 16'],
        ticks=40, steps=merge({0: ['look 0 30']}, {t: ['mouse right click'] for t in range(5, 25, 4)})),
    'use_note_block': dict(
        setup=[f'setblock 0 {FLOOR + 1} 2 minecraft:note_block'],
        ticks=40, steps={0: ['look 0 30'], 5: ['mouse right click'], 10: ['mouse right click'], 15: ['mouse left click']}),
    'use_jukebox': dict(
        setup=[f'setblock 0 {FLOOR + 1} 2 minecraft:jukebox',
               f'item replace entity {PLAYER} hotbar.0 with minecraft:music_disc_cat'],
        ticks=40, steps={0: ['look 0 30'], 5: ['mouse right click'], 20: ['mouse right click']}),
    'use_bed': dict(
        setup=[f'setblock 0 {FLOOR + 1} 2 minecraft:red_bed[facing=south,part=foot]',
               f'setblock 0 {FLOOR + 1} 3 minecraft:red_bed[facing=south,part=head]'],
        ticks=40, steps={0: ['look 0 30'], 5: ['mouse right click']}),
    # Tools on blocks and mobs: shears on a sheep, a brush on suspicious sand, a fire charge on stone.
    'use_shears': dict(
        setup=[f'summon minecraft:sheep 0.5 {FLOOR + 1} 2.0 {{NoAI:1b,PersistenceRequired:1b}}',
               f'item replace entity {PLAYER} hotbar.0 with minecraft:shears'],
        ticks=40, steps={0: ['look 0 25'], 5: ['mouse right click']}),
    'use_brush': dict(
        setup=[f'setblock 0 {FLOOR + 1} 2 minecraft:suspicious_sand',
               f'item replace entity {PLAYER} hotbar.0 with minecraft:brush'],
        ticks=80, steps={0: ['look 0 30'], 5: ['mouse right down'], 70: ['mouse right up']}),
    'use_fire_charge': dict(
        setup=[f'setblock 0 {FLOOR + 1} 2 minecraft:stone',
               f'item replace entity {PLAYER} hotbar.0 with minecraft:fire_charge 4'],
        ticks=40, steps={0: ['look 0 30'], 5: ['mouse right click']}),
    # An axe disables the shield: a vindicator attacks the blocking player.
    'axe_disables_shield': dict(
        setup=[MACE_RESISTANCE, f'item replace entity {PLAYER} weapon.offhand with minecraft:shield'],
        ticks=120, steps={0: ['look 0 10', f'command summon minecraft:vindicator 0.5 {FLOOR + 1} 5.5 '
                              '{PersistenceRequired:1b,equipment:{mainhand:{id:"minecraft:iron_axe",count:1}}}'],
                          5: ['mouse right down'], 110: ['mouse right up']}),
    # Real mouse turning, at the default sensitivity (0.5) both clients have: raw mouse counts through
    # MouseHandler (the sensitivity's scale, Entity.turn's 0.15), the rotation steps anticheats' aim checks read.
    'mouse_slow_turn': dict(
        ticks=60, steps={t: [f'mouse-move {3 + t % 2} {1 if t % 5 == 0 else 0}'] for t in range(0, 50)}),
    'mouse_fast_turn': dict(
        ticks=50, steps={t: [f'mouse-move {85 if t < 20 else -70} {12 if t % 3 else -9}'] for t in range(0, 40)}),
    'mouse_flicks': dict(
        ticks=50, steps={5: ['mouse-move 640 0'], 15: ['mouse-move -1100 120'], 25: ['mouse-move 37 -260'],
                         35: ['mouse-move -2 3']}),
    'mouse_micro': dict(
        ticks=50, steps={t: [f'mouse-move {(-1) ** t} {(-1) ** (t // 2)}'] for t in range(0, 40)}),
    'mouse_turn_walk_attack': dict(
        setup=[TARGET.format(x=0.5, y=FLOOR + 1, z=4.5), SWORD],
        ticks=70, steps=merge({0: ['key w down'], 50: ['key w up']},
                              {t: [f'mouse-move {(-1) ** (t // 6) * 9} {2 if t < 20 else -1}'] for t in range(0, 50)},
                              {t: ['mouse left click'] for t in (12, 24, 36, 48)})),
    # Spear: jabs at a target, then a charge held while sprinting at it.
    'spear_jab_charge': dict(
        setup=[TARGET.format(x=0.5, y=FLOOR + 1, z=5.5), f'item replace entity {PLAYER} weapon.mainhand with minecraft:iron_spear'],
        ticks=90, steps={0: ['look 0 10'], 5: ['mouse left click'], 25: ['mouse left click'], 40: ['key ctrl down', 'key w down'],
                         42: ['mouse right down'], 70: ['mouse right up'], 75: ['key w up', 'key ctrl up']}),
    # A happy ghast with a harness, ridden: up, forward, down.
    # Turned to 0 and mounted after the settle: left alone, the mob turned at random before the mount, so the two
    # clients' rides differed.
    'happy_ghast_ride': dict(
        setup=[f'summon minecraft:happy_ghast 0.5 {FLOOR + 1} 3.5 {{PersistenceRequired:1b,Rotation:[0f,0f],'
               'equipment:{body:{id:"minecraft:white_harness",count:1}}}'],
        mount=['rotate @e[type=minecraft:happy_ghast,limit=1] 0 0',
               f'ride {PLAYER} mount @e[type=minecraft:happy_ghast,limit=1]'],
        ticks=100, steps={8: ['key space down'],
                          28: ['key space up', 'key w down'], 60: ['key w up', 'key shift down'], 62: ['key shift up']}),
    # A boat shared with a pig: the player takes the second seat and paddles.
    'shared_boat': dict(
        setup=[f'fill -5 {FLOOR} 1 5 {FLOOR} 26 minecraft:water',
               f'summon minecraft:oak_boat 0.5 {FLOOR + 1} 3.5 {{Passengers:[{{id:"minecraft:pig",NoAI:1b}}]}}'],
        ticks=80, steps={0: ['command ride @s mount @e[type=minecraft:oak_boat,limit=1]'], 8: ['key w down'],
                         30: ['key a down'], 40: ['key a up'], 60: ['key w up'], 70: ['key shift down'],
                         72: ['key shift up']}),
    # A server-opened screen: a chested donkey's inventory, opened with the inventory key while riding.
    'donkey_inventory': dict(
        setup=[f'summon minecraft:donkey 0.5 {FLOOR + 1} 2.5 {{Tame:1b,ChestedHorse:1b,PersistenceRequired:1b,Rotation:[0f,0f],'
               'equipment:{saddle:{id:"minecraft:saddle",count:1}}}'],
        ticks=60, steps={0: ['command ride @s mount @e[type=minecraft:donkey,limit=1]'], 10: ['key e tap'],
                         20: ['click-slot 2'], 25: ['click-slot 20'], 35: ['key esc tap'], 45: ['key shift down'],
                         47: ['key shift up']}),
    # Animal interactions: milking a cow, feeding sheep (breeding), dyeing one.
    'animal_interactions': dict(
        setup=[f'summon minecraft:cow 0.5 {FLOOR + 1} 2.0 {{NoAI:1b,PersistenceRequired:1b}}',
               f'summon minecraft:sheep -1.5 {FLOOR + 1} 2.0 {{NoAI:1b,PersistenceRequired:1b}}',
               f'summon minecraft:sheep 2.5 {FLOOR + 1} 2.0 {{NoAI:1b,PersistenceRequired:1b}}',
               f'item replace entity {PLAYER} hotbar.0 with minecraft:bucket',
               f'item replace entity {PLAYER} hotbar.1 with minecraft:wheat 8',
               f'item replace entity {PLAYER} hotbar.2 with minecraft:blue_dye 4'],
        ticks=80, steps={0: ['look 0 25'], 5: ['mouse right click'], 15: ['key 2 tap', 'look 45 25'],
                         20: ['mouse right click'], 30: ['look -45 25'], 35: ['mouse right click'],
                         45: ['key 3 tap'], 50: ['mouse right click']}),
    # Bridging techniques over a trench: a ninja bridge (walking backwards, sneaking at each edge)
    # and a jump bridge (jumping and placing below each jump), facing away.
    'ninja_bridge': dict(
        setup=[f'fill -3 {FLOOR - 2} 2 3 {FLOOR} 24 minecraft:air',
               f'item replace entity {PLAYER} weapon.mainhand with minecraft:stone 64'],
        ticks=120, steps=merge({0: ['look 180 78'], 3: ['key s down']},
                               {t: ['key shift down'] for t in range(10, 110, 10)},
                               {t: ['key shift up'] for t in range(14, 110, 10)},
                               {t: ['mouse right click'] for t in range(11, 110, 10)}, {110: ['key s up']})),
    'jump_bridge': dict(
        setup=[f'fill -3 {FLOOR - 2} 2 3 {FLOOR} 24 minecraft:air',
               f'item replace entity {PLAYER} weapon.mainhand with minecraft:stone 64'],
        ticks=100, steps=merge({0: ['look 180 80'], 3: ['key s down', 'key space down']},
                               {t: ['mouse right click'] for t in range(8, 90, 3)}, {90: ['key s up', 'key space up']})),
    # Knockback from an enchanted weapon: a zombie with a Knockback II sword hits the player.
    'kb_enchanted_mob': dict(
        setup=[MACE_RESISTANCE],
        ticks=100, steps={0: [f'command summon minecraft:zombie 0.5 {FLOOR + 1} 4.5 {{PersistenceRequired:1b,'
                              'equipment:{mainhand:{id:"minecraft:iron_sword",count:1,components:{"minecraft:enchantments":'
                              '{"minecraft:knockback":2}}}}}']}),
    # Trading hits: the player attacks a husk that attacks back, w-tapping between hits.
    'trade_hits': dict(
        setup=[MACE_RESISTANCE, CHASER, SWORD],
        ticks=120, steps=merge({0: ['look 0 5', 'key w down']},
                               {t: ['mouse left click'] for t in range(20, 110, 12)},
                               {t: ['key w up'] for t in range(22, 110, 12)},
                               {t: ['key w down'] for t in range(25, 110, 12)}, {112: ['key w up']})),
    # Keep-alive: 20 seconds standing, so a keep-alive (every 15 s) and its answer fall inside: the replay compares
    # the answer's tick and id, a live run leaves keep-alives out. Run with WIRE_STREAM_LATENCY for a slow or
    # changing connection.
    'keepalive_idle': dict(ticks=400, steps={200: ['key w down'], 204: ['key w up']}),
    # Options both clients start with (WIRE_STREAM_OPTIONS): Auto-Jump walking into a step
    # (autoJump:true), toggled sprint and sneak tapped on and off (toggleSprint:true,toggleCrouch:true).
    'autojump_walk': dict(
        setup=[f'setblock 0 {FLOOR + 1} 3 minecraft:stone', f'fill 0 {FLOOR + 1} 5 0 {FLOOR + 2} 5 minecraft:stone'],
        ticks=60, steps={0: ['key w down'], 45: ['key w up']}),
    'toggle_keys': dict(
        ticks=80, steps={0: ['key ctrl tap'], 2: ['key w down'], 20: ['key shift tap'], 35: ['key shift tap'],
                         45: ['key ctrl tap'], 60: ['key w up']}),
    # Section 23 (anticheat-sensitive). Combat timing: a sword's attacks at exactly its full charge
    # (12.5 ticks: 13 apart) and one tick early (12), at a target walking into reach, while blocking with a shield,
    # and sprint resets (w-tap, s-tap) between hits.
    'attack_cooldown_edges': dict(
        setup=[TARGET.format(x=0.5, y=FLOOR + 1, z=2.6), SWORD],
        ticks=80, steps=merge({0: ['look 0 15']}, {t: ['mouse left click'] for t in (10, 23, 35, 48, 60, 61)})),
    'attack_entering_reach': dict(
        setup=[MACE_RESISTANCE, SWORD, f'summon minecraft:husk 0.5 {FLOOR + 1} 7.5 {{PersistenceRequired:1b,'
               'Health:1000f,attributes:[{id:"minecraft:max_health",base:1000.0}]}'],
        ticks=90, steps=merge({0: ['look 0 5']}, {t: ['mouse left click'] for t in range(8, 80, 4)})),
    'attack_while_blocking': dict(
        setup=[TARGET.format(x=0.5, y=FLOOR + 1, z=2.6), SWORD,
               f'item replace entity {PLAYER} weapon.offhand with minecraft:shield'],
        ticks=60, steps={0: ['look 0 15'], 5: ['mouse right down'], 15: ['mouse left click'], 25: ['mouse left click'],
                         30: ['mouse right up'], 31: ['mouse left click'], 45: ['mouse left click']}),
    'sprint_reset_taps': dict(
        setup=[TARGET.format(x=0.5, y=FLOOR + 1, z=6.5), SWORD],
        ticks=90, steps={0: ['look 0 5', 'key ctrl down', 'key w down'], 18: ['mouse left click'], 20: ['key w up'],
                         22: ['key w down'], 31: ['mouse left click'], 33: ['key s down'], 35: ['key s up'],
                         44: ['mouse left click'], 50: ['key ctrl up'], 52: ['key ctrl down'], 57: ['mouse left click'],
                         70: ['key w up', 'key ctrl up']}),
    # Crits: jumping attacks as the jump rises, at its top and on the way down; a sprinting jump's (no crit).
    **{f'crit_jump_{when}': dict(
        setup=[TARGET.format(x=0.5, y=FLOOR + 1, z=2.6), SWORD],
        ticks=40, steps=merge({0: ['look 0 15'], 5: ['key space down'], 6: ['key space up']},
                              {5 + delay: ['mouse left click']}, sprint))
       for when, delay, sprint in (('rising', 3, {}), ('top', 6, {}), ('falling', 9, {}),
                                   ('sprinting', 7, {3: ['key ctrl down', 'key w down'], 14: ['key w up', 'key ctrl up']}))},
    # Combat across a server stall: the server's ticks stop for 15 ticks while the player keeps hitting.
    'attack_through_freeze': dict(
        setup=[TARGET.format(x=0.5, y=FLOOR + 1, z=2.6), SWORD],
        ticks=70, steps=merge({0: ['look 0 15'], 10: ['command tick freeze'], 25: ['command tick unfreeze']},
                              {t: ['mouse left click'] for t in range(8, 50, 7)})),
    # Block breaking packets: a 2-tick break, a long break given up halfway, a target switched mid-break,
    # a break at the reach limit, breaking while falling.
    'break_quick_and_abort': dict(
        setup=[f'setblock 0 {FLOOR + 1} 2 minecraft:dirt', f'setblock 1 {FLOOR + 1} 2 minecraft:obsidian',
               f'item replace entity {PLAYER} weapon.mainhand with minecraft:diamond_shovel'],
        ticks=80, steps={0: ['look 0 40'], 5: ['mouse left down'], 15: ['mouse left up'], 20: ['look -30 40'],
                         25: ['mouse left down'], 55: ['mouse left up']}),
    'break_switch_target': dict(
        setup=[f'fill -1 {FLOOR + 1} 2 1 {FLOOR + 1} 2 minecraft:stone',
               f'item replace entity {PLAYER} weapon.mainhand with minecraft:iron_pickaxe'],
        ticks=80, steps={0: ['look 0 40', 'mouse left down'], 10: ['look -30 40'], 20: ['look 30 40'], 40: ['look 0 40'],
                         70: ['mouse left up']}),
    'break_at_reach': dict(
        setup=[f'setblock 0 {FLOOR + 1} 5 minecraft:stone', f'item replace entity {PLAYER} weapon.mainhand with minecraft:iron_pickaxe'],
        ticks=70, steps={0: ['look 0 18'], 5: ['mouse left down'], 15: ['key s down'], 17: ['key s up'], 60: ['mouse left up']}),
    'break_while_falling': dict(
        setup=[f'fill -1 {FLOOR + 1} 2 1 {FLOOR + 8} 2 minecraft:stone', f'item replace entity {PLAYER} weapon.mainhand with minecraft:diamond_pickaxe[minecraft:enchantments={{"minecraft:efficiency":5}}]'],
        ticks=60, anchor='accept_teleportation',
        steps={0: ['command tp @s 0.5 ~8 0.5 0 30'], 1: ['look 0 30', 'mouse left down'], 40: ['mouse left up']}),
    # Block placing packets: every face of a block, at the reach limit, against a mob standing where the
    # block would go (refused), and while sprint-jumping.
    'place_faces_and_reach': dict(
        setup=[f'setblock 0 {FLOOR + 2} 3 minecraft:stone', f'item replace entity {PLAYER} weapon.mainhand with minecraft:stone 64'],
        ticks=70, steps={0: ['look 0 10'], 5: ['mouse right click'], 10: ['look -25 0'], 14: ['mouse right click'],
                         18: ['look 25 0'], 22: ['mouse right click'], 26: ['look 0 -30'], 30: ['mouse right click'],
                         34: ['look 0 8'], 36: ['key s down'], 40: ['key s up'], 44: ['mouse right click'], 50: ['mouse right click']}),
    'place_into_mob': dict(
        setup=[TARGET.format(x=0.5, y=FLOOR + 1, z=3.5), f'item replace entity {PLAYER} weapon.mainhand with minecraft:stone 64'],
        ticks=40, steps={0: ['look 0 50'], 5: ['mouse right click'], 10: ['look 0 35'], 15: ['mouse right click']}),
    'place_sprint_jumping': dict(
        setup=[f'item replace entity {PLAYER} weapon.mainhand with minecraft:stone 64'],
        ticks=60, steps=merge({0: ['look 0 70', 'key ctrl down', 'key w down', 'key space down']},
                              {t: ['mouse right click'] for t in range(4, 44, 2)}, {45: ['key w up', 'key ctrl up', 'key space up']})),
    # Elytra: from 30 blocks up, open the wings (a jump in the air), glide, pitch down and up, a firework
    # boost, glide into a wall, then land.
    'elytra_flight': dict(
        setup=[MACE_RESISTANCE, f'item replace entity {PLAYER} armor.chest with minecraft:elytra',
               f'item replace entity {PLAYER} weapon.mainhand with minecraft:firework_rocket 4',
               f'fill -6 {FLOOR + 1} 24 6 {FLOOR + 12} 24 minecraft:stone'],
        ticks=160, anchor='accept_teleportation',
        steps={0: [f'command tp @s 0.5 {FLOOR + 30} 0.5 0 10'], 8: ['key space down'], 9: ['key space up'],
               12: ['key space down'], 13: ['key space up'], 30: ['look 0 40'], 45: ['look 0 -15'], 55: ['mouse right click'],
               90: ['look 0 20'], 140: ['look 0 60']}),
    'elytra_riptide_water': dict(
        setup=[MACE_RESISTANCE, f'item replace entity {PLAYER} armor.chest with minecraft:elytra',
               f'fill -4 {FLOOR - 2} 4 4 {FLOOR} 30 minecraft:water'],
        ticks=120, anchor='accept_teleportation',
        steps={0: [f'command tp @s 0.5 {FLOOR + 14} 0.5 0 20'], 6: ['key space down'], 7: ['key space up'],
               10: ['key space down'], 11: ['key space up'], 30: ['look 0 50']}),
    # Riptide: in rain, at each level, into a wall, two back to back.
    **{f'riptide_rain_{level}': dict(
        setup=['weather rain', f'item replace entity {PLAYER} weapon.mainhand with minecraft:trident[minecraft:enchantments={{"minecraft:riptide":{level}}}]'],
        ticks=70, steps={0: ['look 0 -20'], 5: ['mouse right down'], 20: ['mouse right up'], 50: ['command weather clear']})
       for level in (1, 2, 3)},
    'riptide_into_wall': dict(
        setup=['weather rain', f'fill -3 {FLOOR + 1} 4 3 {FLOOR + 6} 4 minecraft:stone',
               f'item replace entity {PLAYER} weapon.mainhand with minecraft:trident[minecraft:enchantments={{"minecraft:riptide":3}}]'],
        ticks=70, steps={0: ['look 0 -5'], 5: ['mouse right down'], 20: ['mouse right up'], 50: ['command weather clear']}),
    'riptide_twice': dict(
        setup=['weather rain', f'item replace entity {PLAYER} weapon.mainhand with minecraft:trident[minecraft:enchantments={{"minecraft:riptide":2}}]'],
        ticks=90, steps={0: ['look 0 -15'], 5: ['mouse right down'], 18: ['mouse right up'], 40: ['look 90 -15'],
                         41: ['mouse right down'], 54: ['mouse right up'], 80: ['command weather clear']}),
    # Vehicles the server checks: a horse's jump charged 5, 10 and 20 ticks; a strider on lava boosted
    # with a warped fungus; a boat the server teleports mid-ride; dismounting at a trench's edge and in water; a
    # minecart left while it runs on powered rails.
    'horse_jump_strengths': dict(
        setup=[f'summon minecraft:horse 0.5 {FLOOR + 1} 2.5 {{Tame:1b,PersistenceRequired:1b,Rotation:[0f,0f],Variant:0,'
               'equipment:{saddle:{id:"minecraft:saddle",count:1}},attributes:[{id:"minecraft:jump_strength",base:0.8}]}'],
        ticks=120, steps={0: ['command ride @s mount @e[type=minecraft:horse,limit=1]'], 10: ['key space down'],
                          15: ['key space up'], 40: ['key space down'], 50: ['key space up'], 75: ['key space down'],
                          95: ['key space up']}),
    # Facing 0 (summoned at random, the rides differed between the clients), mounted at tick 0: mounted in the
    # setup, the strider took the walk before the scenario and dropped the player into the lava.
    'strider_lava': dict(
        setup=[f'fill -3 {FLOOR} 1 3 {FLOOR} 20 minecraft:lava',
               f'summon minecraft:strider 0.5 {FLOOR + 1} 3.5 {{PersistenceRequired:1b,Rotation:[0f,0f],equipment:{{saddle:{{id:"minecraft:saddle",count:1}}}}}}',
               f'item replace entity {PLAYER} weapon.mainhand with minecraft:warped_fungus_on_a_stick'],
        ticks=90, steps={0: ['command ride @s mount @e[type=minecraft:strider,limit=1]'], 10: ['look 0 10'],
                         30: ['mouse right click'], 55: ['look 25 10'], 80: ['key shift down'], 82: ['key shift up']}),
    'boat_teleported': dict(
        setup=[f'fill -5 {FLOOR} 1 5 {FLOOR} 26 minecraft:water', f'summon minecraft:oak_boat 0.5 {FLOOR + 1} 3.5'],
        ticks=80, steps={0: ['command ride @s mount @e[type=minecraft:oak_boat,limit=1]'], 6: ['key w down'],
                         30: ['command execute as @e[type=minecraft:oak_boat] run tp @s ~ ~ ~4'], 60: ['key w up']}),
    'dismount_edge_and_water': dict(
        setup=[f'fill -3 {FLOOR - 2} 4 3 {FLOOR} 8 minecraft:air', f'fill -3 {FLOOR - 2} 9 3 {FLOOR} 14 minecraft:water',
               f'summon minecraft:horse 0.5 {FLOOR + 1} 2.5 {{Tame:1b,PersistenceRequired:1b,Rotation:[0f,0f],Variant:0,'
               'equipment:{saddle:{id:"minecraft:saddle",count:1}}}'],
        ticks=90, steps={0: ['command ride @s mount @e[type=minecraft:horse,limit=1]'], 6: ['key w down'], 14: ['key w up'],
                         20: ['key shift down'], 22: ['key shift up'],
                         40: ['command ride @s mount @e[type=minecraft:horse,limit=1]'], 45: ['command execute as @e[type=minecraft:horse] run tp @s 0.5 ~ 11.5'],
                         55: ['key shift down'], 57: ['key shift up']}),
    'minecart_jump_off': dict(
        setup=[f'fill 0 {FLOOR} 4 0 {FLOOR} 7 minecraft:redstone_block',
               f'fill 0 {FLOOR + 1} 1 0 {FLOOR + 1} 24 minecraft:rail[shape=north_south]',
               f'fill 0 {FLOOR + 1} 4 0 {FLOOR + 1} 7 minecraft:powered_rail[shape=north_south,powered=true]',
               f'summon minecraft:minecart 0.5 {FLOOR + 1} 2.5'],
        ticks=70, steps={0: ['command ride @s mount @e[type=minecraft:minecart,limit=1]'], 5: ['key w down'],
                         30: ['key shift down'], 32: ['key shift up'], 33: ['key w up']}),
    # The pause menu while walking and jumping: Gui.setScreen releases the held keys, so the player stops
    # (a server's game doesn't pause), and the keys held on past its close don't count until pressed again.
    'pause_menu_walk': dict(
        ticks=70, steps={0: ['key w down', 'key space down'], 10: ['key esc tap'], 30: ['key esc tap'],
                         45: ['key w up', 'key space up'], 50: ['key w down'], 60: ['key w up']}),
    # The advancements screen: the L key opens it (ServerboundSeenAdvancementsPacket.openedTab for the
    # first tab), Escape closes it (closedScreen), twice.
    'advancements_open': dict(
        ticks=40, steps={0: ['key l tap'], 12: ['key esc tap'], 22: ['key l tap'], 32: ['key esc tap']}),
    # A spyglass: held up to the eye while the mouse turns (scoping scales the turn), then lowered.
    # Needs WIRE_STREAM_WINDOWED for the mouse.
    'spyglass_turn': dict(
        setup=[f'item replace entity {PLAYER} weapon.mainhand with minecraft:spyglass'],
        ticks=50, steps=merge({0: ['mouse right down'], 35: ['mouse right up']},
                              {t: [f'mouse-move {(-1) ** (t // 5) * 12} {3 if t < 20 else -3}'] for t in range(4, 44)})),
    # A sign: placed on the floor, which opens its editor, then left with Escape (the editor sends the
    # sign's lines as it goes), and a walk after.
    'sign_place_close': dict(
        setup=[f'item replace entity {PLAYER} weapon.mainhand with minecraft:oak_sign'],
        ticks=40, steps={0: ['look 0 60'], 3: ['mouse right click'], 15: ['key esc tap'], 25: ['key w down'],
                         30: ['key w up']}),
    # A book and quill: opened and closed unchanged, twice.
    'book_open_close': dict(
        setup=[f'item replace entity {PLAYER} weapon.mainhand with minecraft:writable_book'],
        ticks=45, steps={2: ['mouse right click'], 15: ['key esc tap'], 25: ['mouse right click'], 35: ['key esc tap']}),
    # The creative inventory: opened with E, closed with Escape, opened and closed with E.
    'creative_open_close': dict(
        setup=[f'gamemode creative {PLAYER}'],
        ticks=40, steps={0: ['key e tap'], 10: ['key esc tap'], 20: ['key e tap'], 30: ['key e tap']}),
    # Villager trading: a farmer's one offer, opened with a click, the offer picked (the client fills the
    # payment slots itself), the result taken with a shift-click, the screen closed.
    'villager_trade': dict(
        setup=[f'summon minecraft:villager 0.5 {FLOOR + 1} 2.5 {{NoAI:1b,Silent:1b,PersistenceRequired:1b,'
               'VillagerData:{profession:"minecraft:farmer",level:1,type:"minecraft:plains"},'
               'Offers:{Recipes:[{buy:{id:"minecraft:emerald",count:1},sell:{id:"minecraft:bread",count:6},'
               'maxUses:12,xp:1,priceMultiplier:0.05f}]}}', f'give {PLAYER} minecraft:emerald 5'],
        ticks=34, steps={0: ['look 0 15'], 2: ['mouse right click'], 8: ['select-trade 0'],
                         14: ['click-slot 2 shift'], 22: ['key esc tap']}),
    # A beacon on a 3x3 iron pyramid: opened, an iron ingot paid, speed confirmed.
    # The beacon counts its pyramid only every 80 game ticks (BeaconBlockEntity.tick), so the player waits 85
    # before opening it: Paper disconnects a client that asks a level-0 beacon for speed.
    'beacon_set': dict(
        setup=[f'fill -1 {FLOOR} 2 1 {FLOOR} 4 minecraft:iron_block', f'setblock 0 {FLOOR + 1} 3 minecraft:beacon',
               f'give {PLAYER} minecraft:iron_ingot 1'],
        ticks=115, steps={0: ['look 0 20'], 87: ['mouse right click'], 93: ['click-slot 28'], 97: ['click-slot 0'],
                          103: ['beacon speed']}),
    # Statistics: the pause menu's Statistics button (the stats request), back to the pause menu, out.
    'stats_open': dict(ticks=30, steps={2: ['open-stats'], 15: ['key esc tap'], 22: ['key esc tap']}),
    # 30 minutes standing: keep-alives, the 20-tick position reminder and the time over a long idle, then
    # a step, for slow drift.
    # Peaceful for the half hour: a slime killed the standing player once (spawn_mobs doesn't cover it); easy again
    # (the server's default) at the end.
    'afk_30min': dict(setup=['difficulty peaceful'], ticks=36040,
                      steps={36000: ['key w down'], 36010: ['key w up'], 36030: ['command difficulty easy']}),
    # A controller on Sunrise against the keyboard and mouse on vanilla: the left stick walks, south jumps,
    # the left stick's click sprints, the right trigger attacks. On a server the stick moves as the keys it sets do.
    'pad_walk_jump_attack': dict(
        setup=[TARGET.format(x=0.5, y=FLOOR + 1, z=6.5), SWORD], ticks=60,
        steps={0: ['key w down'], 10: ['key space down'], 12: ['key space up'], 20: ['key ctrl down'], 30: ['mouse left click'],
               36: ['key ctrl up'], 45: ['key w up'], 50: ['mouse left click']},
        # The virtual pad is read at the start of the frame, before the tick's script lines: each line comes a
        # tick ahead, as a real pad's state, read between ticks, reaches the tick after it.
        sunrise_steps={-2: ['pad connection connected'], -1: ['pad stick left 0 1'], 9: ['pad south down'], 11: ['pad south up'],
                       19: ['pad ls down'], 29: ['pad trigger right 1'], 30: ['pad trigger right 0'], 35: ['pad ls up'],
                       44: ['pad stick left 0 0'], 49: ['pad trigger right 1'], 50: ['pad trigger right 0'],
                       58: ['pad connection disconnected']}),
    # Half a tilt of the stick forward and to the side: on a server the keys it sets, at full speed.
    'pad_half_stick': dict(
        ticks=50, steps={0: ['key w down', 'key d down'], 30: ['key w up', 'key d up']},
        sunrise_steps={-2: ['pad connection connected'], -1: ['pad stick left 0.45 0.45'], 29: ['pad stick left 0 0'],
                       48: ['pad connection disconnected']}),
    # Death and respawn: killed, keys and clicks on the death screen (which take no action),
    # the respawn button, then walking again.
    'death_respawn': dict(
        ticks=90, steps={5: ['command kill @s'], 15: ['mouse left click', 'key w down'], 20: ['key w up'],
                         40: ['respawn'], 70: ['key w down'], 80: ['key w up']}),
    # Dimension changes: to the Nether's roof, walking there, to the End above its island, falling
    # onto it, and back to the start (the reset teleports within the dimension the player is in).
    'dimensions': dict(
        ticks=200, steps={5: ['command execute in minecraft:the_nether run tp @s 0.5 128 0.5'],
                          50: ['key w down'], 60: ['key w up'],
                          80: ['command execute in minecraft:the_end run tp @s 0.5 75 0.5'],
                          150: ['command execute in minecraft:overworld run tp @s 0.5 -60 0.5'],
                          180: ['key w down'], 190: ['key w up']}),
    # Container clicks: every ContainerInput a screen sends, in a chest, a crafting table, a
    # furnace and the player's own inventory. Each ContainerClick carries the client's predicted slot changes and
    # carried item, which the server compares with its own (a wrong one resyncs, and anticheats watch them).
    'chest_clicks': dict(
        setup=[f'setblock 0 {FLOOR + 1} 2 minecraft:chest[facing=north]{{Items:[{{Slot:0b,id:"minecraft:stone",'
               'count:32},{Slot:1b,id:"minecraft:oak_planks",count:10}]}',
               f'item replace entity {PLAYER} hotbar.0 with minecraft:diamond_sword'],
        ticks=60, steps={0: ['look 0 45'], 5: ['mouse right click'], 12: ['click-slot 0'], 15: ['click-slot 55'],
                         18: ['click-slot 1 right'], 21: ['click-slot 27'], 24: ['click-slot 55 shift'],
                         27: ['click-slot 27 SWAP b4'], 30: ['click-slot 0 THROW b0'], 33: ['click-slot 0 THROW b1'],
                         36: ['click-slot 54 SWAP b40'], 40: ['key e tap']}),
    # A left drag of 16 stone over three slots (QUICK_CRAFT start, add, end), a right drag over two, then a
    # double click (PICKUP_ALL) gathering them again.
    'chest_drag': dict(
        setup=[f'setblock 0 {FLOOR + 1} 2 minecraft:chest[facing=north]',
               f'item replace entity {PLAYER} hotbar.0 with minecraft:stone 16'],
        # A screen sends a drag's clicks together when the button comes up (AbstractContainerScreen.mouseReleased).
        ticks=70, steps={0: ['look 0 45'], 5: ['mouse right click'], 12: ['click-slot 54'],
                         14: ['click-slot -999 QUICK_CRAFT b0', 'click-slot 0 QUICK_CRAFT b1',
                              'click-slot 1 QUICK_CRAFT b1', 'click-slot 2 QUICK_CRAFT b1',
                              'click-slot -999 QUICK_CRAFT b2'], 22: ['click-slot 0'],
                         24: ['click-slot -999 QUICK_CRAFT b4', 'click-slot 9 QUICK_CRAFT b5',
                              'click-slot 10 QUICK_CRAFT b5', 'click-slot -999 QUICK_CRAFT b6'],
                         30: ['click-slot 3'], 31: ['click-slot 3 PICKUP_ALL b0'], 35: ['click-slot 54'],
                         45: ['key e tap']}),
    # Planks one by one into the grid (a crafting table appears as the result), a shift-click on the result, and
    # closing with planks left in the grid (they go back to the inventory).
    'crafting_table': dict(
        setup=[f'setblock 0 {FLOOR + 1} 2 minecraft:crafting_table',
               f'item replace entity {PLAYER} hotbar.0 with minecraft:oak_planks 9'],
        ticks=60, steps={0: ['look 0 45'], 5: ['mouse right click'], 12: ['click-slot 37'],
                         14: ['click-slot 1 right'], 16: ['click-slot 2 right'], 18: ['click-slot 4 right'],
                         20: ['click-slot 5 right'], 22: ['click-slot 3 right'], 24: ['click-slot 37'],
                         27: ['click-slot 0 shift'], 32: ['click-slot 0'], 35: ['click-slot 38'],
                         40: ['key e tap']}),
    # Workstations: a grindstone takes an enchanted sword's enchantments off, a smithing table upgrades a
    # diamond sword to netherite (each result shift-clicked out).
    'grindstone_use': dict(
        setup=[f'setblock 0 {FLOOR + 1} 2 minecraft:grindstone[face=floor,facing=north]',
               f'item replace entity {PLAYER} hotbar.0 with minecraft:iron_sword[minecraft:enchantments={{"minecraft:sharpness":3}}]'],
        ticks=60, steps={0: ['look 0 45'], 5: ['mouse right click'], 12: ['click-slot 30'], 15: ['click-slot 0'],
                         20: ['click-slot 2 shift'], 30: ['key e tap']}),
    'smithing_use': dict(
        setup=[f'setblock 0 {FLOOR + 1} 2 minecraft:smithing_table',
               f'item replace entity {PLAYER} hotbar.0 with minecraft:netherite_upgrade_smithing_template',
               f'item replace entity {PLAYER} hotbar.1 with minecraft:diamond_sword',
               f'item replace entity {PLAYER} hotbar.2 with minecraft:netherite_ingot'],
        ticks=60, steps={0: ['look 0 45'], 5: ['mouse right click'], 12: ['click-slot 31 shift'], 15: ['click-slot 32 shift'],
                         18: ['click-slot 33 shift'], 24: ['click-slot 3 shift'], 34: ['key e tap']}),
    # Station buttons: a stonecutter's first recipe for stone, a loom's pattern on a banner; each result
    # shift-clicked out.
    'stonecutter_use': dict(
        setup=[f'setblock 0 {FLOOR + 1} 2 minecraft:stonecutter',
               f'item replace entity {PLAYER} hotbar.0 with minecraft:stone 8'],
        ticks=60, steps={0: ['look 0 45'], 5: ['mouse right click'], 12: ['click-slot 29 shift'], 16: ['menu-button 0'],
                         22: ['click-slot 1 shift'], 30: ['menu-button 1'], 34: ['click-slot 1'], 38: ['click-slot 29'],
                         44: ['key e tap']}),
    'loom_use': dict(
        setup=[f'setblock 0 {FLOOR + 1} 2 minecraft:loom[facing=north]',
               f'item replace entity {PLAYER} hotbar.0 with minecraft:white_banner',
               f'item replace entity {PLAYER} hotbar.1 with minecraft:red_dye 4'],
        ticks=60, steps={0: ['look 0 45'], 5: ['mouse right click'], 12: ['click-slot 31 shift'], 15: ['click-slot 32 shift'],
                         20: ['menu-button 1'], 26: ['click-slot 3 shift'], 36: ['key e tap']}),
    # Shift-clicks route raw iron to the input and coal to the fuel; the furnace starts smelting while open (its
    # progress comes as container data), then the result is taken.
    'furnace_shift': dict(
        setup=[f'setblock 0 {FLOOR + 1} 2 minecraft:furnace[facing=north]{{Items:[{{Slot:2b,'
               'id:"minecraft:iron_ingot",count:3}]}',
               f'item replace entity {PLAYER} hotbar.0 with minecraft:raw_iron 8',
               f'item replace entity {PLAYER} hotbar.1 with minecraft:coal 4'],
        ticks=70, steps={0: ['look 0 45'], 5: ['mouse right click'], 12: ['click-slot 30 shift'],
                         15: ['click-slot 31 shift'], 20: ['click-slot 2 shift'], 50: ['key e tap']}),
    # The player's own inventory: armour by shift-click, number-key and offhand swaps, throwing one and a stack.
    'inventory_clicks': dict(
        setup=[f'item replace entity {PLAYER} hotbar.0 with minecraft:diamond_chestplate',
               f'item replace entity {PLAYER} hotbar.1 with minecraft:cobblestone 20',
               f'item replace entity {PLAYER} inventory.0 with minecraft:bread 5'],
        ticks=60, steps={5: ['key e tap'], 10: ['click-slot 36 shift'], 13: ['click-slot 9 SWAP b8'],
                         16: ['click-slot 37 SWAP b40'], 19: ['click-slot 45 THROW b0'],
                         22: ['click-slot 44 THROW b1'], 25: ['click-slot 6 shift'], 30: ['key e tap']}),
    # Hitting a mob that moves: which entity the crosshair picks, and when, follows how the client
    # interpolates the mob's moves (the server sends them; a replay gives both clients the same). A husk with its
    # AI walks up and hits back (its knockback too); the player strafes and swings, or sprint-hits with w-taps.
    'moving_husk_fight': dict(
        setup=[MACE_RESISTANCE, CHASER, f'item replace entity {PLAYER} weapon.mainhand with minecraft:diamond_sword'],
        ticks=100, steps=merge({0: ['look 0 15', 'key a down'], 25: ['key a up', 'key d down'], 50: ['key d up',
                                'key a down'], 75: ['key a up']}, every(8, 96, 12, 'mouse left click'))),
    'moving_husk_sprint': dict(
        setup=[MACE_RESISTANCE, CHASER, f'item replace entity {PLAYER} weapon.mainhand with minecraft:diamond_sword'],
        ticks=100, steps=merge({0: ['look 0 10', 'key ctrl down', 'key w down']}, every(10, 90, 12, 'mouse left click'),
                               every(12, 92, 12, 'key w up'), every(15, 95, 12, 'key w down'),
                               {95: ['key w up', 'key ctrl up']})),
    # The same with a mace, from the ground: hits on a mob walking in and out of reach.
    'moving_husk_mace': dict(
        setup=[MACE_RESISTANCE, CHASER, f'item replace entity {PLAYER} weapon.mainhand with minecraft:mace'],
        ticks=100, steps=merge({0: ['look 0 15', 'key s down'], 30: ['key s up']}, every(10, 96, 17, 'mouse left click'))),
    # Movement on special blocks and shapes, all client physics a movement-predicting anticheat simulates. Each
    # builds its course in front of the player (+z) inside the area the reset refills.
    'packed_ice_sprint': dict(
        setup=[f'fill -1 {FLOOR} 1 1 {FLOOR} 14 minecraft:packed_ice'],
        ticks=70, steps=merge({0: ['key ctrl down', 'key w down'], 40: ['key w up', 'key ctrl up']},
                              {10: ['key space down'], 12: ['key space up'], 25: ['key space down'], 27: ['key space up']})),
    'blue_ice_sprint': dict(
        setup=[f'fill -1 {FLOOR} 1 1 {FLOOR} 14 minecraft:blue_ice'],
        ticks=70, steps={0: ['key ctrl down', 'key w down'], 20: ['key w up', 'key ctrl up'], 35: ['key s down'],
                         45: ['key s up']}),
    'soul_sand_walk': dict(
        setup=[f'fill -1 {FLOOR} 1 1 {FLOOR} 8 minecraft:soul_sand'],
        ticks=60, steps={0: ['key ctrl down', 'key w down'], 15: ['key space down'], 17: ['key space up'],
                         40: ['key w up', 'key ctrl up']}),
    'honey_walk': dict(
        setup=[f'fill -1 {FLOOR} 1 1 {FLOOR} 8 minecraft:honey_block'],
        ticks=60, steps={0: ['key w down'], 15: ['key space down'], 17: ['key space up'], 40: ['key w up']}),
    # Falling 5 blocks onto slime: the bounces; then onto it again sneaking (no bounce). The player starts on the
    # ground and is put up there from the scenario, so both clients start the fall on the same tick.
    'slime_bounce': dict(
        setup=[f'fill -1 {FLOOR} -1 1 {FLOOR} 1 minecraft:slime_block'],
        ticks=90, steps={0: ['command tp @s 0.5 ~5 0.5'], 45: ['key shift down'], 46: ['command tp @s 0.5 ~5 0.5'],
                         85: ['key shift up']}),
    'cobweb_walk': dict(
        setup=[f'fill 0 {FLOOR + 1} 3 0 {FLOOR + 2} 4 minecraft:cobweb'],
        ticks=70, steps={0: ['key w down'], 20: ['key space down'], 22: ['key space up'], 55: ['key w up']}),
    'powder_snow_walk': dict(
        setup=[f'fill 0 {FLOOR} 3 0 {FLOOR + 1} 4 minecraft:powder_snow'],
        ticks=70, steps={0: ['key w down'], 25: ['key space down'], 35: ['key space up'], 55: ['key w up']}),
    # Walking a long strip of powder snow: the frost (LivingEntity.tryAddFrost, server-side) builds up tick by
    # tick, and the client gets the slowdown from the server's attributes. With leather boots the player walks on
    # it and never freezes: the control.
    'powder_snow_long': dict(
        setup=[f'fill 0 {FLOOR + 1} 2 0 {FLOOR + 1} 12 minecraft:powder_snow'],
        ticks=90, steps={0: ['key w down'], 80: ['key w up']}),
    'powder_snow_boots': dict(
        setup=[f'fill 0 {FLOOR + 1} 2 0 {FLOOR + 1} 12 minecraft:powder_snow',
               f'item replace entity {PLAYER} armor.feet with minecraft:leather_boots'],
        ticks=90, steps={0: ['key w down'], 5: ['key space down'], 7: ['key space up'], 80: ['key w up']}),
    # Up a ladder, held still sneaking halfway, on up, then off backwards.
    'ladder_climb': dict(
        setup=[f'fill 0 {FLOOR + 1} 2 0 {FLOOR + 8} 2 minecraft:stone',
               f'fill 0 {FLOOR + 1} 1 0 {FLOOR + 8} 1 minecraft:ladder[facing=north]'],
        ticks=90, steps={0: ['key w down'], 25: ['key shift down'], 40: ['key shift up'], 55: ['key w up'],
                         56: ['key s down', 'key space down'], 58: ['key space up'], 65: ['key s up']}),
    'scaffolding_climb': dict(
        setup=[f'fill 0 {FLOOR + 1} 1 0 {FLOOR + 6} 1 minecraft:scaffolding'],
        ticks=90, steps={0: ['key w down'], 4: ['key w up', 'key space down'], 35: ['key space up'],
                         45: ['key shift down'], 75: ['key shift up']}),
    # Into a pool, sprint-swimming underwater, up and out over the edge.
    'water_swim': dict(
        setup=[f'fill -3 {FLOOR - 3} 2 3 {FLOOR} 10 minecraft:water'],
        ticks=100, steps={0: ['key w down'], 15: ['look 0 40', 'key ctrl down'], 45: ['look 0 -30'],
                          60: ['key ctrl up', 'key space down'], 85: ['key space up', 'key w up']}),
    # Sprint-jumping under a ceiling two blocks up: every jump hits it.
    'head_hitter': dict(
        setup=[f'fill -1 {FLOOR + 3} 1 1 {FLOOR + 3} 14 minecraft:stone'],
        ticks=60, steps={0: ['key ctrl down', 'key w down', 'key space down'], 40: ['key space up', 'key w up',
                                                                                    'key ctrl up']}),
    # Sprinting up a stair, a slab and a step (0.5 and 1 high), jumping where it must.
    'stairs_slabs_sprint': dict(
        setup=[f'setblock 0 {FLOOR + 1} 3 minecraft:stone_slab', f'setblock 0 {FLOOR + 1} 4 minecraft:stone',
               f'setblock 0 {FLOOR + 2} 5 minecraft:stone_stairs[facing=south]',
               f'fill 0 {FLOOR + 1} 5 0 {FLOOR + 1} 9 minecraft:stone', f'fill 0 {FLOOR + 2} 6 0 {FLOOR + 2} 9 minecraft:stone',
               f'setblock 0 {FLOOR + 3} 7 minecraft:oak_slab'],
        ticks=60, steps={0: ['key ctrl down', 'key w down'], 8: ['key space down'], 9: ['key space up'],
                         40: ['key w up', 'key ctrl up']}),
    # Sneaking to the edge of a raised platform, straight and diagonally: it holds the player on it.
    'sneak_edge': dict(
        setup=[f'fill -1 {FLOOR + 1} -1 1 {FLOOR + 1} 2 minecraft:stone', f'tp {PLAYER} 0.5 {FLOOR + 2} 0.5 0 0'],
        ticks=70, steps={0: ['key shift down', 'key w down'], 30: ['key w up', 'key d down', 'key s down'],
                         55: ['key d up', 'key s up', 'key shift up']}),
    # Diagonally along a wall: horizontal collision each tick.
    'wall_slide': dict(
        setup=[f'fill 2 {FLOOR + 1} -2 2 {FLOOR + 3} 14 minecraft:stone'],
        ticks=60, steps={0: ['look -20 0', 'key ctrl down', 'key w down', 'key d down'], 40: ['key w up', 'key d up',
                                                                                             'key ctrl up']}),
    'speed_jump_effects': dict(
        setup=[f'effect give {PLAYER} minecraft:speed 30 2 true', f'effect give {PLAYER} minecraft:jump_boost 30 2 true'],
        ticks=60, steps={0: ['key ctrl down', 'key w down', 'key space down'], 30: ['key space up'],
                         40: ['key w up', 'key ctrl up']}),
    # The effects come from the scenario's own commands, so both clients start rising on the same tick.
    'levitation_slow_falling': dict(
        ticks=90, steps={0: ['command effect give @s minecraft:levitation 2 1 true', 'key w down'],
                         45: ['command effect give @s minecraft:slow_falling 10 0 true'], 60: ['key w up']}),
    # Combat details and interactions.
    # Critical hits: jumping and swinging on the way down.
    'crit_hits': dict(
        setup=[TARGET.format(x=0.5, y=FLOOR + 1, z=2.6), SWORD],
        ticks=80, steps=merge({0: ['look 0 15']}, {t: ['key space down'] for t in (5, 30, 55)},
                              {t: ['key space up'] for t in (7, 32, 57)}, {t: ['mouse left click'] for t in (12, 37, 62)})),
    # A full-charge hit on the ground beside two mobs: the sweep.
    'sweep_hits': dict(
        setup=[TARGET.format(x=0.5, y=FLOOR + 1, z=2.6), TARGET.format(x=1.6, y=FLOOR + 1, z=2.8), SWORD],
        ticks=70, steps=merge({0: ['look 0 15']}, every(5, 65, 15, 'mouse left click'))),
    # A click every tick (far faster than the cooldown) at a mob in reach.
    'high_cps': dict(
        setup=[TARGET.format(x=0.5, y=FLOOR + 1, z=2.6), SWORD],
        ticks=50, steps=merge({0: ['look 0 15']}, every(5, 45, 1, 'mouse left click'))),
    # A mob at the edge of reach, the player stepping in and out of it while swinging.
    'reach_limit': dict(
        setup=[TARGET.format(x=0.5, y=FLOOR + 1, z=3.85), SWORD],
        ticks=70, steps=merge({0: ['look 0 5'], 10: ['key w down'], 12: ['key w up'], 30: ['key s down'],
                               32: ['key s up']}, every(5, 65, 6, 'mouse left click'))),
    # An end crystal set on obsidian and hit: its explosion's knockback, under Resistance.
    'end_crystal': dict(
        setup=[MACE_RESISTANCE, f'setblock 0 {FLOOR} 3 minecraft:obsidian',
               f'item replace entity {PLAYER} weapon.mainhand with minecraft:end_crystal 4'],
        ticks=60, steps={0: ['look 0 40'], 5: ['mouse right click'], 15: ['mouse left click']}),
    'ender_pearl': dict(
        setup=[f'item replace entity {PLAYER} weapon.mainhand with minecraft:ender_pearl 4'],
        ticks=80, steps={0: ['look 0 -20'], 5: ['mouse right click'], 50: ['mouse right click']}),
    'throw_projectiles': dict(
        setup=[f'item replace entity {PLAYER} hotbar.0 with minecraft:snowball 16',
               f'item replace entity {PLAYER} hotbar.1 with minecraft:egg 16',
               f'item replace entity {PLAYER} hotbar.2 with minecraft:wind_charge 16'],
        ticks=60, steps=merge({0: ['look 0 -10']}, every(5, 20, 4, 'mouse right click'), {22: ['key 2 tap']},
                              every(25, 40, 4, 'mouse right click'), {42: ['key 3 tap']}, {45: ['mouse right click']})),
    'trident_throw': dict(
        setup=[f'item replace entity {PLAYER} weapon.mainhand with minecraft:trident'],
        ticks=60, steps={0: ['look 0 -5'], 5: ['mouse right down'], 20: ['mouse right up']}),
    'crossbow_shoot': dict(
        setup=[f'item replace entity {PLAYER} weapon.mainhand with minecraft:crossbow',
               f'give {PLAYER} minecraft:arrow 16'],
        ticks=80, steps={0: ['look 0 -5'], 5: ['mouse right down'], 35: ['mouse right up'], 45: ['mouse right click'],
                         55: ['mouse right down'], 60: ['mouse right up']}),
    # Holding a shield up while a husk with its AI attacks.
    'shield_block_mob': dict(
        setup=[MACE_RESISTANCE, CHASER, f'item replace entity {PLAYER} weapon.offhand with minecraft:shield'],
        ticks=90, steps={0: ['look 0 10'], 10: ['mouse right down'], 60: ['mouse right up'], 70: ['mouse right down'],
                         85: ['mouse right up']}),
    # Totems swapped to the offhand and back in quick succession.
    'totem_swap': dict(
        setup=[f'item replace entity {PLAYER} hotbar.0 with minecraft:totem_of_undying',
               f'item replace entity {PLAYER} hotbar.1 with minecraft:totem_of_undying'],
        ticks=50, steps={5: ['key f tap'], 7: ['key f tap'], 8: ['key 2 tap'], 9: ['key f tap'], 20: ['key f tap'],
                         21: ['key 1 tap', 'key f tap']}),
    # Two clicks and two hotbar keys in one tick (all handled in that tick).
    'inputs_one_tick': dict(
        setup=[TARGET.format(x=0.5, y=FLOOR + 1, z=2.6), SWORD,
               f'item replace entity {PLAYER} hotbar.1 with minecraft:stone 16'],
        ticks=50, steps={0: ['look 0 15'], 10: ['mouse left click', 'mouse left click'], 20: ['key 2 tap', 'key 1 tap'],
                         30: ['key 2 tap', 'mouse right click', 'mouse right click']}),
    # Holding right click on the ground ahead: vanilla places every 4 ticks (rightClickDelay).
    'held_place': dict(
        setup=[f'item replace entity {PLAYER} weapon.mainhand with minecraft:stone 64'],
        ticks=50, steps={0: ['look 0 60'], 5: ['mouse right down'], 35: ['mouse right up']}),
    # Towering: jumping and placing a block under the player at the top of each jump.
    'tower_up': dict(
        setup=[f'item replace entity {PLAYER} weapon.mainhand with minecraft:stone 64'],
        ticks=80, steps=merge({0: ['look 0 90'], 5: ['key space down'], 70: ['key space up']},
                              every(9, 70, 6, 'mouse right click'))),
    # A water bucket placed under the player just before it lands from 10 blocks, then scooped up.
    'water_clutch': dict(
        setup=[f'item replace entity {PLAYER} weapon.mainhand with minecraft:water_bucket',
               f'tp {PLAYER} 0.5 {FLOOR + 11} 0.5 0 90'],
        ticks=70, steps={0: ['look 0 90'], 24: ['mouse right click'], 26: ['mouse right click'], 40: ['mouse right click']}),
    # Instant mining: Haste II and an Efficiency V pickaxe on a row of stone, walking along it.
    'instant_mine': dict(
        setup=[f'fill 0 {FLOOR + 1} 2 0 {FLOOR + 1} 8 minecraft:stone', f'effect give {PLAYER} minecraft:haste 30 1 true',
               f'item replace entity {PLAYER} weapon.mainhand with minecraft:diamond_pickaxe[minecraft:enchantments={{"minecraft:efficiency":5}}]'],
        ticks=70, steps={0: ['look 0 40', 'mouse left down'], 5: ['key w down'], 55: ['key w up', 'mouse left up']}),
    # An end crystal clicked onto grass (EndCrystalItem.useOn: FAIL) and onto obsidian (SUCCESS), a shield in the
    # other hand: either result ends the click (Minecraft.startUseItem), so no UseItem and no offhand use follows.
    'end_crystal_fail': dict(
        setup=[f'item replace entity {PLAYER} weapon.mainhand with minecraft:end_crystal 4',
               f'item replace entity {PLAYER} weapon.offhand with minecraft:shield'],
        ticks=40, steps={0: ['look 0 40'], 5: ['mouse right click'], 15: ['mouse right click']}),
    'end_crystal_place': dict(
        setup=[f'setblock 0 {FLOOR} 2 minecraft:obsidian',
               f'item replace entity {PLAYER} weapon.mainhand with minecraft:end_crystal 4',
               f'item replace entity {PLAYER} weapon.offhand with minecraft:shield'],
        ticks=40, steps={0: ['look 0 40'], 5: ['mouse right click'], 15: ['mouse right click']}),
    # An armor stand given a helmet with a right click on its head.
    'armor_stand_interact': dict(
        setup=[f'summon minecraft:armor_stand 0.5 {FLOOR + 1} 2.5 {{Rotation:[180f,0f]}}',
               f'item replace entity {PLAYER} weapon.mainhand with minecraft:iron_helmet'],
        ticks=40, steps={0: ['look 0 -20'], 5: ['mouse right click'], 20: ['look 0 -20'], 25: ['mouse right click']}),
    # A wind charge at the feet launches the player; the mace comes out and smashes on the way down.
    'mace_wind_charge': dict(
        setup=[TARGET.format(x=0.5, y=FLOOR + 1, z=2.4), MACE_RESISTANCE,
               f'item replace entity {PLAYER} hotbar.0 with minecraft:mace',
               f'item replace entity {PLAYER} hotbar.1 with minecraft:wind_charge 8'],
        ticks=90, steps=merge({0: ['key 2 tap', 'look 0 90'], 4: ['mouse right click'], 8: ['key 1 tap'],
                               12: ['look 0 55']}, every(20, 80, 3, 'mouse left click'))),
    # A TNT's explosion beside the player (the explode packet's knockback), under Resistance; its fuse outlasts the
    # reset's settling. Last: the crater
    # stays below the floor the reset refills.
    'tnt_knockback': dict(
        setup=[MACE_RESISTANCE, f'summon minecraft:tnt 0.5 {FLOOR + 1} 3.0 {{fuse:80}}'],
        ticks=60, steps={}),
}


# WIRE_STREAM_RANDOM=SEED,COUNT: COUNT generated scenarios, random_SEED_N, in an arena with steps, a pillar,
# a gap, water and a target, each 120 ticks of random walking, sprinting, jumping, sneaking, turning, clicking and
# hotbar changes. The same seed gives the same scenarios, so a difference found can be run again.
def random_scenarios(seed, count):
    import random
    rng = random.Random(seed)
    arena = [f'fill -3 {FLOOR + 1} 4 -1 {FLOOR + 1} 6 minecraft:stone', f'fill 2 {FLOOR + 1} 3 2 {FLOOR + 3} 3 minecraft:oak_planks',
             f'fill -2 {FLOOR} 9 2 {FLOOR} 10 minecraft:air', f'fill 3 {FLOOR} 7 5 {FLOOR} 9 minecraft:water',
             f'setblock -2 {FLOOR + 2} 5 minecraft:oak_slab', TARGET.format(x=0.5, y=FLOOR + 1, z=8.5), SWORD,
             f'item replace entity {PLAYER} hotbar.1 with minecraft:oak_planks 64']
    out = {}
    for n in range(count):
        held, steps = set(), {}
        for tick in range(0, 110, 2):
            if rng.random() < 0.45:
                continue
            kind = rng.choice(['move', 'move', 'jump', 'sprint', 'sneak', 'look', 'look', 'attack', 'use', 'slot'])
            line = []
            if kind == 'move':
                key = rng.choice('wasd')
                line.append(f'key {key} {"up" if key in held else "down"}')
                held ^= {key}
            elif kind == 'jump':
                line.append('key space tap')
            elif kind in ('sprint', 'sneak'):
                key = 'ctrl' if kind == 'sprint' else 'shift'
                line.append(f'key {key} {"up" if key in held else "down"}')
                held ^= {key}
            elif kind == 'look':
                line.append(f'look {rng.randrange(-60, 61)} {rng.randrange(-30, 61)}')
            elif kind == 'attack':
                line.append('mouse left click')
            elif kind == 'use':
                line.append('mouse right click')
            else:
                line.append(f'key {rng.choice("12")} tap')
            steps.setdefault(tick, []).extend(line)
        steps.setdefault(112, []).extend(f'key {k} up' for k in sorted(held))
        out[f'random_{seed}_{n}'] = dict(setup=arena, ticks=120, steps=steps)
    return out


if os.environ.get('WIRE_STREAM_RANDOM'):
    _seed, _count = os.environ['WIRE_STREAM_RANDOM'].split(',')
    SCENARIOS.update(random_scenarios(int(_seed), int(_count)))

# What a run plays unless --scenarios names others (`all`: every scenario above, about two hours): the first
# sixteen, then the moves an anticheat's movement, combat, block and inventory checks read closest, about ten
# minutes in all.
DEFAULT = [
    'walk', 'idle', 'sprint_jump', 'strafe', 'w_tap', 'sneak', 'hit_target', 'sprint_combo', 'block_hit', 'rod',
    'bow', 'eat', 'bridge', 'inventory', 'hotbar_swap', 'dig_teleport',
    'knockback_sprint', 'kb_on_ice', 'attack_cooldown_edges', 'sprint_reset_taps', 'crit_jump_falling', 'high_cps',
    'reach_limit', 'break_quick_and_abort', 'place_faces_and_reach', 'tower_up', 'ninja_bridge', 'mace_fall_10',
    'ender_pearl', 'mouse_flicks', 'mouse_turn_walk_attack', 'chest_clicks', 'chest_drag', 'inventory_clicks',
    'ladder_climb', 'water_swim',
]


def sunrise_lines(lines):
    """A scenario's lines for Sunrise's tick script, which takes them as the vanilla driver does: click-slot names
    the menu's own slot (game/world/tick_script.rs)."""
    return list(lines)


def script(name, spec, sunrise):
    steps = {0: ['key 1 tap', 'key w down'], 2: ['key w up']}
    # `sunrise_steps`: what Sunrise does instead (a controller), where vanilla has only the keyboard and mouse.
    for tick, lines in (spec.get('sunrise_steps', spec['steps']) if sunrise else spec['steps']).items():
        steps.setdefault(tick + PRELUDE, []).extend(sunrise_lines(lines) if sunrise else lines)
    return {'name': name, 'ticks': spec['ticks'] + PRELUDE, 'steps': {str(k): v for k, v in sorted(steps.items())}}


# WIRE_STREAM_GRIM=DIR: the server is Paper with the Grim anticheat (DIR holds paper-<version>-*.jar and
# grimac-bukkit-*.jar, downloaded by hand: the harness fetches nothing); each client's Grim violations are counted
# after the run.


def grim_server(directory, jars):
    """The command for Paper and Grim in `directory`. Paperclip patches the Mojang server jar it finds in its cache,
    so it is given the prepared copy (the same jar its download-context names) and downloads nothing."""
    servers = Path(os.environ['WIRE_STREAM_GRIM'])
    # WIRE_STREAM_VIA=VERSION: an older Paper with ViaVersion, which the 26.3 clients join through.
    via = os.environ.get('WIRE_STREAM_VIA')
    paper = next(servers.glob(f'paper-{via or lab.VERSION}-*.jar'))
    (directory / 'cache').mkdir()
    if not via:
        # The bundled server.jar the user imported (jars['server'] is the unbundled game jar inside it); an older
        # Paper's paperclip fetches its own.
        shutil.copy(jars['server'].with_name('server.jar'), directory / 'cache' / f'mojang_{lab.VERSION}.jar')
    (directory / 'plugins').mkdir()
    shutil.copy(next(servers.glob('grimac-bukkit-*.jar')), directory / 'plugins')
    if via:
        shutil.copy(next(servers.glob('ViaVersion-*.jar')), directory / 'plugins')
    shutil.copy(paper, directory / 'paper.jar')
    return ['java', '-Xmx1536m', '-jar', 'paper.jar', 'nogui']


def grim_violations(db, clients):
    """Each client's Grim violations, from Grim's own history database: it logs every one (punishments.yml
    `1:1 [log]`) but prints an alert only from a violation level of 100 (`100:40 [alert]`), so the console shows
    small deviations never. The clients joined one after the other: their sessions, in order."""
    import sqlite3
    con = sqlite3.connect(db)
    checks = dict(con.execute('select check_id, display from grim_checks'))
    sessions = [row[0] for row in con.execute('select session_id from grim_sessions order by started_at')]
    out = {}
    for who, session in zip(clients, sessions):
        rows = con.execute('select check_id, vl from grim_violations where session_id = ? order by occurred_at',
                           (session,)).fetchall()
        out[who] = [(checks.get(check, str(check)), vl) for check, vl in rows]
    con.close()
    return out


class Failed(Exception):
    pass


MESA_EGL = Path('/run/opengl-driver/share/glvnd/egl_vendor.d/50_mesa.json')


def screen_env(env):
    """Where the vanilla client's window goes. On Linux, the virtual screen (`vscreen`), never a real monitor,
    and Mesa's EGL: NVIDIA's crashed the client there (libEGL_nvidia). On macOS, the logged-in session."""
    if sys.platform == 'darwin' or not shutil.which('vscreen'):
        return env
    subprocess.run(['vscreen', 'up'], check=False, capture_output=True)
    text = subprocess.run(['vscreen', 'env'], capture_output=True, text=True, check=True).stdout
    env = dict(env)
    env.pop('DISPLAY', None)
    for match in re.finditer(r'(\w+)=(\S+)', text.split('unset')[0]):
        env[match.group(1)] = match.group(2).strip('"\'')
    if MESA_EGL.exists():
        env['__EGL_VENDOR_LIBRARY_FILENAMES'] = str(MESA_EGL)
    return env


class VanillaClient:
    """The vanilla 26.3 client, VanillaTickScript as its main class, joined to `address`."""

    def __init__(self, address, work, log):
        import vanilla_client
        classes = work / 'classes'
        classes.mkdir(parents=True)
        jars = os.pathsep.join(map(str, vanilla_client.classpath()))
        release = str(lab.config()[0]['minecraft']['java_major'])
        source = lab.ROOT / 'mods/sunrise/src/test/java/dev/sunriselab/parity/VanillaTickScript.java'
        subprocess.run(['javac', '--release', release, '-proc:none', '-nowarn', '-cp', jars, '-d', classes, source],
                       check=True, capture_output=True)
        args = argparse.Namespace(name=PLAYER, join=address, world=None, game_dir=work / 'client', assets=None,
                                  heap='2G', width=854, height=480, lan_port=None, backend=None)
        args.game_dir.mkdir()
        # 20 frames a second at least: a tick's lines then act in their own frame, as Sunrise's do. On a virtual
        # screen the frames are drawn on the CPU: the lightest graphics keep each well under a tick, or the client
        # runs its missed ticks in bursts and takes the server's packets on other ticks than a smooth one would.
        vanilla_client.write_options(args.game_dir, {
            'maxFps': '60', 'graphicsMode': '0', 'renderClouds': '"false"', 'ao': 'false',
            'entityShadows': 'false', 'biomeBlendRadius': '0', 'mipmapLevels': '0',
            **extra_options()})
        command = vanilla_client.command(args)
        cp = command.index('-cp') + 1
        command[cp] = os.pathsep.join([str(classes), command[cp]])
        command[command.index('net.minecraft.client.main.Main')] = 'dev.sunriselab.parity.VanillaTickScript'
        self.log = open(log, 'w', encoding='utf-8')
        self.ready, self.done = threading.Event(), {}
        # Its window opens without the focus (SDL's hint), so it takes neither the keyboard nor the pointer of
        # whoever is at the computer; VanillaTickScript keeps their input out once it runs.
        env = dict(screen_env(vanilla_client.environment()), SDL_WINDOW_ACTIVATE_WHEN_SHOWN='0')
        self.process = vanilla_client.spawn(command, args.game_dir, env)
        threading.Thread(target=self.pump, daemon=True).start()

    def pump(self):
        for line in self.process.stdout:
            self.log.write(line)
            self.log.flush()
            if '[VanillaTickScript] ready' in line:
                self.ready.set()
            elif '[VanillaTickScript] done ' in line:
                self.done[line.split('[VanillaTickScript] done ')[1].strip()] = time.time()

    def play(self, path, name, timeout):
        self.process.stdin.write(f'run {path}\n')
        self.process.stdin.flush()
        deadline = time.time() + timeout
        while name not in self.done:
            if time.time() > deadline or self.process.poll() is not None:
                raise Failed(f'vanilla: scenario {name} never finished (see vanilla-client.log)')
            time.sleep(0.2)

    def close(self):
        self.process.terminate()
        try:
            self.process.wait(20)
        except subprocess.TimeoutExpired:
            self.process.kill()
        self.log.close()


def serve_resource_pack(folder):
    """WIRE_STREAM_RESOURCE_PACK=1: a required server resource pack (its own pack.mcmeta only), served
    over HTTP from `folder`: the server.properties entries that offer it. Each client downloads and applies it in
    configuration, and its ResourcePack answers (accepted, downloaded, loaded) are what is compared."""
    import hashlib
    import http.server
    import zipfile
    folder.mkdir(parents=True)
    pack = folder / 'wire.zip'
    with zipfile.ZipFile(pack, 'w') as z:
        z.writestr('pack.mcmeta', json.dumps({'pack': {'description': 'wire stream test pack', 'min_format': 97,
                                                       'max_format': 97}}))
    port = free_port()
    handler = lambda *a, **k: http.server.SimpleHTTPRequestHandler(*a, directory=str(folder), **k)
    server = http.server.ThreadingHTTPServer(('127.0.0.1', port), handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    return {'resource-pack': f'http\\://127.0.0.1\\:{port}/wire.zip',
            'resource-pack-sha1': hashlib.sha1(pack.read_bytes()).hexdigest(), 'require-resource-pack': 'true'}


# What both clients tell the server in ClientboundClientInformation and the server acts on: the view distance
# (how many chunks it sends) and the particle level. The vanilla client runs light (2, minimal) to keep its frames
# short on a virtual screen; Sunrise starts with the same, or the two get different chunks.
SHARED_OPTIONS = {'renderDistance': '2', 'particles': '2'}


def extra_options():
    """SHARED_OPTIONS, then WIRE_STREAM_OPTIONS=key:value,...: options.txt entries both clients start with (a
    sensitivity, auto-jump, toggled sprint and sneak, an inverted mouse), in vanilla's names and form."""
    spec = os.environ.get('WIRE_STREAM_OPTIONS', '')
    return {**SHARED_OPTIONS, **dict(item.split(':', 1) for item in spec.split(',') if ':' in item)}


class HeadlessGame(Game):
    """server_pack_e2e's game, `--headless`: no display, window or GPU; the frame and the ticks run as a
    window's do (game/runtime/headless.rs), and only the ticks' packets matter here."""

    def __init__(self, bins, resources, work, scripts, extras=None):
        self.home = work / 'home'
        for d in ('home', 'launcher', 'data', 'config'):
            (work / d).mkdir(parents=True)
        if extras:
            # The game folder is the home here (Game runs sunrise-ctl there): extras sit next to options.txt.
            (self.home / 'sunrise-extras.txt').write_text(extras)
        # The same options as the vanilla client's, in vanilla's options.txt form.
        (self.home / 'options.txt').write_text(''.join(f'{k}:{v}\n' for k, v in extra_options().items()))
        self.env = lab.game_env(SUNRISE_HOME=self.home, SUNRISE_LAUNCHER_HOME=work / 'launcher',
                                XDG_DATA_HOME=work / 'data', XDG_CONFIG_HOME=work / 'config',
                                SUNRISE_TELEMETRY_URL='off', SUNRISE_TICK_SCRIPT=scripts / 'scenario.json')
        self.base = [str(lab.binary(bins, 'sunrise-ctl')), '--socket', str(work / 'c.sock')]
        self.log = self.home / 'logs' / 'latest.log'
        headless = [] if os.environ.get('WIRE_STREAM_WINDOWED') == '1' else ['--headless']
        reply = self('launch', '--game', lab.binary(bins, 'sunrise-client'), *headless, '--resources', resources)
        self.pid = reply.get('pid')


class SunriseClient:
    def __init__(self, bins, resources, work, address, scripts, extras=None):
        self.scripts = scripts
        self.game = HeadlessGame(bins, resources, work, scripts, extras)
        self.game('connect', address, PLAYER)
        self.game('wait-for', 'loaded', '--timeout', 180)

    def play(self, path, name, timeout):
        target = self.scripts / 'scenario.json'
        done = target.with_suffix('.done')
        done.unlink(missing_ok=True)
        shutil.copy(path, target)
        target.with_suffix('.go').touch()
        deadline = time.time() + timeout
        while not done.exists():
            if time.time() > deadline:
                raise Failed(f'Sunrise: scenario {name} never finished (see sunrise-client.log)')
            time.sleep(0.2)

    def close(self):
        self.game('disconnect', ok=False)
        self.game.quit()


class ClientCounts:
    """How many teleports the client has accepted, and how many ticks it has ended, read from its capture as it
    grows (complete lines only)."""

    def __init__(self, capture):
        self.capture, self.offset, self.accepted, self.ticks = capture, 0, 0, 0
        # The player's entity id (the play Login's), the vehicle the server last set it riding (SetPassengers), and
        # the dimensions of the Respawns it was sent, in order.
        self.me, self.vehicle, self.respawns = None, None, []

    def update(self):
        with open(self.capture, 'rb') as f:
            f.seek(self.offset)
            data = f.read()
        data = data[:data.rfind(b'\n') + 1]
        self.offset += len(data)
        for line in data.splitlines():
            if b'"c2s"' in line:
                e = json.loads(line)
                self.accepted += e.get('name') == 'accept_teleportation'
                self.ticks += e.get('name') == 'client_tick_end'
            elif b'"s2c"' in line and (b'"login"' in line or b'"set_passengers"' in line or b'"respawn"' in line):
                e = json.loads(line)
                if e.get('state') != 'play' or 'data' not in e:
                    continue
                payload = bytes.fromhex(e['data'])
                if e['name'] == 'login':
                    self.me = wire_stream.self_id([e])
                elif e['name'] == 'set_passengers' and self.me is not None:
                    r = wire_stream.Reader(payload)
                    vehicle = r.varint()
                    passengers = [r.varint() for _ in range(r.varint())]
                    if self.me in passengers:
                        self.vehicle = vehicle
                    elif vehicle == self.vehicle:
                        self.vehicle = None
                elif e['name'] == 'respawn':
                    # CommonPlayerSpawnInfo: the dimension type (a registry id), then the dimension's key.
                    r = wire_stream.Reader(payload)
                    r.varint()
                    self.respawns.append(r.string())
        return self


def settle(counts, accepted, timeout=120):
    """Wait until the client has accepted `accepted` teleports in all, then for SETTLE more of its ticks. The
    last command of a reset is a teleport, so the server has run them all by then; a busy server can take
    seconds to, and a teleport landing in a scenario's first ticks would change all of its moves."""
    deadline = time.time() + timeout
    while counts.update().accepted < accepted:
        if time.time() > deadline:
            raise Failed(f'{counts.capture.stem}: the client never accepted the reset\'s teleports')
        time.sleep(0.2)
    ticks = counts.ticks
    while counts.update().ticks < ticks + SETTLE:
        if time.time() > deadline:
            raise Failed(f'{counts.capture.stem}: the client stopped ticking')
        time.sleep(0.2)


def saved_logout(server, left_before, timeout=60):
    """Wait until the server has logged the player out (its `left the game` line after `left_before` of them) and
    saved the world: PlayerList.remove writes the player's file on the server thread after that line, and
    `save-all flush` runs on the same thread after it, so its `Saved the game` comes once the file is written."""
    deadline = time.time() + timeout
    while sum(f'{PLAYER} left the game' in line for line in server.lines) <= left_before:
        if time.time() > deadline:
            raise Failed(f'the server never logged {PLAYER} out')
        time.sleep(0.2)
    saved_before = sum('Saved the game' in line for line in server.lines)
    server.console('save-all flush')
    while sum('Saved the game' in line for line in server.lines) <= saved_before:
        if time.time() > deadline:
            raise Failed('the server never saved after the logout')
        time.sleep(0.2)


def forget_player(directory, level):
    """The next client joins as a new player: Steve's saved data from this one (last death, portal cooldown,
    spawn point, stats, advancements) would otherwise reach it in its first Login and Respawn, and the server would
    treat it differently from the first client (end_portal: a Respawn with the first client's death location, and
    the portal taken at another tick). Only the scratch server's world: the one run() started."""
    # 26.3 keeps them under players/ (LevelResource.PLAYER_DATA_DIR...); 1.21.11 (behind ViaVersion) at the top.
    for kind in ('players/data', 'players/stats', 'players/advancements', 'playerdata', 'stats', 'advancements'):
        for saved in (directory / level / kind).glob('*'):
            if saved.is_file():
                saved.unlink(missing_ok=True)

def play_all(client, server, scenarios, scripts, sunrise, capture):
    counts = ClientCounts(capture)
    for name in scenarios:
        spec = SCENARIOS[name]
        # The place settles: blocks and items reach the client, the player stands still on the ground.
        # From the console `~` is the world spawn: the player teleports onto itself.
        # WIRE_STREAM_SERVER_TPS: the server answers within one of its (shorter) ticks, so an answer reaches both
        # clients on the same client tick instead of whichever one two independent 50 ms clocks happen to give.
        # The clients stay at 20 ticks a second: a server's client only ever slows down (getTickTargetMillis).
        # The server's floating kick counts its own ticks (80): at a faster tick rate a smash's or Wind Burst's
        # airtime would end in it, so these runs set allow-flight (server-side only; the clients send the same).
        tps = os.environ.get('WIRE_STREAM_SERVER_TPS')
        commands = ([f'tick rate {tps}'] if tps else []) + RESET + spec.get('setup', []) + [f'tp {PLAYER} {PLAYER}']
        accepted = counts.update().accepted + sum(c.startswith('tp ') for c in commands)
        for command in commands:
            server.console(command)
        settle(counts, accepted)
        if spec.get('mount'):
            mount(server, counts, spec['mount'])
        respawns = len(counts.update().respawns)
        path = scripts / f'{name}-{"sunrise" if sunrise else "vanilla"}.json'
        path.write_text(json.dumps(script(name, spec, sunrise)))
        client.play(path, name, timeout=max(60, spec['ticks'] / 20 + 30))
        if spec.get('enters') and spec['enters'] not in counts.update().respawns[respawns:]:
            raise Failed(f'{counts.capture.stem}: {name} never took the player into {spec["enters"]}')
        time.sleep(0.5)


# Client ticks between the mount's SetPassengers and the scenario: the vehicle's first moves are the client's.
MOUNT_SETTLE = 10


def mount(server, counts, commands, timeout=60):
    """`mount`: commands that seat the player, run after the settle's teleport (a teleport takes the player off
    its vehicle); the scenario starts MOUNT_SETTLE ticks after the server set the player riding."""
    counts.update().vehicle = None
    for command in commands:
        server.console(command)
    deadline = time.time() + timeout
    while counts.update().vehicle is None:
        if time.time() > deadline:
            raise Failed(f'{counts.capture.stem}: the server never set the player riding')
        time.sleep(0.1)
    ticks = counts.ticks
    while counts.update().ticks < ticks + MOUNT_SETTLE:
        if time.time() > deadline:
            raise Failed(f'{counts.capture.stem}: the client stopped ticking')
        time.sleep(0.1)


# WIRE_STREAM_TRANSFER=1: after the scenarios the server transfers each client back to its own
# capture proxy (ClientboundTransferPacket; accepts-transfers), and the second connection the client makes is
# compared: its handshake, login and configuration packets, then its first play ticks.
TRANSFER_TICKS = 60


def transfer_back(server, listen, capture):
    server.console(f'transfer 127.0.0.1 {listen} {PLAYER}')
    deadline = time.time() + 90
    while time.time() < deadline:
        ticks = sum(e.get('conn') == 2 and e.get('name') == 'client_tick_end' for e in net_capture.load(capture))
        if ticks >= TRANSFER_TICKS:
            return
        time.sleep(1)
    raise Failed(f'{capture.stem}: no {TRANSFER_TICKS} play ticks on the connection after the transfer')


def transfer_stream(capture):
    """The client's packets on the connection after the transfer: those before play, then its play ticks' up to
    TRANSFER_TICKS."""
    out, ticks = [], 0
    for e in net_capture.load(capture):
        if e.get('conn') != 2 or e.get('dir') != 'c2s' or 'name' not in e:
            continue
        if e.get('state') == 'play':
            if ticks >= TRANSFER_TICKS:
                break
            ticks += e['name'] == 'client_tick_end'
        data = e.get('data', '')
        if e['name'] == 'intention':
            # Each client's proxy listens on its own port: the intent (3, a transfer) is what the client chose.
            data = f'intent {data[-2:]}'
        elif e['name'] in ('keep_alive', 'chunk_batch_received') or e.get('state') == 'play':
            # The server's ids and the chunk rate the client measured differ run to run; in play the order counts.
            data = ''
        out.append(f"{e.get('state')} {e['name']} {data}".rstrip())
    return out


def run(args, out):
    lock, jars, libraries = base.verify_artifacts()
    deps = os.pathsep.join(map(str, [jars['server'], *libraries]))
    vanilla = ['java', '-Xmx1g', '-cp', deps, 'net.minecraft.server.Main', 'nogui']
    resources = lab.LOCAL / 'sunrise-client' / lab.VERSION
    work = Path(tempfile.mkdtemp(prefix='hz-wire-stream-'))
    print(f'scratch: {work}')
    for var, sub in (('SUNRISE_HOME', 'home'), ('SUNRISE_LAUNCHER_HOME', 'launcher'), ('XDG_DATA_HOME', 'data'),
                     ('XDG_CONFIG_HOME', 'config')):
        (work / 'env' / sub).mkdir(parents=True)
        os.environ[var] = str(work / 'env' / sub)
    os.environ.update(lab.game_env(SUNRISE_TELEMETRY_URL='off'))
    directory = work / 'server'
    directory.mkdir()
    port = free_port()
    props = dict(PROPERTIES, **{'server-port': str(port), 'server-ip': '127.0.0.1'})
    if os.environ.get('WIRE_STREAM_SERVER_TPS'):
        props['allow-flight'] = 'true'
    if os.environ.get('WIRE_STREAM_RESOURCE_PACK'):
        props.update(serve_resource_pack(work / 'pack'))
    if os.environ.get('WIRE_STREAM_TRANSFER'):
        props['accepts-transfers'] = 'true'
    (directory / 'server.properties').write_text(''.join(f'{k}={v}\n' for k, v in props.items()),
                                                 encoding='iso-8859-1')
    (directory / 'eula.txt').write_text('eula=true\n')
    (directory / 'ops.json').write_text(json.dumps([{'uuid': str(lab.offline_uuid(PLAYER)), 'name': PLAYER,
                                                     'level': 4, 'bypassesPlayerLimit': False}]))
    scripts = work / 'scripts'
    scripts.mkdir()
    if os.environ.get('WIRE_STREAM_GRIM'):
        vanilla = grim_server(directory, jars)
    server = Server(vanilla, directory, 'server')
    captures = {}
    try:
        server.wait_for(r'Done \(', timeout=900)
        for who in args.clients:
            capture = out / f'{who}.jsonl'
            capture.touch()
            listen = free_port()
            # WIRE_STREAM_PHASE (seconds): the server's packets reach the client that long after each of its
            # tick-ends (net_capture's phase), so wire_replay.py --phase can give Sunrise them on the same ticks.
            phase = float(os.environ['WIRE_STREAM_PHASE']) if os.environ.get('WIRE_STREAM_PHASE') else None
            stop = net_capture.start_background(listen, f'127.0.0.1:{port}', capture, full=True, phase=phase)
            client = None
            left_before = sum(f'{PLAYER} left the game' in line for line in server.lines)
            try:
                if who != 'vanilla':
                    # WIRE_STREAM_EXTRAS=key:value,...: the Sunrise client's sunrise-extras.txt.
                    extras = os.environ.get('WIRE_STREAM_EXTRAS', '').replace(',', '\n') or None
                    client = SunriseClient(Path(args.bin).resolve(), resources, work / who, f'127.0.0.1:{listen}',
                                           scripts, LEGACY_EXTRAS if who == 'sunrise-legacy' else extras)
                else:
                    client = VanillaClient(f'127.0.0.1:{listen}', work / 'vanilla', out / 'vanilla-client.log')
                    if not client.ready.wait(900):
                        raise Failed('the vanilla client never got into the world')
                time.sleep(3)
                play_all(client, server, args.scenarios, scripts, who != 'vanilla', capture)
                if os.environ.get('WIRE_STREAM_TRANSFER'):
                    transfer_back(server, listen, capture)
            finally:
                if client:
                    if who != 'vanilla' and client.game.log.exists():
                        shutil.copy(client.game.log, out / f'{who}-client.log')
                    client.close()
                stop.set()
            # Before the next client joins: its first Login must not carry this one's saved player.
            if client:
                saved_logout(server, left_before)
            forget_player(directory, props.get('level-name', 'world'))
            captures[who] = capture
    finally:
        server.stop()
        shutil.copy(server.log_path, out / server.log_path.name)
        grim_db = directory / 'plugins' / 'GrimAC' / 'data' / 'history.v1.db'
        if grim_db.exists():
            shutil.copy(grim_db, out / 'grim.db')
        if not args.keep:
            shutil.rmtree(work, ignore_errors=True)
    return captures


def anchored_diffs(v, h, ticks, anchor, problems, names=('vanilla', 'Sunrise')):
    """wire_stream.compare over the scenario's ticks. With an anchor (a frame packet the server's answer brings,
    as AcceptTeleportation), the two ticks that answer arrived in compare with each other
    (wire_stream.compare_answered). Numbers are vanilla's ticks."""
    if not anchor:
        return wire_stream.compare(v, h, ticks)
    av, ah = wire_stream.first_frame(v, anchor), wire_stream.first_frame(h, anchor)
    if av is None or ah is None:
        problems.append(f'  no {anchor}: {names[0]} at tick {av}, {names[1]} at tick {ah}')
        return wire_stream.compare(v, h, ticks)
    return wire_stream.compare_answered(v, h, ticks, av, ah)


def compare(captures, scenarios, replayed=False, uncertain=None):
    """Each scenario's problems, the second client's stream against the first's (`captures` in that order).

    `replayed`: both clients got the same server stream (wire_replay.py), checked here, so nothing is renumbered or
    realigned: each tick is one ordered stream, every packet in its place (the frame's and the network thread's
    answers included), compared byte for byte (wire_stream's `exact`). A tick that differs only in where the
    network thread's answers sit, or in a measured body, is not verified (inconclusive). `uncertain` is the
    replay's timing record ({scenario: [why its delivery ticks are not known to be vanilla's]}, REPLAY_TIMING): such
    a scenario is inconclusive, its differences listed; None (no record) leaves every scenario inconclusive.

    Both: a client that didn't finish the scenario's measured window (a short capture) fails it.

    Live (not `replayed`): ids and sequences are renumbered per client, a scenario's answer tick is realigned
    (`anchor`) and the frame's answers compare by name and order only; a live match is not an exact pass."""
    (ref, ref_name), (new, new_name) = [(who, CLIENTS[who]) for who in captures]
    runs, inputs = {}, {}
    for who, capture in captures.items():
        entries = [e for e in net_capture.load(capture) if e.get('conn') == 1 and 'name' in e]
        found = wire_stream.scenarios(entries, exact=replayed)
        if len(found) != len(scenarios):
            raise Failed(f'{who}: {len(found)} marked scenarios in the capture, expected {len(scenarios)}')
        runs[who] = dict(zip(scenarios, found))
        if replayed:
            inputs[who] = wire_stream.shared_input(entries)
    if replayed and inputs[ref] != inputs[new]:
        at = next(i for i, (a, b) in enumerate(itertools.zip_longest(inputs[ref], inputs[new])) if a != b)
        raise Failed(f'the replayed captures did not get the same server packets (first difference at play packet '
                     f'{at}): nothing compares exactly')
    results = []
    for name in scenarios:
        v, h = runs[ref][name], runs[new][name]
        ticks = SCENARIOS[name]['ticks'] + PRELUDE
        problems = [f'  {who} ended {max(len(run) - 1, 0)} ticks into the {ticks + 1}-tick measured window: the '
                    'capture is short' for who, run in ((ref_name, v), (new_name, h))
                    if not wire_stream.complete(run, ticks)]
        if replayed:
            if uncertain is None:
                problems.append(f'{INCONCLUSIVE} no delivery-timing record with the replay capture '
                                f'({REPLAY_TIMING}): when Sunrise took the server\'s packets is unknown')
            for why in (uncertain or {}).get(name, []):
                problems.append(f'{INCONCLUSIVE} {why}')
            diffs = wire_stream.compare(v, h, ticks, key='wire')
        else:
            diffs = anchored_diffs(v, h, ticks, SCENARIOS[name].get('anchor'), problems, (ref_name, new_name))
        wire = lambda run, i: run[i]['wire'] if i < len(run) else None  # noqa: E731
        for i, vt, ht in diffs:
            if replayed and wire_stream.interleaving_only(wire(v, i), wire(h, i)):
                answers = [n for n, _ in wire(v, i) if n in wire_stream.NETWORK]
                problems.append(f'{INCONCLUSIVE} tick {i}: the network thread\'s answers {answers} sit elsewhere '
                                'among the tick\'s packets, which its own timing decides: this order is not verified')
                continue
            problems.append(f'  tick {i}:\n  {ref_name}:\n{wire_stream.describe(vt)}\n  {new_name}:\n'
                            f'{wire_stream.describe(ht)}')
            problems += wire_stream.field_diff(vt, ht, (ref_name, new_name), (wire(v, i), wire(h, i)))
        if replayed:
            problems += network_answers(v, h, ticks, (ref_name, new_name))
        else:
            # The server decides when keep-alives and chunk batches come; the others answer what the scenario did.
            # Order only: a diagnostic of the live run, not an exact comparison.
            frame = lambda run: [n for t in run[:ticks + 1] for n, _ in t['frame']  # noqa: E731
                                 if n not in ('keep_alive', 'chunk_batch_received')]
            if frame(v) != frame(h):
                problems.append(f'  frame packets: {ref_name} {frame(v)}, {new_name} {frame(h)}')
            # An answer can belong inside a tick for both (a teleport's handler sends a dig abort between two
            # pongs): only a difference is one.
            misplaced = lambda run: [(i, t['misplaced']) for i, t in enumerate(run[:ticks + 1]) if t['misplaced']]  # noqa: E731
            if misplaced(h) != misplaced(v):
                problems.append(f'  answers inside a tick: {ref_name} {misplaced(v)}, {new_name} {misplaced(h)}')
        results.append((name, problems))
    return results


def network_answers(v, h, ticks, names):
    """Replay: the measured bodies the ordered stream leaves out (wire_stream.MEASURED), answer by answer. Their
    ticks and order are the stream's; a body that differs is each client's own measurement, not verified
    (inconclusive)."""
    ref_name, new_name = names
    answers = lambda run: [(i, n, b) for i, t in enumerate(run[:ticks + 1]) for n, b in t['frame_wire']  # noqa: E731
                           if n in wire_stream.MEASURED]
    measured = [(x, y) for x, y in zip(answers(v), answers(h)) if x[2] != y[2]]
    return [f'{INCONCLUSIVE} {x[1]} at tick {x[0]} carries each client\'s own timing measurement, not verified: '
            f'{ref_name} {x[2]}, {new_name} {y[2]}' for x, y in measured]


# Beside a replay's sunrise.jsonl: its delivery-timing record (wire_replay.py), read again by --compare-only.
REPLAY_TIMING = 'timing.json'
INCONCLUSIVE = '  inconclusive:'


def verdict(problems, how):
    """A scenario's verdict, `how` it was compared ('live' or 'replay'). Neither is an exact pass: a live comparison
    realigns the server's answers, and a replay infers the tick each client handled a server packet on from the
    proxy's timestamps; no client reports the tick it handled it on (an accepted-tick observation). So no
    difference is a `match`, a diagnostic; one is a FAIL, or INCONCLUSIVE when the comparison can't vouch for it."""
    if not problems:
        return f'match ({how})'
    return 'INCONCLUSIVE' if any(p.startswith(INCONCLUSIVE) for p in problems) else 'FAIL'


def timing_record(path, scenarios):
    """The replay's delivery-timing record at `path`, checked whole: written for exactly these scenarios, in their
    order, each doubt a reason under one of them. Its {scenario: [why]}."""
    try:
        return wire_stream.timing_doubts(json.loads(path.read_text()), scenarios)
    except (OSError, ValueError) as e:
        raise Failed(f'{path}: {e}')


def replay_evidence(out, scenarios):
    """A replay's Sunrise capture and delivery-timing record, wherever they are: OUT/replay/ (this harness's
    replay) or OUT itself (wire_replay.py run alone writes vanilla.jsonl, sunrise.jsonl and REPLAY_TIMING into its
    --out). Captures whose clients got the same server play packets are a replay with or without the record:
    (Sunrise's capture, {scenario: [why]} or None when the record is missing, which leaves every scenario
    inconclusive). None for a live run's captures."""
    nested = out / 'replay'
    folder = nested if (nested / 'sunrise.jsonl').exists() else out
    capture, timing = folder / 'sunrise.jsonl', folder / REPLAY_TIMING
    if not capture.exists():
        return None
    inputs = lambda path: wire_stream.shared_input(  # noqa: E731
        [e for e in net_capture.load(path) if e.get('conn') == 1 and 'name' in e])
    if folder == out and not timing.exists() and inputs(out / 'vanilla.jsonl') != inputs(capture):
        return None
    return capture, timing_record(timing, scenarios) if timing.exists() else None


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--bin', default=str(lab.cargo_target() / 'fast'), help='sunrise-client and sunrise-ctl')
    parser.add_argument('--out', default=str(lab.LOCAL / 'wire-stream-parity'))
    parser.add_argument('--scenarios', default=','.join(DEFAULT),
                        type=lambda s: list(SCENARIOS) if s == 'all' else s.split(','),
                        help='scenario names, comma-separated, or `all` (default: the DEFAULT set)')
    parser.add_argument('--live', action='store_true',
                        help='compare Sunrise as it played against the live server, not the replay of vanilla\'s '
                             'server stream')
    parser.add_argument('--clients', default='vanilla,sunrise', type=lambda s: s.split(','),
                        help='the reference client and the one compared with it, of vanilla, sunrise and '
                             'sunrise-legacy; one alone is only captured (a loaded machine whose vanilla client fails '
                             'to log in: rerun sunrise alone, copy an earlier vanilla.jsonl into --out, then '
                             '--compare-only)')
    parser.add_argument('--compare-only', action='store_true', help='compare the captures already in --out')
    parser.add_argument('--keep', action='store_true', help='keep the scratch folders')
    args = parser.parse_args()
    unknown = [s for s in args.scenarios if s not in SCENARIOS]
    if unknown:
        parser.error(f'unknown scenarios {unknown}; known: {", ".join(SCENARIOS)}')
    if len(args.clients) not in (1, 2) or len(set(args.clients)) != len(args.clients) \
            or not set(args.clients) <= set(CLIENTS):
        parser.error(f'--clients takes one or two of {", ".join(CLIENTS)}')
    if args.compare_only and len(args.clients) != 2:
        parser.error('--compare-only compares two clients: name both in --clients')
    out = Path(args.out)
    # Vanilla against Sunrise: vanilla plays the live server, then its server stream is replayed to Sunrise
    # (tools/wire_replay.py), so both clients got the same answers on the same ticks and the server's timing and
    # randomness leave the comparison. --live plays Sunrise against the live server instead, as do a Grim server
    # and a transfer, which judge or move Sunrise itself, and a required resource pack: the replay serves no pack,
    # so the clients' answers to it are compared live.
    replayed = (args.clients == ['vanilla', 'sunrise'] and not args.live and not args.compare_only
                and not os.environ.get('WIRE_STREAM_GRIM') and not os.environ.get('WIRE_STREAM_TRANSFER')
                and not os.environ.get('WIRE_STREAM_RESOURCE_PACK'))
    replay_capture, uncertain = False, None
    if args.compare_only:
        captures = {who: out / f'{who}.jsonl' for who in args.clients}
        if args.clients == ['vanilla', 'sunrise'] and not args.live:
            try:
                evidence = replay_evidence(out, args.scenarios)
            except Failed as e:
                sys.exit(f'FAIL: {e}')
            if evidence is not None:
                (captures['sunrise'], uncertain), replay_capture = evidence, True
    else:
        if out.exists() and (not out.is_dir() or any(out.iterdir())):
            sys.exit(f'FAIL: {out} is not an empty folder: give --out a new one, or remove it first (a run writes '
                     'only into an empty folder, never over earlier captures)')
        out.mkdir(parents=True, exist_ok=True)
        try:
            if replayed:
                import wire_replay
                run(argparse.Namespace(**{**vars(args), 'clients': ['vanilla']}), out)
                phase = float(os.environ['WIRE_STREAM_PHASE']) if os.environ.get('WIRE_STREAM_PHASE') else None
                results = wire_replay.replay(out / 'vanilla.jsonl', args.scenarios, args.bin, out / 'replay', phase)
                captures = {'vanilla': out / 'vanilla.jsonl', 'sunrise': out / 'replay' / 'sunrise.jsonl'}
            else:
                captures = run(args, out)
        except Failed as e:
            sys.exit(f'FAIL: {e}')
        if len(captures) < 2:
            print(f'captured {", ".join(captures)} in {out}; nothing to compare it with')
            return
    try:
        if not replayed:
            results = compare(captures, args.scenarios, replay_capture, uncertain)
    except Failed as e:
        sys.exit(f'FAIL: {e}')
    if os.environ.get('WIRE_STREAM_TRANSFER'):
        (ref, ref_name), (new, new_name) = [(who, CLIENTS[who]) for who in captures]
        a, b = transfer_stream(captures[ref]), transfer_stream(captures[new])
        problems = [f'  #{i}: {ref_name} {x}, {new_name} {y}'
                    for i, (x, y) in enumerate(itertools.zip_longest(a, b, fillvalue='-')) if x != y]
        results.append(('transfer_back', problems[:40]))
    if (out / 'grim.db').exists():
        for who, violations in grim_violations(out / 'grim.db', args.clients).items():
            counts = {}
            for check, _ in violations:
                counts[check] = counts.get(check, 0) + 1
            print(f'Grim violations, {CLIENTS[who]}: {len(violations)} {counts}')
    how = 'replay' if replayed or replay_capture else 'live'
    detail = ('replayed vanilla server stream, compared byte for byte; delivery ticks inferred, not observed'
              if how == 'replay' else 'live server: aligned to its answers')
    print(f'\nserverbound play streams, {CLIENTS[args.clients[0]]} vs {CLIENTS[args.clients[1]]} ({detail}; a '
          f'match is a diagnostic, not an exact pass); captures in {out}')
    for name, problems in results:
        print(f'  {verdict(problems, "live" if name == "transfer_back" else how)}  {name}')
        for p in problems:
            print(p)
    if any(p for _, p in results):
        sys.exit(1)


if __name__ == '__main__':
    main()
