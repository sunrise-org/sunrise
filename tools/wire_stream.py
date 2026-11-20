"""The serverbound play stream of a capture (tools/net_capture.py), cut into client ticks and decoded field by
field, so two clients' streams can be compared tick by tick (tools/test_wire_stream_parity.py).

A client tick's packets end with ServerboundClientTickEndPacket (Minecraft.tick sends it last). A scenario
starts at the command /wire-mark, which both drivers send at the start of the scenario's first tick.

Some packets are not the tick's: the server's own timing decides them (keep-alive answers, pongs, chunk batch
acknowledgements, teleport acceptances and the move that follows one, chat acknowledgements). They are kept
apart (`frame`), never compared tick by tick, and `misplaced` checks vanilla's rule for them instead: the
client processes the server's packets at the start of a frame (Minecraft.runTick: processQueuedPackets before
the ticks), so none of them lands between two packets of one tick. Keep-alive answers and chunk batch
acknowledgements are the exception: the network thread sends them (ClientCommonPacketListenerImpl.handleKeepAlive,
ClientPacketListener.handleChunkBatchFinished), so they may land anywhere.

`exact` (a replay, tools/wire_replay.py: both clients got the same server packets): nothing is renumbered. Each
tick's `wire` is one ordered stream: every serverbound packet of the tick in the order it was sent, the frame's
answers and the network thread's among them, compared byte for byte (floats by their bits). Only the chunk batch
acknowledgement's body is left out of it: its rate is the client's own measurement (`MEASURED`). Entity ids are
named through the shared server stream (`self`, then e1, e2... in the order the server added them), the same on
both sides, for reading only. The network thread's answers are also kept in `frame`, their bytes in `frame_wire`.
A tick that differs only in where the network thread's answers sit among the others (`interleaving_only`) is not
the client's tick to decide.
"""
import struct

# Sent from the frame (the server's packets), not from a tick.
FRAME = {'keep_alive', 'pong', 'chunk_batch_received', 'accept_teleportation', 'chat_ack'}
# Answers to the server that also come from the frame; `misplaced` checks them. The network thread's own
# answers are left out: they may come between two packets of a tick.
ASYNC = FRAME - {'keep_alive', 'chunk_batch_received'}
# The network thread's own answers (ClientCommonPacketListenerImpl.handleKeepAlive,
# ClientPacketListener.handleChunkBatchFinished).
NETWORK = FRAME - ASYNC
# The network thread's answer whose body is the client's own measurement: ChunkBatchReceived's rate comes from how
# long the client took over the batch's chunks (ChunkBatchSizeCalculator), so a replay can't make it the same.
MEASURED = {'chunk_batch_received'}


class Reader:
    def __init__(self, data):
        self.data, self.pos = data, 0

    def take(self, n):
        out = self.data[self.pos:self.pos + n]
        if len(out) < n:
            raise ValueError('short packet')
        self.pos += n
        return out

    def varint(self):
        value = shift = 0
        while True:
            b = self.take(1)[0]
            value |= (b & 0x7F) << shift
            if not b & 0x80:
                return value - (1 << 32) if value >= 1 << 31 else value
            shift += 7

    def double(self):
        return struct.unpack('>d', self.take(8))[0]

    def float(self):
        return struct.unpack('>f', self.take(4))[0]

    def ubyte(self):
        return self.take(1)[0]

    def short(self):
        return struct.unpack('>h', self.take(2))[0]

    def bool(self):
        return self.take(1)[0] != 0

    def pos_long(self):
        # BlockPos.asLong: x in the top 26 bits, z in the next 26, y in the low 12, each signed.
        v = struct.unpack('>q', self.take(8))[0]
        signed = lambda value, bits: value - (1 << bits) if value >= 1 << (bits - 1) else value  # noqa: E731
        return [v >> 38, signed(v & 0xFFF, 12), signed((v >> 12) & 0x3FFFFFF, 26)]

    def string(self):
        return self.take(self.varint()).decode('utf-8')

    def rest(self):
        out = self.data[self.pos:]
        self.pos = len(self.data)
        return out.hex()


def flags(v):
    return {'on_ground': bool(v & 1), 'horizontal_collision': bool(v & 2)}


