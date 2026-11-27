//! tools/test_region_bytes_parity.py: each case's `r.0.0.mca` opened writable, every listed slot read (absent,
//! loaded with its NBT as written back, or an error), the loaded chunks written back in place with the selected
//! compression, then the region closed. Input lines: `<case> <region-file-compression> <slot>:<timestamp>...`,
//! the timestamp being the wall-clock second vanilla stamped the slot with, which a write here takes as input.
use std::{
    fmt::Write as _,
    fs,
    io::{self, Write},
    path::Path,
};
use sunrise_core::{
    nbt::{self, Limits},
    position::ChunkPos,
    version::VANILLA_VERSION,
};
use sunrise_world::{
    compression::Compression,
    region::{ChunkRecord, RegionFile, ReportedNbt},
};

pub fn main() -> io::Result<()> {
    let mut args = crate::args().skip(1);
    let root = args.next().unwrap();
    let inputs = fs::read_to_string(args.next().unwrap())?;
    let mut out = io::BufWriter::new(fs::File::create(args.next().unwrap())?);
    writeln!(out, "version {VANILLA_VERSION}")?;
    for line in inputs.lines() {
        let mut words = line.split(' ');
        let case = words.next().unwrap();
        writeln!(out, "== {case}")?;
        // As `RegionFileVersion.configure`: a name it does not know keeps the selection.
        Compression::configure(words.next().unwrap());
        let mut region =
            RegionFile::open_writable(Path::new(&root).join(case).join("r.0.0.mca"), false)?;
        let mut loaded = Vec::new();
        for word in words {
            let (slot, timestamp) = word.split_once(':').unwrap();
            let slot: i32 = slot.parse().unwrap();
            let pos = ChunkPos::new(slot % 32, slot / 32);
            let read = region
                .read(pos)
                .and_then(|record| record.map(|r| r.nbt(Limits::saved())).transpose());
            match read {
                Ok(None) => writeln!(out, "slot {slot} absent")?,
                Ok(Some(root)) => {
                    let bytes = nbt::write_named(&root, Limits::saved())
                        .map_err(|e| io::Error::other(e.to_string()))?;
                    let hex = bytes.iter().fold(String::new(), |mut s, b| {
                        let _ = write!(s, "{b:02x}");
                        s
                    });
                    writeln!(out, "slot {slot} loaded {hex}")?;
                    loaded.push((pos, root, timestamp.parse::<u32>().unwrap()));
                }
                Err(e) => {
                    eprintln!("{case} slot {slot}: {e}");
                    let class = match e.get_ref() {
                        Some(e) if e.is::<ReportedNbt>() => {
                            "net.minecraft.nbt.ReportedNbtException"
                        }
                        _ => "io",
                    };
                    writeln!(out, "slot {slot} error {class}")?;
                }
            }
        }
        for (pos, root, timestamp) in loaded {
            let record =
                ChunkRecord::from_nbt(&root, Compression::selected(), timestamp, Limits::saved())?;
            region.write(pos, &record)?;
        }
        region.close()?;
    }
    out.flush()
}
