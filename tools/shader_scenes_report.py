"""tools/shader_scenes.py's comparison: reads both sides' pictures, box-filters them to a grid, compares them cell
by cell and writes the report (report.json, report.md, index.html, heat maps and side-by-side pairs). Standard
library only: the PNG reader takes the 8-bit RGB and RGBA pictures both games write."""
import concurrent.futures
import hashlib
import html
import json
import math
import os
import struct
import sys
import zlib
from pathlib import Path

# The comparison grid: 16:9 at 1/20 of 1280x720, coarse enough that animated water, clouds and leaves count little.
GRID = (64, 36)
# Frame flags, on the grid's Rec. 709 luma (0..255): nearly all black, nearly all white, or one flat colour.
BLACK_LUMA, WASHED_LUMA, FLAT_SD = 8.0, 247.0, 2.0
SIDES = ("sunrise", "java")
RUN_MARK = ".shader-scenes-run"


# PNG -----------------------------------------------------------------------------------------------------------

class PictureError(ValueError):
    """A picture this tool cannot compare; the message says why."""


def read_png(path, size):
    """The RGB rows of an 8-bit RGB or RGBA, non-interlaced PNG of exactly `size` (the scene's): (w, h, bytes).
    Every way a file can be damaged is a PictureError naming it: a chunk running past the file, a bad CRC, a short
    IHDR, a missing IEND, a zlib stream that is invalid, cut short or longer than the rows. Any other size is
    refused before its pixels are decompressed, which also bounds the memory a picture takes."""
    data = Path(path).read_bytes()
    if data[:8] != b"\x89PNG\r\n\x1a\n":
        raise PictureError(f"{path}: not a PNG")
    pos, header, chunks, ended = 8, None, [], False
    while pos < len(data):
        if pos + 12 > len(data):
            raise PictureError(f"{path}: a chunk header runs past the end of the file")
        length, kind = struct.unpack(">I4s", data[pos:pos + 8])
        end = pos + 8 + length
        if end + 4 > len(data):
            raise PictureError(f"{path}: chunk {kind!r} runs past the end of the file")
        body = data[pos + 8:end]
        if struct.unpack(">I", data[end:end + 4])[0] != zlib.crc32(kind + body):
            raise PictureError(f"{path}: chunk {kind!r} fails its CRC")
        pos = end + 4
        if kind == b"IHDR":
            if len(body) != 13:
                raise PictureError(f"{path}: IHDR is {len(body)} bytes, not 13")
            header = struct.unpack(">IIBBBBB", body)
            if header[4] != 0 or header[5] != 0:
                raise PictureError(f"{path}: IHDR compression method {header[4]} or filter method {header[5]} "
                                   "is not 0, the only one PNG defines")
        elif kind == b"IDAT":
            chunks.append(body)
        elif kind == b"IEND":
            if body:
                raise PictureError(f"{path}: IEND carries {len(body)} bytes; it must be empty")
            ended = True
            break
    if header is None:
        raise PictureError(f"{path}: no IHDR")
    if not ended:
        raise PictureError(f"{path}: no IEND: the file is cut short")
    width, height, depth, colour, _, _, interlace = header
    if [width, height] != list(size):
        raise PictureError(f"{path}: {width}x{height}, the scene asks for {size[0]}x{size[1]}")
    if depth != 8 or colour not in (2, 6) or interlace:
        raise PictureError(f"{path}: bit depth {depth}, colour type {colour}, interlace {interlace}: "
                           "only 8-bit RGB or RGBA without interlace")
    channels = 3 if colour == 2 else 4
    stride = width * channels
    expected = (stride + 1) * height
    stream = zlib.decompressobj()
    try:
        raw = stream.decompress(b"".join(chunks), expected + 1)
    except zlib.error as error:
        raise PictureError(f"{path}: invalid image data ({error})") from None
    if not stream.eof:
        raise PictureError(f"{path}: the image data is cut short or longer than its rows")
    if len(raw) != expected:
        raise PictureError(f"{path}: {len(raw)} bytes of rows, {expected} expected")
    rows = bytearray()
    prior = bytearray(stride)
    for y in range(height):
        start = y * (stride + 1)
        kind = raw[start]
        line = bytearray(raw[start + 1:start + 1 + stride])
        _unfilter(kind, line, prior, channels, path)
        rows += line if channels == 3 else _drop_alpha(line)
        prior = line
    return width, height, bytes(rows)


