"""tools/shader_scenes.py's comparison: base against head, pixel for pixel at full resolution.

A view is compared only when both sides recorded it in their run.json without a problem, the picture the record
names is there with the sha256 the record gave it when the capture finished, it decodes as a PNG of the scene's size,
and both runs were taken in the same world, for the same scene definition, of the same staged pack. Anything else
fails the view as invalid: a missing record, a missing or changed file, a damaged picture, a mismatched pair. A
valid pair whose pixels differ in any channel is a difference. Pictures in a run folder that no record names are
listed and never compared. Standard library only: the PNG reader takes the 8-bit RGB and RGBA pictures Sunrise
writes."""
import concurrent.futures
import hashlib
import html
import json
import os
import shutil
import struct
import sys
import zlib
from pathlib import Path

SIDES = ("base", "head")
RUN_MARK = ".shader-scenes-run"
REPORT_MARK = ".shader-scenes-report"
# The tool's exit status: every view valid and equal; the comparison completed and some valid view differs; some
# view invalid or missing; the run could not be set up or the command line is wrong (nothing was compared).
SAME, DIFFERENT, INVALID, SETUP_FAILED = 0, 1, 2, 3


# PNG -----------------------------------------------------------------------------------------------------------

class PictureError(ValueError):
    """A picture this tool cannot compare; the message says why."""


def _png_rows(path, size):
    """The checked structure of an 8-bit RGB or RGBA, non-interlaced PNG of exactly `size`: (channels, the
    decompressed filtered rows). Every way a file can be damaged is a PictureError naming it: a chunk running past
    the file, a bad CRC, a short IHDR, a missing IEND, a zlib stream that is invalid, cut short or longer than the
    rows, a row filter PNG doesn't define. Any other size is refused before its pixels are decompressed, which also
    bounds the memory a picture takes."""
    data = Path(path).read_bytes()
    if data[:8] != b"\x89PNG\r\n\x1a\n":
        raise PictureError(f"{path}: not a PNG")
    pos, header, chunks, ended = 8, None, [], False
    while pos < len(data):
        first = pos == 8
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
        if first and kind != b"IHDR":
            raise PictureError(f"{path}: the first chunk is {kind!r}, not IHDR")
        if kind == b"IHDR":
            if header is not None:
                raise PictureError(f"{path}: a second IHDR")
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
    expected = (width * channels + 1) * height
    stream = zlib.decompressobj()
    try:
        raw = stream.decompress(b"".join(chunks), expected + 1)
    except zlib.error as error:
        raise PictureError(f"{path}: invalid image data ({error})") from None
    if not stream.eof:
        raise PictureError(f"{path}: the image data is cut short or longer than its rows")
    if len(raw) != expected:
        raise PictureError(f"{path}: {len(raw)} bytes of rows, {expected} expected")
    stride = width * channels + 1
    for y in range(height):
        if raw[y * stride] > 4:
            raise PictureError(f"{path}: unknown filter type {raw[y * stride]}")
    return channels, raw


def check_png(path, size):
    """Raises PictureError unless `path` is a whole PNG of `size` read_png can decode (without unfiltering it)."""
    _png_rows(path, size)


def read_png(path, size):
    """The pixels of a PNG of exactly `size`: (width, height, channels, bytes), rows top to bottom, RGB or RGBA as
    the file stores them."""
    channels, raw = _png_rows(path, size)
    width, height = size
    stride = width * channels
    rows = bytearray()
    prior = bytearray(stride)
    for y in range(height):
        start = y * (stride + 1)
        line = bytearray(raw[start + 1:start + 1 + stride])
        _unfilter(raw[start], line, prior, channels)
        rows += line
        prior = line
    return width, height, channels, bytes(rows)


def _unfilter(kind, line, prior, bpp):
    n = len(line)
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


def write_png(path, width, height, rgb):
    """An 8-bit RGB PNG of `rgb` (rows top to bottom, no filter)."""
    def chunk(kind, body):
        return struct.pack(">I", len(body)) + kind + body + struct.pack(">I", zlib.crc32(kind + body))
    stride = width * 3
    raw = b"".join(b"\x00" + rgb[y * stride:(y + 1) * stride] for y in range(height))
    Path(path).parent.mkdir(parents=True, exist_ok=True)
    Path(path).write_bytes(b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0))
                           + chunk(b"IDAT", zlib.compress(raw, 6)) + chunk(b"IEND", b""))