def decode(name, data):
    """The fields of one serverbound play packet (vanilla's codecs); `raw` holds what isn't decoded."""
    r = Reader(data)
    if name == 'move_player_pos':
        return {'x': r.double(), 'y': r.double(), 'z': r.double(), **flags(r.ubyte())}
    if name == 'move_player_pos_rot':
        return {'x': r.double(), 'y': r.double(), 'z': r.double(), 'yaw': r.float(), 'pitch': r.float(),
                **flags(r.ubyte())}
    if name == 'move_player_rot':
        return {'yaw': r.float(), 'pitch': r.float(), **flags(r.ubyte())}
    if name == 'move_player_status_only':
        return flags(r.ubyte())
    if name == 'player_input':
        v = r.ubyte()
        keys = ['forward', 'backward', 'left', 'right', 'jump', 'shift', 'sprint']
        return {'keys': [k for i, k in enumerate(keys) if v >> i & 1]}
    if name == 'player_command':
        actions = ['stop_sleeping', 'start_sprinting', 'stop_sprinting', 'start_riding_jump', 'stop_riding_jump',
                   'open_inventory', 'start_fall_flying']
        entity, action, data_ = r.varint(), r.varint(), r.varint()
        return {'entity': entity, 'action': actions[action] if 0 <= action < len(actions) else action,
                'data': data_}
    if name == 'set_carried_item':
        return {'slot': r.short()}
    if name == 'attack':
        return {'entity': r.varint()}
    if name == 'interact':
        # The point (Vec3.LP_STREAM_CODEC, packed) and the sneak flag stay raw.
        return {'entity': r.varint(), 'hand': r.varint(), 'raw': r.rest()}
    if name == 'use_item':
        return {'hand': r.varint(), 'sequence': r.varint(), 'yaw': r.float(), 'pitch': r.float()}
    if name == 'use_item_on':
        return {'hand': r.varint(), 'pos': r.pos_long(), 'face': r.varint(),
                'cursor': [r.float(), r.float(), r.float()], 'inside': r.bool(), 'world_border': r.bool(),
                'sequence': r.varint()}
    if name == 'player_action':
        return {'action': r.varint(), 'pos': r.pos_long(), 'direction': r.ubyte(), 'sequence': r.varint()}
    if name == 'container_click':
        return {'container': r.varint(), 'state': r.varint(), 'slot': r.short(), 'button': r.ubyte(),
                'input': r.varint(), 'raw': r.rest()}
    if name == 'container_close':
        return {'container': r.varint()}
    if name == 'chat_command':
        return {'command': r.string()}
    if name in ('chat', 'chat_command_signed'):
        # The timestamp (Instant.now) and the salt (a random long) differ on every send, on any client: left out.
        # The signatures, the last-seen acknowledgements and their checksum stay raw.
        text = r.string()
        r.take(16)
        return {'message' if name == 'chat' else 'command': text, 'raw': r.rest()}
    return {'raw': r.rest()} if data else {}


def is_mark(name, data):
    """Whether a serverbound play packet is the /wire-mark command itself (not its bytes inside another packet)."""
    if name != 'chat_command':
        return False
    try:
        return Reader(data).string() == 'wire-mark'
    except (ValueError, UnicodeDecodeError):
        return False


def wire_key(name, data):
    """What of a serverbound packet two clients that got the same server packets must send alike: its bytes, but
    for a chat message's or signed command's timestamp (Instant.now) and salt (a random long)."""
    if name in ('chat', 'chat_command_signed'):
        r = Reader(data)
        r.string()
        return data[:r.pos].hex() + '|' + data[r.pos + 16:].hex()
    return data.hex()


def server_entities(entries):
    """{entity id: name} by the server's AddEntity order (e1, e2...), the player's own id as `self`."""
    names = {self_id(entries): 'self'}
    for e in entries:
        if e.get('dir') == 's2c' and e.get('state') == 'play' and e.get('name') == 'add_entity':
            entity = Reader(bytes.fromhex(e['data'])).varint()
            names.setdefault(entity, f'e{len(names)}')
    return names


def shared_input(entries):
    """The server's play packets in order: two replayed captures must have got the same."""
    return [(e['name'], e.get('data')) for e in entries if e.get('dir') == 's2c' and e.get('state') == 'play'
            and 'name' in e]


def self_id(entries):
    """The player's entity id: the first field (an Int) of the clientbound Login."""
    for e in entries:
        if e.get('dir') == 's2c' and e.get('state') == 'play' and e.get('name') == 'login':
            return struct.unpack('>i', bytes.fromhex(e['data'])[:4])[0]
    return None


def is_mark(command):
    """Whether a chat command is a scenario marker: /wire-mark, or /wire-mark NN naming the scenario's number."""
    if command is None:
        return False
    word, space, number = command.partition(' ')
    return word == 'wire-mark' and (not space or number.isdigit())


def mark_packet(name, data):
    """Whether a serverbound play packet is a /wire-mark command itself (not its bytes inside another packet)."""
    if name != 'chat_command':
        return False
    try:
        return is_mark(Reader(data).string())
    except (ValueError, UnicodeDecodeError):
        return False


