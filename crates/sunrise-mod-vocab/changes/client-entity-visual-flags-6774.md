# Client entity fire and glow

Client `EntityRef::is_on_fire` uses the local fire timer and, when present, the synced on-fire flag. The flag does not write to the timer. Synced glow still drives rendering without changing the saved `Glowing` tag.