def sha256_file(path):
    digest = hashlib.sha256()
    with open(path, "rb") as source:
        for block in iter(lambda: source.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


# Pixels ----------------------------------------------------------------------------------------------------------

def pixel_diff(a, b):
    """Two decoded pictures of one size, compared exactly: None when every pixel is equal, else {"pixels": how many
    pixels differ in any channel, "max_delta": the largest channel difference, "box": [x0, y0, x1, y1] (inclusive)
    around them}. An RGB and an RGBA picture compare by RGB, plus alpha 255 for the RGB one."""
    (width, height, ca, pa), (_, _, cb, pb) = a, b
    if ca == cb and pa == pb:
        return None
    count, worst, box = 0, 0, [width, height, -1, -1]
    for y in range(height):
        for x in range(width):
            pxa, pxb = _rgba(pa, ca, y * width + x), _rgba(pb, cb, y * width + x)
            if pxa != pxb:
                count += 1
                worst = max(worst, max(abs(p - q) for p, q in zip(pxa, pxb)))
                box = [min(box[0], x), min(box[1], y), max(box[2], x), max(box[3], y)]
    return None if count == 0 else {"pixels": count, "max_delta": worst, "box": box}


def diff_rgb(a, b):
    """The head picture at a quarter of its brightness, with every pixel that differs from base in full red."""
    (width, height, ca, pa), (_, _, cb, pb) = a, b
    out = bytearray(width * height * 3)
    for k in range(width * height):
        head = _rgba(pb, cb, k)
        out[k * 3:k * 3 + 3] = b"\xff\x00\x00" if _rgba(pa, ca, k) != head else bytes(v // 4 for v in head[:3])
    return width, height, bytes(out)


def _rgba(pixels, channels, k):
    """Pixel k as (r, g, b, a): alpha 255 for an RGB picture."""
    i = k * channels
    return tuple(pixels[i:i + 4]) if channels == 4 else (pixels[i], pixels[i + 1], pixels[i + 2], 255)


# Records ---------------------------------------------------------------------------------------------------------

def record_shape_problem(record):
    """Why a run record is not one run_capture writes (None when it is): an object whose start_problems and notes are
    lists of strings, whose pack is a name and whose views map each name to an object (the identities and each
    view's fields are checked where they are read: identity_problems, side_view)."""
    if not isinstance(record, dict):
        return "not an object"
    for key in ("start_problems", "notes"):
        value = record.get(key)
        if not isinstance(value, list) or not all(isinstance(v, str) for v in value):
            return f"{key} is not a list of strings"
    if not isinstance(record.get("pack"), str) or not record["pack"]:
        return "pack is not a name"
    views = record.get("views")
    if not isinstance(views, dict) or not all(isinstance(v, dict) for v in views.values()):
        return "views is not an object of view records"
    return None


def read_run(folder):
    """A run's record, or None when it has none; an unreadable or malformed record is a dict with "unreadable"."""
    path = Path(folder) / "run.json"
    if not path.exists():
        return None
    try:
        record = json.loads(path.read_text())
    except (OSError, ValueError) as error:
        return {"unreadable": f"{path}: {error}"}
    try:
        # A lone surrogate (JSON's "\\ud800") decodes, but no report can be written with it.
        json.dumps(record, ensure_ascii=False).encode("utf-8")
    except UnicodeEncodeError:
        return {"unreadable": f"{path}: a string in it is not text (a lone surrogate)"}
    problem = record_shape_problem(record)
    return {"unreadable": f"{path}: {problem}"} if problem else record


def run_folders(out, keys=None):
    """{pack key: {side: folder}} for the run folders this tool made under out/base and out/head: those of `keys`
    only when given, else every one there is."""
    runs = {}
    for side in SIDES:
        base = Path(out) / side
        if keys is not None:
            folders = [base / key for key in keys]
        else:
            folders = sorted(base.iterdir()) if base.is_dir() else []
        for folder in folders:
            if folder.is_dir() and (folder / RUN_MARK).exists():
                runs.setdefault(folder.name, {})[side] = folder
    return runs


def scene_hash(scenes):
    """The scene definition's identity: sha256 of the parsed scene (defaults filled in, keys sorted), so two runs
    are compared only when they were taken with the same views, options, gamerules and capture clock."""
    return hashlib.sha256(json.dumps(scenes, sort_keys=True).encode()).hexdigest()


# What a run records of the inputs that shape its pictures besides the build, and what tells them apart: the
# world and its save, the scene definition, the staged pack, the prepared resources, and the renderer the game
# picked (its GPU, vendor, driver, device type and graphics backend, as `sunrise-ctl status` reports them).
IDENTITY = (("world_id", "worlds"), ("save_sha256", "saves"), ("scene_hash", "scene definitions"),
            ("pack_sha256", "pack files"), ("resources_sha256", "prepared resources"), ("renderer", "renderers"))
RENDERER_KEYS = ("gpu", "vendor", "driver", "device_type", "backend")


def _identity_value(key, value):
    """A recorded identity as it compares (None when it is missing or not of its type): a string, or for the
    renderer an object of RENDERER_KEYS strings."""
    if key == "renderer":
        if not isinstance(value, dict) or not all(isinstance(value.get(k), str) for k in RENDERER_KEYS):
            return None
        return json.dumps({k: value[k] for k in RENDERER_KEYS}, sort_keys=True)
    return value if isinstance(value, str) and value else None


def identity_problems(records):
    """Why a pack's base and head runs can't be compared: taken in different worlds or saves, for different scene
    definitions, of different staged pack files, with different prepared resources or on different renderers, or
    with a record missing an identity (or giving one of the wrong type)."""
    problems = []
    for key, what in IDENTITY:
        values = {side: _identity_value(key, record.get(key)) for side, record in records.items()}
        if None in values.values():
            shown = {side: record.get(key) for side, record in records.items()}
            problems.append(f"a run records no valid {key} ({shown})")
        elif len(set(values.values())) > 1:
            problems.append(f"the two sides were taken with different {what} ({values})")
    return problems


def describe_inputs(record):
    """One line of what a run's pictures were drawn with, for the report."""
    renderer = record.get("renderer") if isinstance(record.get("renderer"), dict) else {}
    resources = record.get("resources_sha256")
    return (f"{renderer.get('gpu')} ({renderer.get('vendor')}, {renderer.get('device_type')}), driver "
            f"{renderer.get('driver')}, backend {renderer.get('backend')}; resources "
            f"{resources[:12] if isinstance(resources, str) else None}")


def unrecorded_pictures(folder, record):
    """PNG files in a run folder that no view record of it names (a capture that stopped before recording, or a
    file put there by hand): listed, never compared."""
    named = {v.get("picture") for v in (record or {}).get("views", {}).values()
             if isinstance(v, dict) and isinstance(v.get("picture"), str)}
    return sorted(p.name for p in Path(folder).glob("*.png") if p.name not in named)


def side_view(folder, record, view, size):
    """One side's picture of a view, checked: (path, problems). The path is None unless the record carries the
    view's checks with no problem, names the view's own picture (`<view>.png`), the file is there, its sha256 is the
    one recorded when the capture finished and it is a whole PNG of the scene's size."""
    if record is None:
        return None, ["the run has no run.json"]
    if "unreadable" in record:
        return None, [record["unreadable"]]
    problems = [f"start: {p}" for p in record["start_problems"]]
    entry = record["views"].get(view)
    if entry is None:
        return None, problems + ["no record of this view (the capture never finished)"]
    checks = entry.get("problems")
    if not isinstance(checks, list) or not all(isinstance(p, str) for p in checks):
        return None, problems + ["the record carries no checks of this view"]
    problems += checks
    name = entry.get("picture")
    if name != f"{view}.png" or not isinstance(entry.get("sha256"), str):
        return None, problems + [f"the record names no picture {view}.png with its hash (picture {name!r})"]
    path = Path(folder) / name
    if not path.is_file():
        return None, problems + [f"{name} is missing"]
    if sha256_file(path) != entry["sha256"]:
        problems.append(f"{name} is not the file the capture recorded (its sha256 changed)")
    else:
        try:
            check_png(path, size)
        except (PictureError, OSError) as error:
            problems.append(str(error))
    return (None if problems else path), problems


# Report ----------------------------------------------------------------------------------------------------------

def _compare_pair(task):
    base, head, size, diff_path = task
    try:
        if sha256_file(base) == sha256_file(head):
            return None
        a, b = read_png(base, size), read_png(head, size)
    except (PictureError, OSError) as error:
        return {"error": str(error)}
    result = pixel_diff(a, b)
    if result is not None:
        write_png(diff_path, *diff_rgb(a, b))
    return result


def fresh_report_dir(out):
    """out/report, emptied when an earlier compare made it (it carries REPORT_MARK); never another folder."""
    report_dir = Path(out) / "report"
    if report_dir.exists():
        if not (report_dir / REPORT_MARK).exists():
            raise SystemExit(f"{report_dir} was not made by this tool; not replacing it")
        shutil.rmtree(report_dir)
    report_dir.mkdir(parents=True)
    (report_dir / REPORT_MARK).write_text("")
    return report_dir


def compare(out, scenes, packs=None, jobs=None):
    """Compares every view of every pack (`packs`: the pack keys to compare, and only those; default every pack
    either side ran)
    and writes report/ under out: report.json, report.md, index.html and diff/<pack>/<view>.png for each view that
    differs. Returns the report; its "status" is SAME, DIFFERENT or INVALID."""
    out = Path(out)
    report_dir = fresh_report_dir(out)
    size = scenes["size"]
    # An explicit list (`run`'s packs) is the whole scope: only its folders are looked for, so an earlier run's
    # folder of another pack is not this comparison's. Without one (`compare` on its own) every run folder this tool
    # made is compared.
    runs = run_folders(out, None if packs is None else sorted(set(packs)))
    keys = sorted(set(runs) | set(packs or []))
    current = scene_hash(scenes)
    report = {"size": size, "scene_hash": current, "packs": {}}
    tasks, rows_of = [], {}
    for key in keys:
        folders = runs.get(key, {})
        records = {side: read_run(folders[side]) if side in folders else None for side in SIDES}
        notes = [f"{side}: no run" for side in SIDES if side not in folders]
        pair = {}
        for side in SIDES:
            if side in folders and records[side] is not None and "unreadable" not in records[side]:
                pair[side] = records[side]
                notes += [f"{side}: {n}" for n in records[side].get("notes", [])]
                notes += [f"{side}: picture {p} has no record; not compared"
                          for p in unrecorded_pictures(folders[side], records[side])]
        mismatch = identity_problems(pair) if len(pair) == 2 else []
        mismatch += [f"{side} was taken for another scene definition than this scene file"
                     for side, record in pair.items() if record.get("scene_hash") not in (None, current)]
        rows = []
        for view in scenes["views"]:
            row = {"view": view["name"], "problems": list(mismatch)}
            paths = {}
            for side in SIDES:
                if side not in folders:
                    path, problems = None, ["no run of this pack"]
                else:
                    path, problems = side_view(folders[side], records[side], view["name"], size)
                row[side] = {"problems": problems}
                if path is not None:
                    paths[side] = path
                    row[side]["picture"] = os.path.relpath(path, report_dir)
            row["valid"] = len(paths) == 2 and not row["problems"]
            if row["valid"]:
                diff = report_dir / "diff" / key / f"{view['name']}.png"
                tasks.append((paths["base"], paths["head"], size, diff))
                rows_of[len(tasks) - 1] = (row, diff)
            rows.append(row)
        notes += [f"{side} drawn on: {describe_inputs(record)}" for side, record in pair.items()]
        report["packs"][key] = {"pack": next((r["pack"] for r in pair.values() if isinstance(r.get("pack"), str)
                                              and r["pack"]), key),
                                "world_id": {side: r.get("world_id") for side, r in pair.items()},
                                "notes": notes, "rows": rows}
    workers = max(1, min(jobs or os.cpu_count() or 1, 8))
    if len(tasks) > 1 and workers > 1:
        with concurrent.futures.ProcessPoolExecutor(max_workers=workers) as pool:
            results = list(pool.map(_compare_pair, tasks))
    else:
        results = [_compare_pair(t) for t in tasks]
    for index, result in enumerate(results):
        row, diff = rows_of[index]
        if isinstance(result, dict) and "error" in result:
            row["valid"] = False
            row["problems"].append(result["error"])
        elif result is None:
            row["equal"] = True
        else:
            row["equal"] = False
            row.update(result)
            row["diff"] = os.path.relpath(diff, report_dir)
    for pack in report["packs"].values():
        rows = pack["rows"]
        pack["views"] = len(rows)
        pack["valid"] = sum(1 for r in rows if r["valid"])
        pack["equal"] = sum(1 for r in rows if r.get("equal"))
        pack["different"] = sum(1 for r in rows if r.get("equal") is False)
    everything = [r for p in report["packs"].values() for r in p["rows"]]
    if not everything or any(not r["valid"] for r in everything):
        report["status"] = INVALID
    elif any(r.get("equal") is False for r in everything):
        report["status"] = DIFFERENT
    else:
        report["status"] = SAME
    (report_dir / "report.json").write_text(json.dumps(report, indent=2) + "\n")
    (report_dir / "report.md").write_text(markdown(report))
    (report_dir / "index.html").write_text(page(report))
    return report


def _row_notes(row):
    notes = list(row["problems"])
    for side in SIDES:
        notes += [f"{side}: {p}" for p in row[side]["problems"]]
    return notes


def _outcome(row):
    if not row["valid"]:
        return "invalid"
    if row.get("equal"):
        return "equal"
    return f"{row['pixels']} pixels differ (max {row['max_delta']}, box {row['box']})"


def markdown(report):
    lines = ["# Shader scenes: base against head\n",
             f"Every view compared pixel for pixel at {report['size'][0]}x{report['size'][1]}. A view is invalid when "
             "a side has no recorded, checked picture of it, or the two sides were taken in different worlds, scene "
             "definitions or pack files.\n",
             "| Pack | Views | Valid | Equal | Different |", "|---|---|---|---|---|"]
    for key, pack in report["packs"].items():
        lines.append(f"| {pack['pack']} | {pack['views']} | {pack['valid']} | {pack['equal']} | {pack['different']} |")
    for key, pack in report["packs"].items():
        lines += ["", f"## {pack['pack']}", ""] + [f"- {n}" for n in pack["notes"]] + ([""] if pack["notes"] else [])
        lines += ["| View | Outcome | Notes |", "|---|---|---|"]
        for row in pack["rows"]:
            lines.append(f"| {row['view']} | {_outcome(row)} | {'; '.join(_row_notes(row))} |")
    return "\n".join(lines) + "\n"


def page(report):
    e = html.escape
    parts = ["<!doctype html><meta charset=utf-8><title>Shader scenes</title>",
             "<style>body{font:14px sans-serif;background:#111;color:#ddd}img{width:480px;display:block}"
             "td{vertical-align:top;padding:4px}table{border-collapse:collapse}.bad{color:#f88}</style>",
             "<h1>Shader scenes: base | head | difference</h1>"]
    for key, pack in report["packs"].items():
        parts.append(f"<h2>{e(pack['pack'])}: {pack['equal']} of {pack['views']} views equal</h2>")
        parts += [f"<p class=bad>{e(n)}</p>" for n in pack["notes"]]
        parts.append("<table><tr><th>View</th><th>Base</th><th>Head</th><th>Difference</th></tr>")
        for row in pack["rows"]:
            notes = "<br>".join(e(n) for n in _row_notes(row))
            cells = [f"<td>{e(row['view'])}<br>{e(_outcome(row))}<div class=bad>{notes}</div></td>"]
            for side in SIDES:
                src = row[side].get("picture")
                cells.append(f"<td><img src='{e(src)}' loading=lazy></td>" if src else "<td>missing</td>")
            diff = row.get("diff")
            cells.append(f"<td><img src='{e(diff)}' loading=lazy></td>" if diff else "<td></td>")
            parts.append("<tr>" + "".join(cells) + "</tr>")
        parts.append("</table>")
    return "\n".join(parts) + "\n"


def summary(report):
    """Prints each pack's counts and every view that is not equal; returns the report's status."""
    for key, pack in report["packs"].items():
        print(f"{pack['pack']}: {pack['equal']} of {pack['views']} views equal, {pack['different']} different, "
              f"{pack['views'] - pack['valid']} invalid")
        for row in pack["rows"]:
            if not row.get("equal"):
                notes = "; ".join(_row_notes(row))
                print(f"  {row['view']}: {_outcome(row)}{': ' + notes if notes else ''}")
    if not report["packs"]:
        print("nothing to compare: no run under base/ or head/", file=sys.stderr)
    return report["status"]