def scenarios(entries, exact=False):
    """One list of ticks for each /wire-mark in the play stream, in order (the ticks after a mark run to the
    next mark); a tick is {'packets': [(name, fields)], 'wire': [(name, bytes)], 'frame': [(name, fields)],
    'frame_wire': [(name, bytes)], 'misplaced': [name], 'raw': [(where, name, payload hex)]}: 'raw' holds every
    serverbound packet as it was on the wire, in order, before any of the rewrites below.

    Live (`exact` false):

    A click's state id becomes `stale`: how far it is behind the last one the server sent for that container
    (0 = up to date). The ids themselves count every change the server made since the login, so two runs
    whose earlier scenarios differed number the same click differently. Prediction sequences are numbered from
    the client's last one before the scenario for the same reason (number_sequences), and other entities are
    named by their first appearance in the client's own packets (name_entities).

    `exact`: none of that (see the module's doc)."""
    me = self_id(entries)
    shared = server_entities(entries) if exact else None
    play = [e for e in entries if e.get('state') == 'play' and 'name' in e and e.get('dir') in ('c2s', 's2c')]
    runs, current, server_state = [], None, {}
    for e in play:
        name = e['name']
        if e['dir'] == 's2c':
            if not exact and name in ('container_set_content', 'container_set_slot'):
                r = Reader(bytes.fromhex(e.get('data', '')))
                container = r.varint()
                server_state[container] = r.varint()
            continue
        data = bytes.fromhex(e.get('data', ''))
        if mark_packet(name, data):
            current = [new_tick()]
            runs.append(current)
            continue
        if current is None:
            continue
        fields = decode(name, data)
        if not exact and name == 'container_click':
            last = server_state.get(fields['container'], fields['state'])
            fields = {'container': fields['container'], 'stale': last - fields.pop('state'), **fields}
        tick = current[-1]
        tick['raw'].append(('frame' if name in FRAME else 'packets', name, e.get('data', '')))
        if exact and isinstance(fields.get('entity'), int):
            fields['entity'] = shared.get(fields['entity'], fields['entity'])
        elif 'entity' in fields and fields['entity'] == me:
            fields['entity'] = 'self'
        if name in FRAME and not exact:
            tick['frame'].append((name, fields))
            tick['frame_wire'].append((name, wire_key(name, data)))
            if name in ASYNC and tick['packets']:
                tick['misplaced'].append(name)
            continue
        if exact and name in NETWORK:
            # In the stream in its place, and kept apart for its measured body.
            tick['frame'].append((name, fields))
            tick['frame_wire'].append((name, wire_key(name, data)))
        tick['packets'].append((name, fields))
        tick['wire'].append((name, None if exact and name in MEASURED else wire_key(name, data)))
        if name == 'client_tick_end':
            current.append(new_tick())
    if not exact:
        before = 0
        for run in runs:
            name_entities(run)
            before = number_sequences(run, before)
    return runs


def number_sequences(run, before):
    """Block prediction sequences (player_action, use_item, use_item_on) count every sequenced action since the
    login: count them from the client's last one before the scenario (`before`, 0 at the login) instead, so one
    extra sequenced packet in an earlier scenario doesn't make every later one differ. The scenario's first
    sequence stays what it is past that one (a wrong start, or a reset that didn't happen, #365, still shows).
    The client's last sequence, for the next scenario."""
    last = before
    for tick in run:
        for _, fields in tick['packets']:
            sequence = fields.get('sequence')
            if isinstance(sequence, int) and sequence != 0:
                last = sequence
                fields['sequence'] = sequence - before
    return last


def first_frame(run, name):
    """The first tick whose frame sent `name` (the server's answer it carries), or None."""
    return next((i for i, tick in enumerate(run) if any(n == name for n, _ in tick['frame'])), None)


def new_tick():
    return {'packets': [], 'wire': [], 'frame': [], 'frame_wire': [], 'misplaced': [], 'raw': []}


def name_entities(run):
    """Other entities' ids differ from one run to the next: name them by first appearance (e1, e2...)."""
    names = {}
    for tick in run:
        for _, fields in tick['packets']:
            entity = fields.get('entity')
            if isinstance(entity, int):
                fields['entity'] = names.setdefault(entity, f'e{len(names) + 1}')


def compare(vanilla, sunrise, ticks, key='packets'):
    """The ticks 0..ticks where the two streams differ: [(tick, vanilla packets, Sunrise packets)]. Fields
    compare exactly, floats included: they are what vanilla's client sends. `key` 'wire': by the packets' bytes
    (an exact scenarios()); the decoded packets are what is returned either way. A tick either stream doesn't
    reach is a difference, even when both miss it: a short capture never matches."""
    out = []
    at = lambda run, i, k: run[i][k] if i < len(run) else None  # noqa: E731
    for i in range(ticks + 1):
        if i >= len(vanilla) or i >= len(sunrise) or at(vanilla, i, key) != at(sunrise, i, key):
            out.append((i, at(vanilla, i, 'packets'), at(sunrise, i, 'packets')))
    return out


