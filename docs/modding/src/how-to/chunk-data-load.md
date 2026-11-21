# Loading chunk attachments

`chunk_data_load` receives `&dyn ChunkDiskHost` and a `ChunkDataLoadEvent` after
a full or proto disk record has decoded, before its attachment store is
published. A missing record, fresh generation and proto-to-full promotion do
not fire this hook. The callback uses `event.data.get(&KEY)` and
`event.data.set(&KEY, value)` to read and edit the typed store of this load.
For example, reading disk value 8 and setting 9 publishes 9 on success.

The worker host supplies only the job identity and immutable resource reads.
It has no live backend and is not a `ServerHost`.
Every typed value needs a persistence codec and must implement `Wire`.
Unknown bounded attachment entries are preserved opaquely.

Codec errors and unwinding panics refuse the entire
store: none of its edits are kept. The chunk still loads, with the attachments
exactly as they were stored, so a mod bug leaves no gap in the world. No
`chunk_data_save` listener runs for that chunk for the rest of the session, so
half-initialised state never replaces the stored record, and the chunk's later
saves write its stored attachments back unchanged. The failure is logged once
per chunk, naming the chunk and the mod. Fix the mod and rejoin the world to run
its listeners on that chunk again.

The load runs on the thread that decodes the record, never with access to live
world state. The budgets are 65536 bytes per value, depth 64 and 1048576 bytes
per chunk.
