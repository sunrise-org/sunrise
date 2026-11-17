pub trait Notes {
    #[notes = r#"## Recipe book view

`PlayerRef::discovered_recipes(host)` lists every recipe the player's book knows, the whole set
`has_discovered_recipe` asks about one at a time. It reads a guest's book from outside their turn
as it reads the host's. Use `award_recipes` and `reset_recipes` for direct book changes, or
`award_used_recipe` when a mod result also needs vanilla's `recipe_crafted` criterion."#]
    fn player_stats_note0();
}
