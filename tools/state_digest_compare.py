#!/usr/bin/env python3
"""Compares two state digests step by step and names the first differing step per group: A/digest.txt B/digest.txt; a --coverage run's coverage.txt beside them is compared too."""
import argparse
import sys
from pathlib import Path

# `tick` and `dim` are compared too: a changed game time or level routing is a behaviour change.
GROUPS = (
    'tick', 'dim', 'walk_error', 'entities', 'randoms', 'hangings', 'cushions', 'other_entities', 'server_thrown_eyes',
    'fangs', 'lightning', 'end_crystals', 'dragon', 'breath_clouds', 'ominous_spawners', 'leash_knots',
    'fishing_hooks', 'server_thrown_potions', 'seats', 'entity_identity', 'entity_shared', 'players',
    'retained_entities', 'entity_memory', 'entity_portals', 'server_entity_state',
    'server_portal_travel', 'server_player_transit', 'block_entities', 'level_random', 'counters', 'scheduled',
)


IGNORABLE_GROUPS = tuple(group for group in GROUPS if group not in ('tick', 'dim', 'walk_error'))


def parse(lines):
    """The running ticks' fields, and how many frozen ticks moved the level random."""
    running, frozen_moves, last_random = [], 0, None
    current = None
    for line in lines:
        if line.startswith('  p\t') and current is not None:
            values = dict(part.split('=', 1) for part in line.rstrip('\r\n').split('\t')[1:] if '=' in part)
            if 'id' in values and 'name' in values:
                entity_id = values.pop('id')
                name = values.pop('name')
                current.setdefault('player_detail', {})[entity_id] = (name, values)
        if not line.startswith('tick='):
            continue
        current = None
        payload, separator, walk_error = line.rstrip().partition(' walk_error=')
        fields = dict(part.split('=', 1) for part in payload.split())
        if separator:
            fields['walk_error'] = walk_error
        if fields['run'] == '1':
            running.append(fields)
            current = fields
        elif last_random is not None and fields['level_random'] != last_random:
            frozen_moves += 1
        last_random = fields['level_random']
    return running, frozen_moves


def player_differences(a, b):
    left, right = a.get('player_detail'), b.get('player_detail')
    if left is None or right is None:
        return
    for entity_id in dict.fromkeys((*left, *right)):
        if entity_id not in left or entity_id not in right:
            present = left if entity_id in left else right
            side = 'left' if present is left else 'right'
            yield f'  player id={entity_id} name={present[entity_id][0]} only in {side} run'
            continue
        name, a_values = left[entity_id]
        other_name, b_values = right[entity_id]
        identity = f'id={entity_id} name={name}' if name == other_name else f'id={entity_id} name={name} vs {other_name}'
        for label in dict.fromkeys((*a_values, *b_values)):
            before = a_values.get(label, '<absent>')
            after = b_values.get(label, '<absent>')
            if before != after:
                yield f'  player {identity} {label}: {before} vs {after}'


def compare(a_lines, b_lines, ignore_groups=()):
    """Report first divergences by group and return whether all required groups match."""
    (a, fa), (b, fb) = parse(a_lines), parse(b_lines)
    if not a or not b:
        return [f'running ticks: {len(a)} vs {len(b)}', 'DIFFERENT'], False
    steps = range(min(len(a), len(b)))
    report = [f'running ticks: {len(a)} vs {len(b)}, game time {a[0]["tick"]}..{a[-1]["tick"]} vs '
              f'{b[0]["tick"]}..{b[-1]["tick"]}',
              f'frozen ticks that moved the level random: {fa} vs {fb}']
    same = len(a) == len(b)
    first_failure = first_ignored = None
    invalid_walks = {t for t in steps if 'walk_error' in a[t] or 'walk_error' in b[t]}
    walk_groups = {'entities', 'randoms', 'entity_identity', 'entity_shared'}
    ignored_differences = False
    for group in GROUPS:
        muted = group in IGNORABLE_GROUPS and group in ignore_groups
        ignored_steps = []
        if group == 'walk_error':
            failures = sorted(invalid_walks)
        else:
            failures = []
            for t in steps:
                if t in invalid_walks and group in walk_groups:
                    continue
                left, right = a[t].get(group), b[t].get(group)
                if left is None or right is None:
                    failures.append(t)
                elif left != right:
                    (ignored_steps if muted else failures).append(t)
        diff = failures or ignored_steps
        if diff:
            t = diff[0]
            ignored = not failures
            ignored_differences |= ignored
            if ignored:
                if first_ignored is None or t < first_ignored[0]:
                    first_ignored = (t, group, True)
            else:
                same = False
                if first_failure is None or t < first_failure[0]:
                    first_failure = (t, group, False)
            label = 'DIFFERS (ignored)' if ignored else 'DIFFERS'
            report.append(f'{group:21} {label} on {len(diff)} steps, first step {t + 1} '
                          f'(tick {a[t]["tick"]} vs {b[t]["tick"]}): '
                          f'{a[t].get(group, "<missing>")} vs {b[t].get(group, "<missing>")}')
            if group == 'players':
                report.extend(player_differences(a[t], b[t]))
        else:
            label = 'identical'
            if invalid_walks and group in walk_groups:
                label = 'skipped on steps with a walk error'
            report.append(f'{group:21} {label}')
    first = first_failure or (first_ignored if same else None)
    if first is not None:
        t, group, ignored = first
        label = f'{group} (ignored)' if ignored else group
        report.insert(2, f'first differing group: {label} at step {t + 1} '
                         f'(tick {a[t]["tick"]} vs {b[t]["tick"]})')
    verdict = 'MATCH (ignored differences)' if ignored_differences else 'IDENTICAL'
    report.append(verdict if same else 'DIFFERENT')
    return report, same


def compare_coverage(a_dir, b_dir):
    """The coverage.txt lines of two --coverage runs (spots reached, saved spawner data), or None when neither has one."""
    a, b = (Path(d) / 'coverage.txt' for d in (a_dir, b_dir))
    if not a.exists() and not b.exists():
        return None
    if a.exists() != b.exists():
        return ['coverage       only in one run'], False
    same = a.read_text() == b.read_text()
    return [f'coverage       {"identical" if same else "DIFFERS"}: ' + ' | '.join(a.read_text().splitlines())], same


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('left', type=Path)
    parser.add_argument('right', type=Path)
    parser.add_argument('--ignore-group', action='append', choices=IGNORABLE_GROUPS, default=[])
    args = parser.parse_args()
    with args.left.open() as a, args.right.open() as b:
        report, same = compare(a, b, args.ignore_group)
    coverage = compare_coverage(args.left.parent, args.right.parent)
    if coverage:
        report = report[:-1] + coverage[0] + [report[-1] if same and coverage[1] else 'DIFFERENT']
        same = same and coverage[1]
    print('\n'.join(report))
    return 0 if same else 1


if __name__ == '__main__':
    sys.exit(main())