def _drop_alpha(line):
    out = bytearray(len(line) // 4 * 3)
    out[0::3], out[1::3], out[2::3] = line[0::4], line[1::4], line[2::4]
    return out


def _unfilter(kind, line, prior, bpp, path):
    n = len(line)
    if kind == 0:
        return
    if kind == 1:
        for i in range(bpp, n):
            line[i] = (line[i] + line[i - bpp]) & 255
    elif kind == 2:
        for i in range(n):
            line[i] = (line[i] + prior[i]) & 255
    elif kind == 3:
        for i in range(n):
            left = line[i - bpp] if i >= bpp else 0
            line[i] = (line[i] + ((left + prior[i]) >> 1)) & 255
    elif kind == 4:
        for i in range(n):
            a = line[i - bpp] if i >= bpp else 0
            b = prior[i]
            c = prior[i - bpp] if i >= bpp else 0
            p = a + b - c
            pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
            line[i] = (line[i] + (a if pa <= pb and pa <= pc else b if pb <= pc else c)) & 255
    else:
        raise PictureError(f"{path}: unknown filter type {kind}")


def write_png(path, width, height, rgb):
    """An 8-bit RGB PNG of `rgb` (rows top to bottom, no filter)."""
    def chunk(kind, body):
        return struct.pack(">I", len(body)) + kind + body + struct.pack(">I", zlib.crc32(kind + body))
    stride = width * 3
    raw = b"".join(b"\x00" + rgb[y * stride:(y + 1) * stride] for y in range(height))
    Path(path).parent.mkdir(parents=True, exist_ok=True)
    Path(path).write_bytes(b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0))
                           + chunk(b"IDAT", zlib.compress(raw, 6)) + chunk(b"IEND", b""))


