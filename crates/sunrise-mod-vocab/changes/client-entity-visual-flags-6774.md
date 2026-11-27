# Client entity fire and glow

Client `EntityRef::is_on_fire` uses the local fire timer and, when present, the synced on-fire flag. The flag does not write to the timer. Mob lighting and custom entity models honor fire immunity for both sources of fire. The synced glow flag drives client rendering, including when it is clear, without changing the saved `Glowing` tag.