def complete(run, ticks):
    """Whether `run` holds the whole measured window: ticks 0..ticks, each closed by its ClientTickEnd."""
    return len(run) > ticks + 1


def timing_doubts(record, scenarios):
    """A replay's delivery-timing record (tools/wire_replay.py writes it), checked whole: made for exactly these
    scenarios, in their order, each doubt a reason under one of them. Its {scenario: [why]}; ValueError otherwise."""
    if not isinstance(record, dict) or record.get('scenarios') != scenarios:
        found = record.get('scenarios') if isinstance(record, dict) else record
        raise ValueError(f'the delivery-timing record is for the scenarios {found}, not {scenarios}')
    doubts = record.get('uncertain')
    if not isinstance(doubts, dict) or not set(doubts) <= set(scenarios) or not all(
            isinstance(why, list) and all(isinstance(w, str) for w in why) for why in doubts.values()):
        raise ValueError('the delivery-timing record has no whole {scenario: [reason]} list of its doubts')
    return doubts


def interleaving_only(a, b):
    """Whether two ticks' exact `wire` streams differ only in where the network thread's answers sit among the
    tick's other packets: the same packets in the same order without them, and the same answers in the same order.
    That thread sends them whenever the server's packet arrives, so their place is no client's tick to decide."""
    if a is None or b is None:
        return False
    split = lambda wire: ([p for p in wire if p[0] not in NETWORK], [p for p in wire if p[0] in NETWORK])  # noqa: E731
    return split(a) == split(b)


def compare_answered(vanilla, sunrise, ticks, av, ah):
    """compare, for a scenario the server answers once (a teleport): the tick each client received the answer
    in (`av`, `ah`) compares with the other client's, every other tick with the same tick, on the script's
    clock. A loaded server answers a few ticks later in one run than in the other, while the script's own
    packets (a button released, the position resent every 20 ticks) keep its clock. Numbers are vanilla's.

    The moves of the two answer ticks (LocalPlayer.sendPosition, at the end of the player's tick) keep the
    script's clock too: an answer that doesn't move the player (`tp @s ~ ~ ~`) leaves them to the 20-tick
    position reminder, which a later answer can land on in one run and not in the other."""
    out = [d for d in compare(vanilla, sunrise, ticks) if d[0] not in (av, ah)]
    packets = lambda run, i: run[i]['packets'] if i < len(run) else None  # noqa: E731
    v, h = packets(vanilla, av), packets(sunrise, ah)
    if strip_moves(v) != strip_moves(h):
        out.append((av, v, h))
    for i in sorted({av, ah}):
        vi, hi = packets(vanilla, i), packets(sunrise, i)
        if moves(vi) != moves(hi) and all(d[0] != i for d in out):
            out.append((i, vi, hi))
    return sorted(out, key=lambda d: d[0])


MOVES = {'move_player_pos', 'move_player_pos_rot', 'move_player_rot', 'move_player_status_only'}


def moves(packets):
    return None if packets is None else [p for p in packets if p[0] in MOVES]


def strip_moves(packets):
    return None if packets is None else [p for p in packets if p[0] not in MOVES]


def describe(packets):
    if packets is None:
        return '    (no such tick)'
    if not packets:
        return '    (nothing)'
    lines = []
    for name, fields in packets:
        text = ', '.join(f'{k}={round(v, 6) if isinstance(v, float) else v}' for k, v in fields.items())
        lines.append(f'    {name}' + (f' {{{text}}}' if text else ''))
    return '\n'.join(lines)


def field_diff(v, h, names=('vanilla', 'Sunrise'), wire=None):
    """One line per differing field of packets paired by position, with the exact values. `wire`: the two ticks'
    (name, bytes) lists; a packet whose bytes differ where no decoded field does (a float's sign of zero, a NaN's
    bits) gets its bytes."""
    a, b = names
    lines = []
    for i, ((vn, vf), (hn, hf)) in enumerate(zip(v or [], h or [])):
        if vn != hn:
            lines.append(f'    #{i}: {a} {vn}, {b} {hn}')
            continue
        fields = [k for k in sorted(set(vf) | set(hf)) if vf.get(k) != hf.get(k)]
        for k in fields:
            lines.append(f'    #{i} {vn}.{k}: {a} {vf.get(k)!r}, {b} {hf.get(k)!r}')
        if not fields and wire and wire[0] and wire[1] and wire[0][i] != wire[1][i]:
            lines.append(f'    #{i} {vn} bytes: {a} {wire[0][i][1]}, {b} {wire[1][i][1]}')
    return lines
