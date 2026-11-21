# Saving chunk attachments

This hook saves chunk attachments. To serialize a mod block entity's live state into its own
payload, use [`block_entity_save_additional`](../concepts/parts/block-entity-types.md#saved-data)
on the server thread, before the disk worker sees the capture. If block-entity serializer edits
cannot be written, the worker retries with stored block-entity payloads and keeps the result of
`chunk_data_save`.

`chunk_data_save` receives `&dyn ChunkDiskHost` and a `ChunkDataSaveEvent`.
The event's `data` is a typed, job-local attachment store. Use its `get`, `set`
and `remove` methods with an `AttachmentType<T>` that has a persistence codec.
`T` does not need `Wire`. An ignored codec error still refuses the whole job.

The callback runs on the persistence worker after capture and before encoding.
For example, capturing 7, writing 8 in the callback and later writing 9 in the
live chunk produces disk value 8 without changing live value 9. The host's
`job()` borrows a `&ChunkDiskContext` identifying the world epoch, dimension,
coordinates, full/proto kind and phase. It grants no live chunk lookup,
scheduling, packets or world writes.
`registry(name)` lends `Option<&[String]>` in registry ID order from the frozen generation
retained by the worker. `None` means that generation has no such registry. Use the slice within
the callback, or copy it if it must outlive the callback. Neither metadata getter reads files;
`ConfigHost::setting` still returns an owned `Option<String>` from the same snapshot.

A captured record caches its callback result. Disk encoding and pending reads
reuse it; they cannot invoke the callback twice. A refusal keeps the original
capture and prevents the writer from replacing the prior region record. A chunk
whose `chunk_data_load` listener failed is saved without any save listener.

Every captured record runs the callback exactly once, for full saves (explicit,
forced, late, unload and closing) and proto saves alike. Saves are refused, with
no callback, while an action savepoint is open, and chunk unloading waits for it to close. If the disk write fails, the
old record stays on disk, the snapshot stays readable for recovery and the
wait or flush reports the failure; nothing is marked saved early. A
chunk whose save was refused or failed is marked modified again, so the game
tries it again at the next save.

## Disk NBT budgets

The job checks the uncompressed disk NBT: at most 65,536 bytes per named
`value` tag, 64 container levels in the value, and 1,048,576 bytes for the
whole `sunrise:attachments` entry. These budgets include tag IDs, names,
compound terminators, mixed-list wrappers, empty namespaces and opaque
unknown data as applicable. Over-budget data or data that would need
truncation (such as an overlong modified UTF-8 string) refuses the job.
