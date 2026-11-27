#!/usr/bin/env python3
"""Compare frog shore navigation and open-water buoyancy with vanilla 26.3."""
import argparse

try:
    from . import lab, test_mob_ai_parity as mob
except ImportError:
    import lab
    import test_mob_ai_parity as mob


def scenarios():
    floor = 'fill -10 55 -10 10 55 10 minecraft:stone'
    return [
        ('frog_shore_path', 120, '', [
            floor,
            'fill -10 56 -10 6 63 10 minecraft:water',
            'fill 7 56 -10 10 63 10 minecraft:stone',
            'mob minecraft:frog 0.5 62 0.5 -90 179 persistent',
        ]),
        ('frog_swim_open', 30, '', [
            floor,
            'fill -10 56 -10 10 72 10 minecraft:water',
            'mob minecraft:frog 0.5 64 0.5 -90 179 persistent',
        ]),
    ]


def first_route(ticks):
    """Destination and speed of the first route in the frog's move controller."""
    for fields in ticks:
        if fields['nav'] != 'none' and fields['move'].startswith('MOVE_TO '):
            return fields['nav'].split()[0], fields['move'].split()[-1]
    raise AssertionError('Frog never followed a path toward the bank')


def frog_ticks(trace, name):
    return [mobs[0] for _, mobs in sorted(trace[name]['ticks'].items())]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--rust-only', action='store_true', help='reuse the last vanilla output')
    args = parser.parse_args()
    build = lab.LOCAL / 'frog-swim-parity'
    mob.record_scenarios(build, list(enumerate(scenarios())), rust_only=args.rust_only)
    vanilla = mob.parse(build / 'vanilla.txt')
    sunrise = mob.parse(build / 'sunrise.txt')

    def shore_route(trace):
        ticks = frog_ticks(trace, 'frog_shore_path')
        positions = [tuple(map(float, mob.decode(tick['p']).split(','))) for tick in ticks]
        start_x = positions[0][0]
        assert any(x > start_x and y < 63 for x, y, _ in positions), 'Frog did not swim east below the surface'
        # The bank is the only ground at y=64 in this pool.
        assert ticks[-1]['g'][0] == '1' and positions[-1][1] == 64, 'Frog did not stand on the bank'
        return first_route(ticks)

    expected = shore_route(vanilla)
    actual = shore_route(sunrise)
    assert actual == expected, (actual, expected)
    print(f'Frog swims toward the same shore block {actual[0]} at speed {mob.decode(actual[1])} and climbs out')

    # With no standable shore or viable swim target, only the move control's
    # buoyancy should affect motion. Compare every sampled position and velocity.
    def motion(trace):
        return [(tick['p'], tick['v']) for tick in frog_ticks(trace, 'frog_swim_open')]

    assert motion(vanilla) == motion(sunrise), 'Frog open-water motion differs'
    print('Frog open-water positions and velocities match')


if __name__ == '__main__':
    main()