def downscale(width, height, rgb, grid):
    """Box-filters a picture to grid (cols, rows): a list of (r, g, b) means, row by row. Cell c covers the pixels
    from c * width // cols up to (c + 1) * width // cols, so every pixel lands in exactly one cell."""
    cols, rows = grid
    sums = [[0, 0, 0, 0] for _ in range(cols * rows)]
    col_of = [x * cols // width for x in range(width)]
    for y in range(height):
        base = (y * rows // height) * cols
        line = rgb[y * width * 3:(y + 1) * width * 3]
        for x in range(width):
            cell = sums[base + col_of[x]]
            i = x * 3
            cell[0] += line[i]
            cell[1] += line[i + 1]
            cell[2] += line[i + 2]
            cell[3] += 1
    return [(s[0] / s[3], s[1] / s[3], s[2] / s[3]) for s in sums]


def grid_for(size):
    """GRID, or the picture's own size where it is smaller (a cell needs at least one pixel)."""
    return min(GRID[0], size[0]), min(GRID[1], size[1])


def luma(cell):
    return 0.2126 * cell[0] + 0.7152 * cell[1] + 0.0722 * cell[2]


def frame_flags(cells):
    """black, washed_out and flat, from the cells' luma mean and spread."""
    lumas = [luma(c) for c in cells]
    mean = sum(lumas) / len(lumas)
    sd = math.sqrt(sum((v - mean) ** 2 for v in lumas) / len(lumas))
    flags = []
    if mean < BLACK_LUMA:
        flags.append("black")
    elif mean > WASHED_LUMA:
        flags.append("washed_out")
    if sd < FLAT_SD:
        flags.append("flat")
    return {"luma_mean": round(mean, 3), "luma_sd": round(sd, 3), "flags": flags}


def compare_cells(a, b):
    """The mean absolute difference over cells and RGB channels (0..255), and the worst cell's own mean."""
    per_cell = [(abs(p[0] - q[0]) + abs(p[1] - q[1]) + abs(p[2] - q[2])) / 3 for p, q in zip(a, b)]
    return {"diff": round(sum(per_cell) / len(per_cell), 3), "worst_cell": round(max(per_cell), 3)}, per_cell


# Pairing and report ------------------------------------------------------------------------------------------------

def read_run(folder):
    path = Path(folder) / "run.json"
    return json.loads(path.read_text()) if path.exists() else None


def pair_runs(out, scenes):
    """Every pack either side ran, with each view's two pictures (None where a side has none) and run notes."""
    packs = {}
    for side in SIDES:
        base = Path(out) / side
        if not base.is_dir():
            continue
        for folder in sorted(p for p in base.iterdir() if p.is_dir() and (p / RUN_MARK).exists()):
            packs.setdefault(folder.name, {})[side] = read_run(folder) or {}
    pairs = {}
    for key, runs in sorted(packs.items()):
        rows = []
        for view in scenes["views"]:
            row = {"view": view["name"]}
            for side in SIDES:
                record = runs.get(side, {}).get("views", {}).get(view["name"], {})
                picture = Path(out) / side / key / f"{view['name']}.png"
                row[side] = {"picture": picture if picture.exists() else None,
                             "shots": phase_shots(Path(out) / side / key, view["name"]),
                             "problems": list(record.get("problems", [])) + ([] if side in runs else ["side not run"])
                             # A failed start check (a gamerule, the freeze) holds for every view of that run.
                             + [f"start: {p}" for p in runs.get(side, {}).get("start_problems", [])]}
            rows.append(row)
        mismatch = identity_problems(runs, scene_hash(scenes))
        for row in rows:
            row["problems"] = list(mismatch)
        pairs[key] = {"pack": next((r.get("pack") for r in runs.values() if r.get("pack")), key), "rows": rows,
                      "notes": [f"{side}: {n}" for side in SIDES for n in runs.get(side, {}).get("notes", [])
                                + runs.get(side, {}).get("start_problems", [])]}
    return pairs


def phase_shots(folder, view):
    """A view's pictures in phase order: `<view>.png`, then `<view>.p1.png`, `<view>.p2.png`... while they exist."""
    shots, index = [], 0
    while True:
        path = folder / (f"{view}.png" if index == 0 else f"{view}.p{index}.png")
        if not path.exists():
            return shots
        shots.append(path)
        index += 1


def _cells_of(task):
    path, size, grid = task
    try:
        return downscale(*read_png(path, size), grid)
    except (PictureError, OSError) as error:
        return str(error)


def all_cells(paths, size, grid):
    """Every picture's grid cells (or its error), decoded in parallel: a run has thousands of phase shots."""
    paths = list(dict.fromkeys(paths))
    if len(paths) < 8:
        return {p: _cells_of((p, size, grid)) for p in paths}
    with concurrent.futures.ProcessPoolExecutor(max_workers=min(8, os.cpu_count() or 1)) as pool:
        return dict(zip(paths, pool.map(_cells_of, [(p, size, grid) for p in paths], chunksize=4)))


def best_pair(sunrise, java):
    """The phase shots' best-matching pair (lowest difference; the first such in phase order) as (i, j, numbers,
    per_cell): the two games' texture animations froze on unrelated frames, so no index pairing lines them up, and
    the closest pair is the one in the same phase."""
    best = None
    for i, a in enumerate(sunrise):
        for j, b in enumerate(java):
            numbers, per_cell = compare_cells(a, b)
            if best is None or numbers["diff"] < best[2]["diff"]:
                best = (i, j, numbers, per_cell)
    return best


def own_spread(shots):
    """A side's own variation over a view's phase shots: the mean difference between every two of them (None with
    one shot). Texture animation and anything a pack animates on its own clock show up here."""
    if len(shots) < 2:
        return None
    diffs = [compare_cells(a, b)[0]["diff"] for i, a in enumerate(shots) for b in shots[i + 1:]]
    return round(sum(diffs) / len(diffs), 3)


def scene_hash(scenes):
    """The scene definition's identity: sha256 of the parsed scene (defaults filled in, keys sorted), so two runs
    are comparable only when they were taken with the same views, options and gamerules."""
    return hashlib.sha256(json.dumps(scenes, sort_keys=True).encode()).hexdigest()


def identity_problems(runs, current):
    """Why a pack's two runs can't be compared: taken in different generations of the shared world, or for another
    scene definition than each other or than the scene file compare reads."""
    if len(runs) < 2:
        return []
    problems = []
    worlds = {side: run.get("world_id") for side, run in runs.items()}
    if None in worlds.values() or len(set(worlds.values())) > 1:
        problems.append(f"the two sides were taken in different worlds ({worlds})")
    hashes = {run.get("scene_hash") for run in runs.values()}
    if hashes != {current}:
        problems.append("a side was taken for another scene definition than this scene file")
    packs = {side: run.get("pack_sha256") for side, run in runs.items()}
    if None in packs.values() or len(set(packs.values())) > 1:
        problems.append(f"the two sides staged different pack files ({packs})")
    return problems


def heat_rgb(per_cell, grid, scale):
    """The per-cell difference as a picture: black is equal, white is 64 or more apart, each cell scale^2 pixels."""
    cols, rows = grid
    out = bytearray()
    for y in range(rows * scale):
        for x in range(cols * scale):
            v = min(255, int(per_cell[(y // scale) * cols + x // scale] * 4))
            out += bytes((v, v, v))
    return cols * scale, rows * scale, bytes(out)


def side_by_side(left, right, half=2):
    """Two equal-size pictures next to each other, each box-filtered down by `half` (not at all when a side is
    smaller than `half` pixels: a picture keeps at least one pixel each way)."""
    (w, h, a), (_, _, b) = left, right
    half = half if min(w, h) >= half else 1
    sw, sh = w // half, h // half
    out = bytearray()
    for y in range(sh):
        for picture in (a, b):
            for x in range(sw):
                acc = [0, 0, 0]
                for dy in range(half):
                    i = ((y * half + dy) * w + x * half) * 3
                    for dx in range(half):
                        acc[0] += picture[i + dx * 3]
                        acc[1] += picture[i + dx * 3 + 1]
                        acc[2] += picture[i + dx * 3 + 2]
                out += bytes(v // (half * half) for v in acc)
    return sw * 2, sh, bytes(out)


def compare(out, scenes, pairs_for=()):
    """Writes report/ under out: report.json, report.md, index.html, heat/ and, for the views in pairs_for
    ("pack/view" or "all"), pairs/ side-by-side pictures. Each view's difference is its best-matching pair of
    phase shots (best_pair), shown with the pair's phases and the two sides' own spread (own_spread); a difference
    at or under the larger spread is within the pack's own variation. Returns the report."""
    out = Path(out)
    report_dir = out / "report"
    report_dir.mkdir(parents=True, exist_ok=True)
    grid = grid_for(scenes["size"])
    report = {"grid": list(grid), "size": scenes["size"], "packs": {}}
    pairs = pair_runs(out, scenes)
    cells = all_cells([p for pack in pairs.values() for row in pack["rows"] for side in SIDES
                       for p in row[side]["shots"]], scenes["size"], grid)
    for key, pack in pairs.items():
        rows = []
        for row in pack["rows"]:
            entry = {"view": row["view"], "problems": row["problems"]}
            good = {}
            for side in SIDES:
                info = {"problems": row[side]["problems"]}
                shots = row[side]["shots"]
                if not shots:
                    info["missing"] = True
                else:
                    info["picture"] = os.path.relpath(shots[0], report_dir)
                    info["shots"] = len(shots)
                    broken = [(path, cells[path]) for path in shots if isinstance(cells[path], str)]
                    info["problems"] += [error for _, error in broken]
                    good[side] = [(path, cells[path]) for path in shots if not isinstance(cells[path], str)]
                    info["own_spread"] = own_spread([c for _, c in good[side]])
                    if good[side]:
                        info.update(frame_flags(good[side][0][1]))
                entry[side] = info
            if all(good.get(s) for s in SIDES):
                i, j, numbers, per_cell = best_pair([c for _, c in good["sunrise"]], [c for _, c in good["java"]])
                chosen = {"sunrise": good["sunrise"][i][0], "java": good["java"][j][0]}
                entry.update(numbers)
                entry["phase"] = {"sunrise": shot_index(chosen["sunrise"]), "java": shot_index(chosen["java"])}
                spreads = [entry[s]["own_spread"] for s in SIDES if entry[s]["own_spread"] is not None]
                entry["noise"] = max(spreads) if spreads else None
                entry["within_own_variation"] = entry["noise"] is not None and numbers["diff"] <= entry["noise"]
                for side, (_, side_cells) in (("sunrise", good["sunrise"][i]), ("java", good["java"][j])):
                    entry[side]["picture"] = os.path.relpath(chosen[side], report_dir)
                    entry[side].update(frame_flags(side_cells))
                # Scored only when neither side nor the pair has a problem: a picture of the wrong scene is kept
                # and shown with its difference, but counts in no mean and no noise floor.
                entry["scored"] = not entry["problems"] and not any(entry[s]["problems"] for s in SIDES)
                heat = report_dir / "heat" / key / f"{row['view']}.png"
                write_png(heat, *heat_rgb(per_cell, grid, 8))
                entry["heat"] = os.path.relpath(heat, report_dir)
                if "all" in pairs_for or f"{key}/{row['view']}" in pairs_for:
                    pair = report_dir / "pairs" / key / f"{row['view']}.png"
                    write_png(pair, *side_by_side(read_png(chosen["sunrise"], scenes["size"]),
                                                  read_png(chosen["java"], scenes["size"])))
                    entry["pair"] = os.path.relpath(pair, report_dir)
            rows.append(entry)
        compared = [r["diff"] for r in rows if r.get("scored")]
        report["packs"][key] = {
            "pack": pack["pack"],
            "mean_diff": round(sum(compared) / len(compared), 3) if compared else None,
            "compared": len(compared), "views": len(rows), "notes": pack["notes"], "rows": rows,
        }
    floor = report["packs"].get("none", {}).get("mean_diff")
    report["noise_floor"] = floor
    (report_dir / "report.json").write_text(json.dumps(report, indent=2) + "\n")
    (report_dir / "report.md").write_text(markdown(report))
    (report_dir / "index.html").write_text(page(report))
    return report


def shot_index(path):
    """The phase index of a picture's file name: 0 for `<view>.png`, n for `<view>.pn.png`."""
    stem = Path(path).name[:-len(".png")]
    tail = stem.rpartition(".p")[2]
    return int(tail) if "." in stem and tail.isdigit() else 0


def _notes(entry):
    notes = list(entry.get("problems", []))
    if entry.get("within_own_variation"):
        notes.append("within the pack's own variation")
    if "diff" in entry and not entry.get("scored"):
        notes.append("not scored")
    for side in SIDES:
        info = entry[side]
        if info.get("missing"):
            notes.append(f"{side}: no picture")
        notes += [f"{side}: {f}" for f in info.get("flags", [])]
        notes += [f"{side}: {p}" for p in info.get("problems", [])]
    return notes


def ranked(report):
    """Packs by mean difference, largest first; packs with nothing compared last."""
    return sorted(report["packs"].items(), key=lambda kv: (kv[1]["mean_diff"] is None, -(kv[1]["mean_diff"] or 0)))


def markdown(report):
    lines = [f"# Shader scene comparison\n", f"Mean absolute RGB difference (0..255) on a {report['grid'][0]}x"
             f"{report['grid'][1]} grid, "
             "Sunrise against Java 26.3 (vanilla for `none`, Iris for a pack). Each view's difference is its "
             "best-matching pair of phase shots; its own variation is the larger of the two sides' mean difference "
             "between their own phase shots.\n"]
    if report["noise_floor"] is not None:
        lines.append(f"Noise floor (`none`, vanilla against vanilla): {report['noise_floor']}\n")
    lines += ["| Pack | Mean difference | Views compared |", "|---|---|---|"]
    for key, pack in ranked(report):
        lines.append(f"| {pack['pack']} | {pack['mean_diff']} | {pack['compared']} / {pack['views']} |")
    for key, pack in ranked(report):
        lines += ["", f"## {pack['pack']}", ""] + [f"- {n}" for n in pack["notes"]] + ([""] if pack["notes"] else [])
        lines += ["| View | Difference | Own variation | Phases (Sunrise, Java) | Worst cell | Notes |",
                  "|---|---|---|---|---|---|"]
        for entry in sorted(pack["rows"], key=lambda r: -r.get("diff", -1)):
            notes = "; ".join(_notes(entry)) or ""
            phase = entry.get("phase")
            phases = f"{phase['sunrise']}, {phase['java']}" if phase else ""
            lines.append(f"| {entry['view']} | {entry.get('diff', '')} | {entry.get('noise', '')} | {phases} | "
                         f"{entry.get('worst_cell', '')} | {notes} |")
    return "\n".join(lines) + "\n"


def page(report):
    e = html.escape
    parts = ["<!doctype html><meta charset=utf-8><title>Shader scenes</title>",
             "<style>body{font:14px sans-serif;background:#111;color:#ddd}img{width:480px;display:block}"
             "td{vertical-align:top;padding:4px}table{border-collapse:collapse}.bad{color:#f88}</style>",
             "<h1>Shader scenes: Sunrise | Java</h1>"]
    if report["noise_floor"] is not None:
        parts.append(f"<p>Noise floor (none): {report['noise_floor']}</p>")
    for key, pack in ranked(report):
        parts.append(f"<h2>{e(pack['pack'])}: {pack['mean_diff']}</h2>")
        parts += [f"<p class=bad>{e(n)}</p>" for n in pack["notes"]]
        parts.append("<table>")
        parts.append("<tr><th>View</th><th>Sunrise</th><th>Java</th><th>Difference</th></tr>")
        for entry in pack["rows"]:
            cells = [f"<td>{e(entry['view'])}<br>{entry.get('diff', '')}"
                     f"<div class=bad>{'<br>'.join(e(n) for n in _notes(entry))}</div></td>"]
            for side in SIDES:
                src = entry[side].get("picture")
                cells.append(f"<td><img src='{e(src)}' loading=lazy></td>" if src else "<td>missing</td>")
            heat = entry.get("heat")
            cells.append(f"<td><img src='{e(heat)}' style='width:256px'></td>" if heat else "<td></td>")
            parts.append("<tr>" + "".join(cells) + "</tr>")
        parts.append("</table>")
    return "\n".join(parts) + "\n"


def summary(report):
    """Prints the ranked packs; exits non-zero when no view was scored (pictures on both sides, no problem)."""
    total = 0
    for key, pack in ranked(report):
        total += pack["compared"]
        print(f"{pack['pack']}: mean difference {pack['mean_diff']} ({pack['compared']} of {pack['views']} views "
              "scored)")
    if report["noise_floor"] is not None:
        print(f"noise floor (none): {report['noise_floor']}")
    if not total:
        print("nothing compared: no view has a picture on both sides without a problem", file=sys.stderr)
        return 1
    return 0
